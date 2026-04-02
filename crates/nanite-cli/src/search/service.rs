use crate::context::ContextState;
use crate::util::{canonicalize_utf8, utf8_from_path_buf};
use anyhow::{Context, Result, anyhow, bail};
use arborium::{Config as HighlightConfig, Highlighter, HtmlFormat, detect_language};
use camino::{Utf8Path, Utf8PathBuf};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{AllQuery, BooleanQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{
    FAST, Field, IndexRecordOption, NumericOptions, STORED, STRING, Schema, TantivyDocument,
    TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::{
    LowerCaser, NgramTokenizer, RemoveLongFilter, SimpleTokenizer, TextAnalyzer,
};
use tantivy::{Index, IndexReader, IndexWriter, Order, ReloadPolicy, Term, doc};

const INDEX_MANIFEST_VERSION: u32 = 2;
const INDEX_WRITER_BYTES: usize = 50_000_000;
const MAX_INDEXED_FILE_BYTES: u64 = 512 * 1024;
const MAX_HIGHLIGHT_FILE_BYTES: u64 = 256 * 1024;
const MAX_TOKEN_BYTES: usize = 120;

#[derive(Debug, Clone, Copy)]
pub enum RefreshMode {
    RefreshIfNeeded,
    ForceRebuild,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexPhase {
    Scanning,
    Building,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexProgress {
    pub phase: IndexPhase,
    pub message: String,
    pub files_scanned: usize,
    pub files_total: Option<usize>,
    pub files_indexed: usize,
    pub lines_indexed: usize,
    pub skipped_binary: usize,
    pub skipped_large: usize,
}

pub struct SearchOpen {
    pub engine: SearchEngine,
    pub report: IndexBuildReport,
}

pub struct SearchEngine {
    index: Index,
    reader: IndexReader,
    schema: SearchSchema,
    repos: Vec<RepositoryEntry>,
    highlighter: Highlighter,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchRequest {
    pub query: String,
    pub repo_filters: Vec<String>,
    pub path_filters: Vec<String>,
    pub file_filters: Vec<String>,
    pub lang_filters: Vec<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub query: SearchRequest,
    pub total_hits: usize,
    pub truncated: bool,
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub repo: String,
    pub repo_label: String,
    pub path: String,
    pub file: String,
    pub language: Option<String>,
    pub line_number: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoSummary {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileView {
    pub repo: String,
    pub repo_label: String,
    pub path: String,
    pub language: Option<String>,
    pub line_count: usize,
    pub highlighted_html: Option<String>,
    pub plain_text: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexBuildReport {
    pub rebuilt: bool,
    pub partial: bool,
    pub repos_refreshed: usize,
    pub files_scanned: usize,
    pub files_indexed: usize,
    pub lines_indexed: usize,
    pub skipped_binary: usize,
    pub skipped_large: usize,
    pub index_path: String,
}

#[derive(Debug, Clone)]
struct RepositoryEntry {
    id: String,
    label: String,
    root: Utf8PathBuf,
    canonical_root: Utf8PathBuf,
}

#[derive(Debug, Clone)]
struct WorkspaceSnapshot {
    files: Vec<CollectedFile>,
    skipped_binary: usize,
    skipped_large: usize,
}

#[derive(Debug, Clone)]
struct CollectedFile {
    manifest: ManifestEntry,
    repo_label: String,
    relative_path: Utf8PathBuf,
    file_name: String,
    text: Option<String>,
}

struct ResolvedFile<'a> {
    repo: &'a RepositoryEntry,
    canonical_path: Utf8PathBuf,
    requested_path: Utf8PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct IndexManifest {
    version: u32,
    files: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ManifestEntry {
    key: String,
    repo_id: String,
    relative_path: String,
    size: u64,
    modified_secs: u64,
    modified_nanos: u32,
    language: Option<String>,
    status: IndexedFileStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum IndexedFileStatus {
    Text,
    Binary,
    TooLarge,
}

#[derive(Debug, Clone)]
struct SearchSchema {
    repo: Field,
    repo_label: Field,
    path_raw: Field,
    file_raw: Field,
    language_raw: Field,
    line_number: Field,
    sort_key: Field,
    text_raw: Field,
    content: Field,
    content_ngram: Field,
    path: Field,
    path_ngram: Field,
    file: Field,
    file_ngram: Field,
    language: Field,
}

struct SearchIndexPaths {
    index_dir: Utf8PathBuf,
    manifest_path: Utf8PathBuf,
}

impl SearchEngine {
    pub fn open(context: &ContextState, mode: RefreshMode) -> Result<SearchOpen> {
        let report = refresh_index(context, mode, |_| {})?;
        let engine = Self::open_existing(context)?
            .ok_or_else(|| anyhow!("search index was not available after refresh"))?;

        Ok(SearchOpen { engine, report })
    }

    pub fn open_existing(context: &ContextState) -> Result<Option<Self>> {
        let paths = index_paths(context);
        if !paths.index_dir.join("meta.json").exists() {
            return Ok(None);
        }

        let repos = discover_repositories(context.workspace_paths.repos_root())?;
        let index = Index::open_in_dir(paths.index_dir.as_std_path())
            .with_context(|| format!("failed to open search index in {}", paths.index_dir))?;
        register_tokenizers(&index);
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        let Ok(schema) = SearchSchema::from_schema(&index.schema()) else {
            return Ok(None);
        };
        let highlighter = Highlighter::with_config(HighlightConfig {
            max_injection_depth: 2,
            html_format: HtmlFormat::ClassNamesWithPrefix("arb".to_owned()),
        });

        Ok(Some(Self {
            index,
            reader,
            schema,
            repos,
            highlighter,
        }))
    }

    pub fn search(&self, request: &SearchRequest) -> Result<SearchResponse> {
        if request.limit == 0
            || (request.query.trim().is_empty()
                && request.repo_filters.is_empty()
                && request.path_filters.is_empty()
                && request.file_filters.is_empty()
                && request.lang_filters.is_empty())
        {
            return Ok(SearchResponse {
                query: request.clone(),
                total_hits: 0,
                truncated: false,
                hits: Vec::new(),
            });
        }

        let query = self.build_query(request)?;
        let searcher = self.reader.searcher();
        let collector =
            TopDocs::with_limit(request.limit).order_by_string_fast_field("sort_key", Order::Asc);
        let (top_docs, total_hits) = searcher.search(&query, &(collector, Count))?;
        let hits = top_docs
            .into_iter()
            .map(|(_, address)| self.hit_from_doc(&searcher, address))
            .collect::<Result<Vec<_>>>()?;

        Ok(SearchResponse {
            query: request.clone(),
            total_hits,
            truncated: total_hits > hits.len(),
            hits,
        })
    }

    pub fn repo_summaries(&self) -> Vec<RepoSummary> {
        self.repos
            .iter()
            .map(|repo| RepoSummary {
                id: repo.id.clone(),
                label: repo.label.clone(),
            })
            .collect()
    }

    pub fn open_in_zed(
        &self,
        zed_binary: &str,
        repo_id: &str,
        relative_path: &str,
        line: u64,
        column: u64,
    ) -> Result<()> {
        let resolved = self.resolve_file(repo_id, relative_path)?;
        let line = line.max(1);
        let column = column.max(1);
        let target = format!("{}:{line}:{column}", resolved.canonical_path);

        Command::new(zed_binary)
            .arg(resolved.repo.root.as_std_path())
            .arg(target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn {zed_binary}"))?;

        Ok(())
    }

    pub fn file_view(&mut self, repo_id: &str, relative_path: &str) -> Result<FileView> {
        let resolved = self.resolve_file(repo_id, relative_path)?;
        let repo_id = resolved.repo.id.clone();
        let repo_label = resolved.repo.label.clone();
        let requested_path = resolved.requested_path.to_string();
        let metadata = fs::metadata(resolved.canonical_path.as_std_path())
            .with_context(|| format!("failed to stat {}", resolved.canonical_path))?;
        if !metadata.is_file() {
            bail!("{} is not a file", resolved.canonical_path);
        }

        let bytes = fs::read(resolved.canonical_path.as_std_path())
            .with_context(|| format!("failed to read {}", resolved.canonical_path))?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let language = detect_language(resolved.requested_path.as_str()).map(str::to_owned);
        let line_count = text.lines().count().max(1);

        if metadata.len() > MAX_HIGHLIGHT_FILE_BYTES {
            return Ok(FileView {
                repo: repo_id,
                repo_label,
                path: requested_path,
                language,
                line_count,
                highlighted_html: None,
                plain_text: Some(text),
                truncated: true,
            });
        }

        let highlighted_html = language
            .as_deref()
            .and_then(|language| self.highlighter.highlight(language, &text).ok());
        let plain_text = highlighted_html.is_none().then_some(text);

        Ok(FileView {
            repo: repo_id,
            repo_label,
            path: requested_path,
            language,
            line_count,
            highlighted_html,
            plain_text,
            truncated: false,
        })
    }

    fn resolve_file(&self, repo_id: &str, relative_path: &str) -> Result<ResolvedFile<'_>> {
        let repo = self
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .ok_or_else(|| anyhow!("unknown repository `{repo_id}`"))?;
        let requested_path = Utf8PathBuf::from(relative_path);
        if requested_path.is_absolute() {
            bail!("file path must be repo-relative");
        }

        let absolute = repo.root.join(&requested_path);
        let canonical_path = canonicalize_utf8(&absolute)?;
        if !canonical_path.starts_with(&repo.canonical_root) {
            bail!("file path escapes repository root");
        }

        Ok(ResolvedFile {
            repo,
            canonical_path,
            requested_path,
        })
    }

    fn build_query(&self, request: &SearchRequest) -> Result<Box<dyn Query>> {
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        if !request.query.trim().is_empty() {
            let mut parser = QueryParser::for_index(
                &self.index,
                vec![
                    self.schema.content,
                    self.schema.content_ngram,
                    self.schema.path,
                    self.schema.path_ngram,
                    self.schema.file,
                    self.schema.file_ngram,
                ],
            );
            parser.set_conjunction_by_default();
            clauses.push((Occur::Must, parser.parse_query(&request.query)?));
        }

        for repo in &request.repo_filters {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.schema.repo, &repo.to_ascii_lowercase()),
                    IndexRecordOption::Basic,
                )),
            ));
        }

        for lang in &request.lang_filters {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.schema.language, &lang.to_ascii_lowercase()),
                    IndexRecordOption::Basic,
                )),
            ));
        }

        for path in &request.path_filters {
            let mut parser =
                QueryParser::for_index(&self.index, vec![self.schema.path, self.schema.path_ngram]);
            parser.set_conjunction_by_default();
            clauses.push((Occur::Must, parser.parse_query(path)?));
        }

        for file in &request.file_filters {
            let mut parser =
                QueryParser::for_index(&self.index, vec![self.schema.file, self.schema.file_ngram]);
            parser.set_conjunction_by_default();
            clauses.push((Occur::Must, parser.parse_query(file)?));
        }

        if clauses.is_empty() {
            return Ok(Box::new(AllQuery));
        }

        Ok(Box::new(BooleanQuery::new(clauses)))
    }

    fn hit_from_doc(
        &self,
        searcher: &tantivy::Searcher,
        address: tantivy::DocAddress,
    ) -> Result<SearchHit> {
        let document: TantivyDocument = searcher.doc(address)?;
        Ok(SearchHit {
            repo: stored_text(&document, self.schema.repo)?,
            repo_label: stored_text(&document, self.schema.repo_label)?,
            path: stored_text(&document, self.schema.path_raw)?,
            file: stored_text(&document, self.schema.file_raw)?,
            language: document
                .get_first(self.schema.language_raw)
                .and_then(|value| value.as_str())
                .map(str::to_owned),
            line_number: stored_u64(&document, self.schema.line_number)?,
            text: stored_text(&document, self.schema.text_raw)?,
        })
    }
}

impl SearchSchema {
    fn build() -> (Schema, Self) {
        let mut builder = Schema::builder();

        let repo = builder.add_text_field("repo", STRING | STORED);
        let repo_label = builder.add_text_field("repo_label", STORED);
        let path_raw = builder.add_text_field("path_raw", STORED);
        let file_raw = builder.add_text_field("file_raw", STORED);
        let language_raw = builder.add_text_field("language_raw", STORED);
        let line_number = builder.add_u64_field(
            "line_number",
            NumericOptions::default()
                .set_fast()
                .set_stored()
                .set_indexed(),
        );
        let sort_key = builder.add_text_field("sort_key", STRING | FAST);
        let text_raw = builder.add_text_field("text_raw", STORED);

        let code_text = text_options("code_terms");
        let ngram_text = text_options("ngram3");
        let exact_lower = builder.add_text_field("language", STRING);
        let content = builder.add_text_field("content", code_text.clone());
        let content_ngram = builder.add_text_field("content_ngram", ngram_text.clone());
        let path = builder.add_text_field("path", code_text.clone());
        let path_ngram = builder.add_text_field("path_ngram", ngram_text.clone());
        let file = builder.add_text_field("file", code_text);
        let file_ngram = builder.add_text_field("file_ngram", ngram_text);

        let schema = builder.build();
        (
            schema,
            Self {
                repo,
                repo_label,
                path_raw,
                file_raw,
                language_raw,
                line_number,
                sort_key,
                text_raw,
                content,
                content_ngram,
                path,
                path_ngram,
                file,
                file_ngram,
                language: exact_lower,
            },
        )
    }

    fn from_schema(schema: &Schema) -> Result<Self> {
        Ok(Self {
            repo: schema.get_field("repo")?,
            repo_label: schema.get_field("repo_label")?,
            path_raw: schema.get_field("path_raw")?,
            file_raw: schema.get_field("file_raw")?,
            language_raw: schema.get_field("language_raw")?,
            line_number: schema.get_field("line_number")?,
            sort_key: schema.get_field("sort_key")?,
            text_raw: schema.get_field("text_raw")?,
            content: schema.get_field("content")?,
            content_ngram: schema.get_field("content_ngram")?,
            path: schema.get_field("path")?,
            path_ngram: schema.get_field("path_ngram")?,
            file: schema.get_field("file")?,
            file_ngram: schema.get_field("file_ngram")?,
            language: schema.get_field("language")?,
        })
    }
}

pub fn parse_search_request(
    query: &str,
    repo: Option<String>,
    path: Option<String>,
    file: Option<String>,
    lang: Option<String>,
    limit: usize,
) -> SearchRequest {
    let mut parts = query_parts(query);
    if let Some(repo) = repo {
        parts.repo_filters.push(repo);
    }
    if let Some(path) = path {
        parts.path_filters.push(path);
    }
    if let Some(file) = file {
        parts.file_filters.push(file);
    }
    if let Some(lang) = lang {
        parts.lang_filters.push(lang);
    }
    SearchRequest {
        query: normalize_query_string(parts.query.trim()),
        repo_filters: parts.repo_filters,
        path_filters: parts
            .path_filters
            .into_iter()
            .map(|value| normalize_query_string(&value))
            .collect(),
        file_filters: parts
            .file_filters
            .into_iter()
            .map(|value| normalize_query_string(&value))
            .collect(),
        lang_filters: parts.lang_filters,
        limit,
    }
}

pub fn refresh_index<F>(
    context: &ContextState,
    mode: RefreshMode,
    mut on_progress: F,
) -> Result<IndexBuildReport>
where
    F: FnMut(IndexProgress),
{
    let paths = index_paths(context);
    on_progress(IndexProgress {
        phase: IndexPhase::Scanning,
        message: "Scanning workspace repositories…".to_owned(),
        files_scanned: 0,
        files_total: None,
        files_indexed: 0,
        lines_indexed: 0,
        skipped_binary: 0,
        skipped_large: 0,
    });
    let snapshot =
        collect_workspace_snapshot(context.workspace_paths.repos_root(), &mut on_progress)?;
    let manifest = load_manifest(&paths.manifest_path)?;
    match refresh_plan(mode, &paths, manifest.as_ref(), &snapshot) {
        RefreshPlan::FullRebuild => rebuild_index(&paths, &snapshot, &mut on_progress),
        RefreshPlan::Partial { dirty_repo_ids } => {
            partial_refresh_index(&paths, &snapshot, &dirty_repo_ids, &mut on_progress)
        }
        RefreshPlan::Unchanged => Ok(IndexBuildReport {
            rebuilt: false,
            partial: false,
            repos_refreshed: 0,
            files_scanned: snapshot.files.len(),
            files_indexed: total_indexed_files(&snapshot),
            lines_indexed: 0,
            skipped_binary: snapshot.skipped_binary,
            skipped_large: snapshot.skipped_large,
            index_path: paths.index_dir.to_string(),
        }),
    }
}

enum RefreshPlan {
    FullRebuild,
    Partial { dirty_repo_ids: Vec<String> },
    Unchanged,
}

fn refresh_plan(
    mode: RefreshMode,
    paths: &SearchIndexPaths,
    manifest: Option<&IndexManifest>,
    snapshot: &WorkspaceSnapshot,
) -> RefreshPlan {
    if matches!(mode, RefreshMode::ForceRebuild) || !paths.index_dir.join("meta.json").exists() {
        return RefreshPlan::FullRebuild;
    }

    let Some(manifest) = manifest else {
        return RefreshPlan::FullRebuild;
    };
    if manifest.version != INDEX_MANIFEST_VERSION {
        return RefreshPlan::FullRebuild;
    }

    let dirty_repo_ids = dirty_repo_ids(manifest, snapshot);
    if dirty_repo_ids.is_empty() {
        RefreshPlan::Unchanged
    } else {
        RefreshPlan::Partial { dirty_repo_ids }
    }
}

fn dirty_repo_ids(manifest: &IndexManifest, snapshot: &WorkspaceSnapshot) -> Vec<String> {
    let mut previous = BTreeMap::<String, Vec<ManifestEntry>>::new();
    for entry in &manifest.files {
        previous
            .entry(entry.repo_id.clone())
            .or_default()
            .push(entry.clone());
    }

    let mut current = BTreeMap::<String, Vec<ManifestEntry>>::new();
    for file in &snapshot.files {
        current
            .entry(file.manifest.repo_id.clone())
            .or_default()
            .push(file.manifest.clone());
    }

    previous
        .keys()
        .chain(current.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|repo_id| previous.get(repo_id) != current.get(repo_id))
        .collect()
}

fn index_paths(context: &ContextState) -> SearchIndexPaths {
    let workspace_hash = workspace_hash(context.workspace_paths.root().as_str());
    let index_dir = context.app_paths.search_index_root().join(workspace_hash);
    SearchIndexPaths {
        manifest_path: index_dir.join("manifest.json"),
        index_dir,
    }
}

fn rebuild_index<F>(
    paths: &SearchIndexPaths,
    snapshot: &WorkspaceSnapshot,
    on_progress: &mut F,
) -> Result<IndexBuildReport>
where
    F: FnMut(IndexProgress),
{
    let (staging_dir, backup_dir, staging_manifest_path) = prepare_staging_dirs(paths)?;
    let (schema, fields) = SearchSchema::build();
    let index = Index::create_in_dir(staging_dir.as_std_path(), schema)?;
    register_tokenizers(&index);

    let mut writer: IndexWriter = index.writer(INDEX_WRITER_BYTES)?;
    let _ = index_snapshot_lines(snapshot, &fields, &writer, None, on_progress)?;

    writer.commit()?;
    save_manifest(
        &staging_manifest_path,
        &IndexManifest {
            version: INDEX_MANIFEST_VERSION,
            files: manifest_entries(&snapshot.files),
        },
    )?;
    replace_live_index(paths, &staging_dir, &backup_dir)?;

    Ok(IndexBuildReport {
        rebuilt: true,
        partial: false,
        repos_refreshed: repository_count(snapshot),
        files_scanned: snapshot.files.len(),
        files_indexed: total_indexed_files(snapshot),
        lines_indexed: total_indexed_lines(snapshot),
        skipped_binary: snapshot.skipped_binary,
        skipped_large: snapshot.skipped_large,
        index_path: paths.index_dir.to_string(),
    })
}

fn partial_refresh_index<F>(
    paths: &SearchIndexPaths,
    snapshot: &WorkspaceSnapshot,
    dirty_repo_ids: &[String],
    on_progress: &mut F,
) -> Result<IndexBuildReport>
where
    F: FnMut(IndexProgress),
{
    let index = Index::open_in_dir(paths.index_dir.as_std_path())
        .with_context(|| format!("failed to open search index in {}", paths.index_dir))?;
    register_tokenizers(&index);
    let fields = SearchSchema::from_schema(&index.schema())?;
    let mut writer: IndexWriter = index.writer(INDEX_WRITER_BYTES)?;
    let dirty_repo_ids = dirty_repo_ids.iter().cloned().collect::<BTreeSet<_>>();

    for repo_id in &dirty_repo_ids {
        writer.delete_term(Term::from_field_text(fields.repo, repo_id));
    }

    let _ = index_snapshot_lines(
        snapshot,
        &fields,
        &writer,
        Some(&dirty_repo_ids),
        on_progress,
    )?;

    writer.commit()?;
    save_manifest(
        &paths.manifest_path,
        &IndexManifest {
            version: INDEX_MANIFEST_VERSION,
            files: manifest_entries(&snapshot.files),
        },
    )?;

    Ok(IndexBuildReport {
        rebuilt: true,
        partial: true,
        repos_refreshed: dirty_repo_ids.len(),
        files_scanned: snapshot.files.len(),
        files_indexed: total_indexed_files(snapshot),
        lines_indexed: total_indexed_lines(snapshot),
        skipped_binary: snapshot.skipped_binary,
        skipped_large: snapshot.skipped_large,
        index_path: paths.index_dir.to_string(),
    })
}

fn prepare_staging_dirs(
    paths: &SearchIndexPaths,
) -> Result<(Utf8PathBuf, Utf8PathBuf, Utf8PathBuf)> {
    let staging_dir = Utf8PathBuf::from(format!("{}.staging", paths.index_dir));
    let backup_dir = Utf8PathBuf::from(format!("{}.previous", paths.index_dir));
    let staging_manifest_path = staging_dir.join("manifest.json");

    if staging_dir.exists() {
        fs::remove_dir_all(staging_dir.as_std_path())
            .with_context(|| format!("failed to remove {staging_dir}"))?;
    }
    if backup_dir.exists() {
        fs::remove_dir_all(backup_dir.as_std_path())
            .with_context(|| format!("failed to remove {backup_dir}"))?;
    }
    fs::create_dir_all(staging_dir.as_std_path())
        .with_context(|| format!("failed to create {staging_dir}"))?;

    Ok((staging_dir, backup_dir, staging_manifest_path))
}

fn index_snapshot_lines<F>(
    snapshot: &WorkspaceSnapshot,
    fields: &SearchSchema,
    writer: &IndexWriter,
    dirty_repo_ids: Option<&BTreeSet<String>>,
    on_progress: &mut F,
) -> Result<(usize, usize)>
where
    F: FnMut(IndexProgress),
{
    let files_total = snapshot
        .files
        .iter()
        .filter(|file| {
            file.manifest.status == IndexedFileStatus::Text
                && dirty_repo_ids.is_none_or(|repo_ids| repo_ids.contains(&file.manifest.repo_id))
        })
        .count();
    let mut files_indexed = 0_usize;
    let mut lines_indexed = 0_usize;

    on_progress(building_progress(
        snapshot,
        files_total,
        0,
        0,
        format!("Indexing {files_total} text files…"),
    ));

    for file in &snapshot.files {
        if file.manifest.status != IndexedFileStatus::Text
            || dirty_repo_ids.is_some_and(|repo_ids| !repo_ids.contains(&file.manifest.repo_id))
        {
            continue;
        }
        let Some(text) = file.text.as_ref() else {
            continue;
        };

        files_indexed += 1;
        let language = file
            .manifest
            .language
            .clone()
            .unwrap_or_else(|| "text".to_owned());
        let normalized_path = normalize_searchable_text(file.relative_path.as_str());
        let normalized_file = normalize_searchable_text(&file.file_name);

        for (index_in_file, line) in text.lines().enumerate() {
            let line_number = u64::try_from(index_in_file + 1).unwrap_or(u64::MAX);
            let normalized_line = normalize_searchable_text(line);
            writer.add_document(doc!(
                fields.repo => file.manifest.repo_id.clone(),
                fields.repo_label => file.repo_label.clone(),
                fields.path_raw => file.relative_path.to_string(),
                fields.file_raw => file.file_name.clone(),
                fields.language_raw => language.clone(),
                fields.language => language.to_ascii_lowercase(),
                fields.line_number => line_number,
                fields.sort_key => sort_key(&file.manifest.repo_id, &file.relative_path, line_number),
                fields.text_raw => line.to_owned(),
                fields.content => normalized_line.clone(),
                fields.content_ngram => normalized_line,
                fields.path => normalized_path.clone(),
                fields.path_ngram => normalized_path.clone(),
                fields.file => normalized_file.clone(),
                fields.file_ngram => normalized_file.clone(),
            ))?;
            lines_indexed += 1;
        }

        on_progress(building_progress(
            snapshot,
            files_total,
            files_indexed,
            lines_indexed,
            format!("Indexed {files_indexed} of {files_total} text files…"),
        ));
    }

    Ok((files_indexed, lines_indexed))
}

fn total_indexed_files(snapshot: &WorkspaceSnapshot) -> usize {
    snapshot
        .files
        .iter()
        .filter(|file| file.manifest.status == IndexedFileStatus::Text)
        .count()
}

fn total_indexed_lines(snapshot: &WorkspaceSnapshot) -> usize {
    snapshot
        .files
        .iter()
        .filter(|file| file.manifest.status == IndexedFileStatus::Text)
        .filter_map(|file| file.text.as_ref())
        .map(|text| text.lines().count())
        .sum()
}

fn repository_count(snapshot: &WorkspaceSnapshot) -> usize {
    snapshot
        .files
        .iter()
        .map(|file| file.manifest.repo_id.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

fn sort_key(repo_id: &str, relative_path: &Utf8Path, line_number: u64) -> String {
    format!("{repo_id}\u{1f}{relative_path}\u{1f}{line_number:010}")
}

const fn building_progress(
    snapshot: &WorkspaceSnapshot,
    files_total: usize,
    files_indexed: usize,
    lines_indexed: usize,
    message: String,
) -> IndexProgress {
    IndexProgress {
        phase: IndexPhase::Building,
        message,
        files_scanned: snapshot.files.len(),
        files_total: Some(files_total),
        files_indexed,
        lines_indexed,
        skipped_binary: snapshot.skipped_binary,
        skipped_large: snapshot.skipped_large,
    }
}

fn replace_live_index(
    paths: &SearchIndexPaths,
    staging_dir: &Utf8Path,
    backup_dir: &Utf8Path,
) -> Result<()> {
    if paths.index_dir.exists() {
        fs::rename(paths.index_dir.as_std_path(), backup_dir.as_std_path()).with_context(|| {
            format!(
                "failed to move existing search index from {} to {backup_dir}",
                paths.index_dir
            )
        })?;
    }

    if let Err(error) = fs::rename(staging_dir.as_std_path(), paths.index_dir.as_std_path()) {
        if backup_dir.exists() && !paths.index_dir.exists() {
            let _ = fs::rename(backup_dir.as_std_path(), paths.index_dir.as_std_path());
        }
        return Err(anyhow!(
            "failed to move new search index from {staging_dir} to {}: {error}",
            paths.index_dir
        ));
    }

    if backup_dir.exists() {
        fs::remove_dir_all(backup_dir.as_std_path())
            .with_context(|| format!("failed to remove {backup_dir}"))?;
    }

    Ok(())
}

fn collect_workspace_snapshot<F>(
    repos_root: &Utf8Path,
    on_progress: &mut F,
) -> Result<WorkspaceSnapshot>
where
    F: FnMut(IndexProgress),
{
    let repos = discover_repositories(repos_root)?;
    let mut files = Vec::new();
    let mut skipped_binary = 0;
    let mut skipped_large = 0;
    let mut files_scanned = 0;

    on_progress(IndexProgress {
        phase: IndexPhase::Scanning,
        message: format!("Discovered {} repositories. Reading files…", repos.len()),
        files_scanned,
        files_total: None,
        files_indexed: 0,
        lines_indexed: 0,
        skipped_binary,
        skipped_large,
    });

    for repo in &repos {
        let mut repo_files = collect_repo_files(
            repo,
            &mut files_scanned,
            &mut skipped_binary,
            &mut skipped_large,
            on_progress,
        )?;
        files.append(&mut repo_files);
    }

    files.sort_by(|left, right| {
        left.manifest
            .repo_id
            .cmp(&right.manifest.repo_id)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });

    Ok(WorkspaceSnapshot {
        files,
        skipped_binary,
        skipped_large,
    })
}

fn discover_repositories(repos_root: &Utf8Path) -> Result<Vec<RepositoryEntry>> {
    if !repos_root.exists() {
        return Ok(Vec::new());
    }

    let mut repositories = Vec::new();
    let repos_root = canonicalize_utf8(repos_root)?;
    let mut queue = VecDeque::from([repos_root.clone()]);

    while let Some(directory) = queue.pop_front() {
        if directory.join(".git").exists() {
            let relative = directory
                .strip_prefix(&repos_root)
                .map_err(|_| anyhow!("{directory} is not inside {repos_root}"))?;
            let segments = relative.iter().collect::<Vec<_>>();
            if segments.len() >= 2 {
                let repo_id = format!("{}/{}", segments[0], segments[1..].join("/"));
                repositories.push(RepositoryEntry {
                    label: segments.last().copied().unwrap_or_default().to_owned(),
                    id: repo_id,
                    canonical_root: directory.clone(),
                    root: directory,
                });
            }
            continue;
        }

        for entry in fs::read_dir(directory.as_std_path())
            .with_context(|| format!("failed to read {directory}"))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = utf8_from_path_buf(entry.path())?;
            if path.file_name() == Some(".git") {
                continue;
            }
            queue.push_back(path);
        }
    }

    repositories.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(repositories)
}

fn collect_repo_files<F>(
    repo: &RepositoryEntry,
    files_scanned: &mut usize,
    skipped_binary: &mut usize,
    skipped_large: &mut usize,
    on_progress: &mut F,
) -> Result<Vec<CollectedFile>>
where
    F: FnMut(IndexProgress),
{
    let mut builder = WalkBuilder::new(repo.root.as_std_path());
    builder.standard_filters(true);
    builder.follow_links(false);
    let mut files = Vec::new();

    for entry in builder.build() {
        let entry = entry?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = utf8_from_path_buf(entry.path().to_path_buf())?;
        let relative = path
            .strip_prefix(&repo.root)
            .map_err(|_| anyhow!("{path} is not inside {}", repo.root))?
            .to_owned();
        let metadata =
            fs::metadata(path.as_std_path()).with_context(|| format!("failed to stat {path}"))?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok());
        let modified_secs = modified.map_or(0, |duration| duration.as_secs());
        let modified_nanos = modified.map_or(0, |duration| duration.subsec_nanos());
        let language = detect_language(relative.as_str()).map(str::to_owned);
        let status;
        let text;

        if metadata.len() > MAX_INDEXED_FILE_BYTES {
            status = IndexedFileStatus::TooLarge;
            text = None;
            *skipped_large += 1;
        } else {
            let bytes =
                fs::read(path.as_std_path()).with_context(|| format!("failed to read {path}"))?;
            if bytes.contains(&0) {
                status = IndexedFileStatus::Binary;
                text = None;
                *skipped_binary += 1;
            } else {
                status = IndexedFileStatus::Text;
                text = Some(String::from_utf8_lossy(&bytes).into_owned());
            }
        }

        let file_name = relative
            .file_name()
            .ok_or_else(|| anyhow!("failed to determine a file name for {relative}"))?
            .to_owned();
        files.push(CollectedFile {
            manifest: ManifestEntry {
                key: format!("{}:{}", repo.id, relative),
                repo_id: repo.id.clone(),
                relative_path: relative.to_string(),
                size: metadata.len(),
                modified_secs,
                modified_nanos,
                language,
                status,
            },
            repo_label: repo.label.clone(),
            relative_path: relative,
            file_name,
            text,
        });
        *files_scanned += 1;
        if *files_scanned == 1 || (*files_scanned).is_multiple_of(32) {
            on_progress(IndexProgress {
                phase: IndexPhase::Scanning,
                message: format!("Scanning {}…", repo.label),
                files_scanned: *files_scanned,
                files_total: None,
                files_indexed: 0,
                lines_indexed: 0,
                skipped_binary: *skipped_binary,
                skipped_large: *skipped_large,
            });
        }
    }

    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    on_progress(IndexProgress {
        phase: IndexPhase::Scanning,
        message: format!("Scanned {} in {} files.", repo.label, files.len()),
        files_scanned: *files_scanned,
        files_total: None,
        files_indexed: 0,
        lines_indexed: 0,
        skipped_binary: *skipped_binary,
        skipped_large: *skipped_large,
    });
    Ok(files)
}

fn load_manifest(path: &Utf8Path) -> Result<Option<IndexManifest>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw =
        fs::read_to_string(path.as_std_path()).with_context(|| format!("failed to read {path}"))?;
    let manifest = serde_json::from_str(&raw).with_context(|| format!("failed to parse {path}"))?;
    Ok(Some(manifest))
}

fn save_manifest(path: &Utf8Path, manifest: &IndexManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent.as_std_path())
            .with_context(|| format!("failed to create {parent}"))?;
    }
    let temp_path = path.with_extension("json.tmp");
    let raw = serde_json::to_string_pretty(manifest)?;
    fs::write(temp_path.as_std_path(), raw)
        .with_context(|| format!("failed to write {temp_path}"))?;
    fs::rename(temp_path.as_std_path(), path.as_std_path())
        .with_context(|| format!("failed to move {temp_path} into {path}"))?;
    Ok(())
}

fn manifest_entries(files: &[CollectedFile]) -> Vec<ManifestEntry> {
    files.iter().map(|file| file.manifest.clone()).collect()
}

fn register_tokenizers(index: &Index) {
    let code_terms = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(MAX_TOKEN_BYTES))
        .filter(LowerCaser)
        .build();
    index.tokenizers().register("code_terms", code_terms);
    index.tokenizers().register(
        "ngram3",
        NgramTokenizer::new(3, 3, false)
            .unwrap_or_else(|_| unreachable!("3-gram tokenizer configuration is valid")),
    );
}

fn text_options(tokenizer: &str) -> TextOptions {
    let indexing = TextFieldIndexing::default()
        .set_tokenizer(tokenizer)
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    TextOptions::default().set_indexing_options(indexing)
}

fn normalize_searchable_text(input: &str) -> String {
    let mut normalized = String::with_capacity(input.len() * 2);
    let chars = input.chars().collect::<Vec<_>>();

    for (index, character) in chars.iter().copied().enumerate() {
        if should_insert_boundary(index, character, &chars) {
            normalized.push(' ');
        }

        match character {
            '/' | '\\' | '_' | '-' | '.' | ':' => normalized.push(' '),
            _ if character.is_alphanumeric() => {
                for lower in character.to_lowercase() {
                    normalized.push(lower);
                }
            }
            _ => normalized.push(' '),
        }
    }

    normalized
}

fn should_insert_boundary(index: usize, character: char, chars: &[char]) -> bool {
    if index == 0 {
        return false;
    }
    let previous = chars[index - 1];
    let next = chars.get(index + 1).copied();

    (previous.is_lowercase() && character.is_uppercase())
        || (previous.is_alphabetic() && character.is_numeric())
        || (previous.is_numeric() && character.is_alphabetic())
        || (previous.is_uppercase()
            && character.is_uppercase()
            && next.is_some_and(char::is_lowercase))
}

fn workspace_hash(input: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn stored_text(document: &TantivyDocument, field: Field) -> Result<String> {
    document
        .get_first(field)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("missing stored text field"))
}

fn stored_u64(document: &TantivyDocument, field: Field) -> Result<u64> {
    document
        .get_first(field)
        .and_then(|value| value.as_u64())
        .ok_or_else(|| anyhow!("missing stored numeric field"))
}

#[derive(Debug)]
struct QueryParts {
    query: String,
    repo_filters: Vec<String>,
    path_filters: Vec<String>,
    file_filters: Vec<String>,
    lang_filters: Vec<String>,
}

fn query_parts(input: &str) -> QueryParts {
    let mut query_tokens = Vec::new();
    let mut repo_filters = Vec::new();
    let mut path_filters = Vec::new();
    let mut file_filters = Vec::new();
    let mut lang_filters = Vec::new();

    for token in split_query_tokens(input) {
        if let Some(value) = token.strip_prefix("repo:") {
            repo_filters.push(trim_matching_quotes(value).to_owned());
        } else if let Some(value) = token.strip_prefix("path:") {
            path_filters.push(trim_matching_quotes(value).to_owned());
        } else if let Some(value) = token.strip_prefix("file:") {
            file_filters.push(trim_matching_quotes(value).to_owned());
        } else if let Some(value) = token.strip_prefix("lang:") {
            lang_filters.push(trim_matching_quotes(value).to_owned());
        } else {
            query_tokens.push(token);
        }
    }

    QueryParts {
        query: query_tokens.join(" "),
        repo_filters,
        path_filters,
        file_filters,
        lang_filters,
    }
}

fn split_query_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for character in input.chars() {
        match character {
            '"' => {
                in_quotes = !in_quotes;
                current.push(character);
            }
            _ if character.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn trim_matching_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|trimmed| trimmed.strip_suffix('"'))
        .unwrap_or(value)
}

fn normalize_query_string(input: &str) -> String {
    split_query_tokens(input)
        .into_iter()
        .filter_map(|token| {
            let normalized = normalize_query_token(&token);
            if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_query_token(token: &str) -> String {
    if token.starts_with('"') && token.ends_with('"') && token.len() >= 2 {
        let inner = &token[1..token.len() - 1];
        let normalized = normalize_searchable_text(inner).trim().to_owned();
        return format!("\"{normalized}\"");
    }

    // Preserve Tantivy wildcard syntax, but normalize everything else so path-ish
    // queries like `agent/crates/intar-agent/proto/.../probes.proto` still map to
    // the same tokenized terms we indexed.
    if token
        .chars()
        .any(|character| matches!(character, '*' | '?'))
    {
        return token.to_owned();
    }

    normalize_searchable_text(token).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        RefreshMode, SearchEngine, normalize_query_string, normalize_searchable_text,
        parse_search_request, query_parts, refresh_index, split_query_tokens, workspace_hash,
    };
    use crate::context::ContextState;
    use camino::Utf8PathBuf;
    use nanite_core::{AgentKind, AppPaths, Config};
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::fs;
    use tempfile::TempDir;

    struct Harness {
        _tempdir: TempDir,
        context: ContextState,
    }

    impl Harness {
        fn new() -> Self {
            let tempdir = tempfile::tempdir().unwrap();
            let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
            let home_dir = root.join("home");
            let config_dir = root.join("config");
            let data_dir = root.join("data");
            let state_dir = root.join("state");
            let workspace_root = root.join("workspace");
            fs::create_dir_all(&home_dir).unwrap();
            fs::create_dir_all(&config_dir).unwrap();
            fs::create_dir_all(&data_dir).unwrap();
            fs::create_dir_all(&state_dir).unwrap();
            fs::create_dir_all(workspace_root.join("repos")).unwrap();

            let env = HashMap::from([
                ("HOME".to_owned(), home_dir.to_string()),
                ("NANITE_CONFIG_DIR".to_owned(), config_dir.to_string()),
                ("NANITE_DATA_DIR".to_owned(), data_dir.to_string()),
                ("NANITE_STATE_DIR".to_owned(), state_dir.to_string()),
            ]);
            let app_paths = AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap();
            let config = Config {
                workspace_root,
                agent: AgentKind::Codex,
            };
            let workspace_paths = config.workspace_paths();

            Self {
                _tempdir: tempdir,
                context: ContextState {
                    app_paths,
                    config,
                    workspace_paths,
                    git_binary: "git".to_owned(),
                    fzf_binary: "fzf".to_owned(),
                    zed_binary: "zed".to_owned(),
                },
            }
        }

        fn create_repo(&self, repo_id: &str) -> Utf8PathBuf {
            let path = self.context.workspace_paths.repos_root().join(repo_id);
            fs::create_dir_all(path.join(".git")).unwrap();
            path
        }
    }

    #[test]
    fn query_parts_extract_qualifiers() {
        let parts = query_parts(
            r#"repo:github.com/icepuma/nanite path:"src/search" lang:rust "search ui""#,
        );

        assert_eq!(parts.repo_filters, vec!["github.com/icepuma/nanite"]);
        assert_eq!(parts.path_filters, vec!["src/search"]);
        assert_eq!(parts.lang_filters, vec!["rust"]);
        assert_eq!(parts.query, "\"search ui\"");
    }

    #[test]
    fn split_query_tokens_preserves_phrases() {
        assert_eq!(
            split_query_tokens(r#"foo "bar baz" repo:test path:"src/lib""#),
            vec!["foo", "\"bar baz\"", "repo:test", "path:\"src/lib\""]
        );
    }

    #[test]
    fn normalize_searchable_text_splits_code_identifiers() {
        assert_eq!(
            normalize_searchable_text("HttpServer_path.rs"),
            "http server path rs"
        );
    }

    #[test]
    fn normalize_query_string_keeps_phrases_and_splits_identifiers() {
        assert_eq!(
            normalize_query_string(r#""HttpServer" workspace_root"#),
            r#""http server" workspace root"#
        );
    }

    #[test]
    fn workspace_hash_is_stable() {
        assert_eq!(workspace_hash("/tmp/nanite"), workspace_hash("/tmp/nanite"));
    }

    #[test]
    fn engine_indexes_workspace_files_and_normalizes_queries() {
        let harness = Harness::new();
        let repo = harness.create_repo("github.com/icepuma/nanite");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(
            repo.join("src/lib.rs"),
            "pub fn workspace_root() {}\nstruct HttpServer;\n",
        )
        .unwrap();

        let open = SearchEngine::open(&harness.context, super::RefreshMode::ForceRebuild).unwrap();

        let snake_case = open
            .engine
            .search(&parse_search_request(
                "workspace_root",
                None,
                None,
                None,
                None,
                10,
            ))
            .unwrap();
        assert_eq!(snake_case.hits.len(), 1);
        assert_eq!(snake_case.hits[0].path, "src/lib.rs");

        let camel_case = open
            .engine
            .search(&parse_search_request(
                "HttpServer",
                None,
                None,
                None,
                None,
                10,
            ))
            .unwrap();
        assert_eq!(camel_case.hits.len(), 1);
        assert_eq!(camel_case.hits[0].line_number, 2);
    }

    #[test]
    fn reindex_picks_up_changed_and_removed_files() {
        let harness = Harness::new();
        let repo = harness.create_repo("github.com/icepuma/nanite");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join("src/lib.rs"), "pub fn workspace_root() {}\n").unwrap();
        fs::write(repo.join("src/obsolete.rs"), "pub fn stale_token() {}\n").unwrap();

        SearchEngine::open(&harness.context, RefreshMode::ForceRebuild).unwrap();
        fs::write(repo.join("src/lib.rs"), "pub fn workspace_token() {}\n").unwrap();
        fs::remove_file(repo.join("src/obsolete.rs")).unwrap();

        let report = refresh_index(&harness.context, RefreshMode::ForceRebuild, |_| {}).unwrap();
        assert!(report.rebuilt);
        let engine = SearchEngine::open_existing(&harness.context)
            .unwrap()
            .unwrap();

        let old = engine
            .search(&parse_search_request(
                "workspace_root",
                None,
                None,
                None,
                None,
                10,
            ))
            .unwrap();
        assert!(old.hits.is_empty());

        let new = engine
            .search(&parse_search_request(
                "workspace_token",
                None,
                None,
                None,
                None,
                10,
            ))
            .unwrap();
        assert_eq!(new.hits.len(), 1);

        let removed = engine
            .search(&parse_search_request(
                "stale_token",
                None,
                None,
                None,
                None,
                10,
            ))
            .unwrap();
        assert!(removed.hits.is_empty());
    }

    #[test]
    fn refresh_if_needed_only_reindexes_dirty_repositories() {
        let harness = Harness::new();
        let repo_a = harness.create_repo("github.com/icepuma/nanite");
        let repo_b = harness.create_repo("github.com/icepuma/another");
        fs::create_dir_all(repo_a.join("src")).unwrap();
        fs::create_dir_all(repo_b.join("src")).unwrap();
        fs::write(repo_a.join("src/lib.rs"), "pub fn alpha_token() {}\n").unwrap();
        fs::write(repo_b.join("src/lib.rs"), "pub fn stable_token() {}\n").unwrap();

        SearchEngine::open(&harness.context, RefreshMode::ForceRebuild).unwrap();
        fs::write(
            repo_a.join("src/lib.rs"),
            "pub fn beta_token_updated() {}\n",
        )
        .unwrap();

        let report = refresh_index(&harness.context, RefreshMode::RefreshIfNeeded, |_| {}).unwrap();
        assert!(report.rebuilt);
        assert!(report.partial);
        assert_eq!(report.repos_refreshed, 1);

        let engine = SearchEngine::open_existing(&harness.context)
            .unwrap()
            .unwrap();
        let updated = engine
            .search(&parse_search_request(
                "beta_token_updated",
                None,
                None,
                None,
                None,
                10,
            ))
            .unwrap();
        assert_eq!(updated.hits.len(), 1);
        assert_eq!(updated.hits[0].repo, "github.com/icepuma/nanite");

        let stable = engine
            .search(&parse_search_request(
                "stable_token",
                None,
                None,
                None,
                None,
                10,
            ))
            .unwrap();
        assert_eq!(stable.hits.len(), 1);
        assert_eq!(stable.hits[0].repo, "github.com/icepuma/another");

        let old = engine
            .search(&parse_search_request(
                "alpha_token",
                None,
                None,
                None,
                None,
                10,
            ))
            .unwrap();
        assert!(old.hits.is_empty());
    }

    #[test]
    fn full_path_queries_normalize_hyphens_and_separators() {
        let harness = Harness::new();
        let repo = harness.create_repo("github.com/icepuma/nanite");
        let path = repo.join("agent/crates/intar-agent/proto/kino/v1");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("probes.proto"), "message Probe {}\n").unwrap();

        let open = SearchEngine::open(&harness.context, RefreshMode::ForceRebuild).unwrap();

        let general = open
            .engine
            .search(&parse_search_request(
                "agent/crates/intar-agent/proto/kino/v1/probes.proto",
                None,
                None,
                None,
                None,
                10,
            ))
            .unwrap();
        assert_eq!(general.hits.len(), 1);
        assert_eq!(
            general.hits[0].path,
            "agent/crates/intar-agent/proto/kino/v1/probes.proto"
        );

        let path_filter = open
            .engine
            .search(&parse_search_request(
                "",
                None,
                Some("agent/crates/intar-agent/proto/kino/v1/probes.proto".to_owned()),
                None,
                None,
                10,
            ))
            .unwrap();
        assert_eq!(path_filter.hits.len(), 1);
        assert_eq!(
            path_filter.hits[0].path,
            "agent/crates/intar-agent/proto/kino/v1/probes.proto"
        );
    }

    #[test]
    fn file_view_highlights_supported_languages_falls_back_to_plain_text_and_rejects_traversal() {
        let harness = Harness::new();
        let repo = harness.create_repo("github.com/icepuma/nanite");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(repo.join("proto/kino/v1")).unwrap();
        fs::write(repo.join("src/lib.rs"), "fn main() {}\n").unwrap();
        fs::write(
            repo.join("proto/kino/v1/probes.proto"),
            "syntax = \"proto3\";\nmessage Probe {}\n",
        )
        .unwrap();

        let mut open =
            SearchEngine::open(&harness.context, super::RefreshMode::ForceRebuild).unwrap();
        let highlighted_view = open
            .engine
            .file_view("github.com/icepuma/nanite", "src/lib.rs")
            .unwrap();
        assert!(highlighted_view.highlighted_html.is_some());
        assert!(
            highlighted_view
                .highlighted_html
                .as_ref()
                .is_some_and(|html| html.contains("arb-"))
        );
        assert!(highlighted_view.plain_text.is_none());

        let plain_text_view = open
            .engine
            .file_view("github.com/icepuma/nanite", "proto/kino/v1/probes.proto")
            .unwrap();
        assert!(plain_text_view.highlighted_html.is_none());
        assert!(
            plain_text_view
                .plain_text
                .as_deref()
                .is_some_and(|text| text.contains("message Probe"))
        );
        assert!(!plain_text_view.truncated);
        assert!(
            open.engine
                .file_view("github.com/icepuma/nanite", "../escape.rs")
                .is_err()
        );
    }
}
