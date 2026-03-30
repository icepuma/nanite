use crate::frontmatter::parse_frontmatter;
use crate::prompt::Prompter;
use anyhow::{Context, Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use tera::{Context as TeraContext, Function as TeraFunction, Tera};
use time::OffsetDateTime;

const MAX_CONTEXT_SNIPPET_BYTES: usize = 4 * 1024;
const MAX_WORKFLOW_SNIPPETS: usize = 8;
const MAX_INVENTORY_ENTRIES_PER_DIR: usize = 12;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TemplateMetadata {
    pub filename: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPlaceholder {
    pub name: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiPlaceholder {
    pub index: usize,
    pub prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadmeFragmentRole {
    Badges,
    Overview,
    QuickStart,
    Usage,
    Tests,
}

impl ReadmeFragmentRole {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Badges => "Badges",
            Self::Overview => "Overview",
            Self::QuickStart => "Quick Start",
            Self::Usage => "Usage",
            Self::Tests => "Tests",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiFragment {
    pub placeholder: AiPlaceholder,
    pub label: String,
    pub readme_role: Option<ReadmeFragmentRole>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateFragment {
    Literal(String),
    Text(TextPlaceholder),
    Expression(String),
    Ai(AiPlaceholder),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSnippet {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoContextFacts {
    pub repo_name: String,
    pub repo_shape: String,
    pub ci_workflows: Vec<String>,
    pub license_source: Option<String>,
    pub bootstrap_command: Option<String>,
    pub run_command: Option<String>,
    pub test_command: Option<String>,
    pub docs_present: Vec<String>,
    pub workspace_inventory: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBundle {
    pub facts: RepoContextFacts,
    pub summary_lines: Vec<String>,
    pub snippets: Vec<ContextSnippet>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiFragmentRequest {
    pub target_path: Utf8PathBuf,
    pub target_file: String,
    pub template_source_path: Utf8PathBuf,
    pub values: BTreeMap<String, String>,
    pub fragment_index: usize,
    pub display_label: String,
    pub fragment_prompt: String,
    pub active_sentinel: String,
    pub document: String,
    pub context: ContextBundle,
    pub readme_role: Option<ReadmeFragmentRole>,
    pub repair_notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadmeVerificationFinding {
    pub fragment_index: Option<usize>,
    pub fragment_label: Option<String>,
    pub repairable: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReadmeVerificationReport {
    pub findings: Vec<ReadmeVerificationFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateVariant {
    pub output_name: String,
    pub source_path: Utf8PathBuf,
    pub fragments: Vec<TemplateFragment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateBundle {
    pub name: String,
    pub source_path: Utf8PathBuf,
    pub templates: Vec<TemplateVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTemplate {
    pub output_name: String,
    pub source_path: Utf8PathBuf,
    pub fragments: Vec<TemplateFragment>,
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedBundle {
    pub name: String,
    pub source_path: Utf8PathBuf,
    pub templates: Vec<PreparedTemplate>,
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRepository {
    bundles: Vec<TemplateBundle>,
}

impl TemplateRepository {
    pub fn load(templates_root: &Utf8Path) -> Result<Self> {
        let entries = fs::read_dir(templates_root)
            .with_context(|| format!("failed to read {}", templates_root))?;
        let mut bundles = Vec::new();

        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let bundle_path = Utf8PathBuf::from_path_buf(entry.path())
                .map_err(|path| anyhow!("non-UTF-8 template path: {}", path.display()))?;
            let bundle_name = bundle_path
                .file_name()
                .ok_or_else(|| anyhow!("failed to determine bundle name for {}", bundle_path))?
                .to_owned();
            let bundle_entries = fs::read_dir(bundle_path.as_std_path())
                .with_context(|| format!("failed to read {}", bundle_path))?;
            let mut templates = Vec::new();

            for template_entry in bundle_entries {
                let template_entry = template_entry?;
                if !template_entry.file_type()?.is_file() {
                    continue;
                }

                let source_path = Utf8PathBuf::from_path_buf(template_entry.path())
                    .map_err(|path| anyhow!("non-UTF-8 template path: {}", path.display()))?;
                let raw = fs::read_to_string(source_path.as_std_path())
                    .with_context(|| format!("failed to read {}", source_path))?;
                let document = parse_frontmatter::<TemplateMetadata>(&raw)
                    .with_context(|| format!("failed to parse {}", source_path))?;
                let fragments = parse_template_fragments(&document.body, &source_path)?;

                templates.push(TemplateVariant {
                    output_name: document.metadata.filename,
                    source_path,
                    fragments,
                });
            }

            templates.sort_by(|left, right| {
                left.output_name
                    .cmp(&right.output_name)
                    .then_with(|| left.source_path.cmp(&right.source_path))
            });

            if templates.is_empty() {
                continue;
            }

            bundles.push(TemplateBundle {
                name: bundle_name,
                source_path: bundle_path,
                templates,
            });
        }

        bundles.sort_by(|left, right| left.name.cmp(&right.name));

        Ok(Self { bundles })
    }

    pub fn output_names(&self) -> Vec<String> {
        self.bundles
            .iter()
            .flat_map(|bundle| {
                bundle
                    .templates
                    .iter()
                    .map(|template| template.output_name.clone())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn selection_labels(&self) -> Vec<String> {
        self.bundles
            .iter()
            .map(TemplateBundle::selection_label)
            .collect()
    }

    pub fn bundle_by_selection_label(&self, label: &str) -> Result<&TemplateBundle> {
        self.bundles
            .iter()
            .find(|bundle| bundle.selection_label() == label)
            .ok_or_else(|| anyhow!("no template bundle found for selection `{label}`"))
    }
}

impl TemplateBundle {
    pub fn prepare(&self, prompter: &mut impl Prompter) -> Result<PreparedBundle> {
        self.prepare_with_values(BTreeMap::new(), prompter)
    }

    pub fn prepare_for_path(
        &self,
        cwd: &Utf8Path,
        prompter: &mut impl Prompter,
    ) -> Result<PreparedBundle> {
        self.prepare_with_seed_values(template_builtin_values(cwd), prompter)
    }

    pub fn prepare_with_seed_values(
        &self,
        seed_values: BTreeMap<String, String>,
        prompter: &mut impl Prompter,
    ) -> Result<PreparedBundle> {
        self.prepare_with_values(seed_values, prompter)
    }

    fn prepare_with_values(
        &self,
        mut values: BTreeMap<String, String>,
        prompter: &mut impl Prompter,
    ) -> Result<PreparedBundle> {
        let mut prompted_names = BTreeSet::new();

        for placeholder in self.text_placeholders() {
            if values.contains_key(&placeholder.name) {
                prompted_names.insert(placeholder.name.clone());
                continue;
            }
            if prompted_names.insert(placeholder.name.clone()) {
                let answer = prompter.prompt(&placeholder)?;
                values.insert(placeholder.name.clone(), answer);
            }
        }

        let templates = self
            .templates
            .iter()
            .map(|template| PreparedTemplate {
                output_name: template.output_name.clone(),
                source_path: template.source_path.clone(),
                fragments: template.fragments.clone(),
                values: values.clone(),
            })
            .collect();

        Ok(PreparedBundle {
            name: self.name.clone(),
            source_path: self.source_path.clone(),
            templates,
            values,
        })
    }

    pub fn selection_label(&self) -> String {
        let files = self
            .templates
            .iter()
            .map(TemplateVariant::selection_file_label)
            .collect::<Vec<_>>()
            .join(", ");
        format!("{} -> {}", self.name, files)
    }

    fn text_placeholders(&self) -> Vec<TextPlaceholder> {
        self.templates
            .iter()
            .flat_map(TemplateVariant::text_placeholders)
            .collect()
    }
}

fn extract_single_prepared_template(bundle: PreparedBundle) -> Result<PreparedTemplate> {
    bundle
        .templates
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("single bundle should contain one prepared template"))
}

impl TemplateVariant {
    fn selection_file_label(&self) -> String {
        let source_name = self
            .source_path
            .file_name()
            .unwrap_or(&self.output_name)
            .to_owned();
        if source_name == self.output_name {
            source_name
        } else {
            format!("{source_name} -> {}", self.output_name)
        }
    }

    pub fn prepare(&self, prompter: &mut impl Prompter) -> Result<PreparedTemplate> {
        TemplateBundle {
            name: "single".to_owned(),
            source_path: "/tmp".into(),
            templates: vec![self.clone()],
        }
        .prepare(prompter)
        .and_then(extract_single_prepared_template)
    }

    pub fn prepare_for_path(
        &self,
        cwd: &Utf8Path,
        prompter: &mut impl Prompter,
    ) -> Result<PreparedTemplate> {
        TemplateBundle {
            name: "single".to_owned(),
            source_path: "/tmp".into(),
            templates: vec![self.clone()],
        }
        .prepare_for_path(cwd, prompter)
        .and_then(extract_single_prepared_template)
    }

    pub fn prepare_with_seed_values(
        &self,
        seed_values: BTreeMap<String, String>,
        prompter: &mut impl Prompter,
    ) -> Result<PreparedTemplate> {
        TemplateBundle {
            name: "single".to_owned(),
            source_path: "/tmp".into(),
            templates: vec![self.clone()],
        }
        .prepare_with_seed_values(seed_values, prompter)
        .and_then(extract_single_prepared_template)
    }

    fn text_placeholders(&self) -> Vec<TextPlaceholder> {
        self.fragments
            .iter()
            .filter_map(|fragment| match fragment {
                TemplateFragment::Text(placeholder) => Some(placeholder.clone()),
                TemplateFragment::Literal(_)
                | TemplateFragment::Expression(_)
                | TemplateFragment::Ai(_) => None,
            })
            .collect()
    }
}

impl PreparedBundle {
    pub fn templates(&self) -> &[PreparedTemplate] {
        &self.templates
    }

    pub fn requires_agent(&self) -> bool {
        self.templates.iter().any(PreparedTemplate::requires_agent)
    }

    pub fn text_values(&self) -> &BTreeMap<String, String> {
        &self.values
    }
}

impl PreparedTemplate {
    pub fn target_path(&self, cwd: &Utf8Path) -> Utf8PathBuf {
        cwd.join(&self.output_name)
    }

    pub fn requires_agent(&self) -> bool {
        self.fragments
            .iter()
            .any(|fragment| matches!(fragment, TemplateFragment::Ai(_)))
    }

    pub fn ai_placeholders(&self) -> Vec<AiPlaceholder> {
        self.ai_fragments()
            .into_iter()
            .map(|fragment| fragment.placeholder)
            .collect()
    }

    pub fn ai_fragments(&self) -> Vec<AiFragment> {
        let readme_roles = if self.is_readme() {
            Some([
                ReadmeFragmentRole::Badges,
                ReadmeFragmentRole::Overview,
                ReadmeFragmentRole::QuickStart,
                ReadmeFragmentRole::Usage,
                ReadmeFragmentRole::Tests,
            ])
        } else {
            None
        };
        let mut fragments = Vec::new();
        let mut last_heading = None;
        let mut generic_index = 1;
        let mut readme_index = 0;

        for fragment in &self.fragments {
            match fragment {
                TemplateFragment::Literal(text) => {
                    if let Some(heading) = last_markdown_heading(text) {
                        last_heading = Some(heading);
                    }
                }
                TemplateFragment::Text(_) | TemplateFragment::Expression(_) => {}
                TemplateFragment::Ai(placeholder) => {
                    let readme_role = readme_roles
                        .as_ref()
                        .and_then(|roles| roles.get(readme_index))
                        .copied();
                    let label = readme_role.map_or_else(
                        || {
                            last_heading.clone().unwrap_or_else(|| {
                                let label = format!("Generate block {generic_index}");
                                generic_index += 1;
                                label
                            })
                        },
                        |role| {
                            readme_index += 1;
                            role.label().to_owned()
                        },
                    );
                    fragments.push(AiFragment {
                        placeholder: placeholder.clone(),
                        label,
                        readme_role,
                    });
                }
            }
        }

        fragments
    }

    pub fn render_final(&self, ai_values: &BTreeMap<usize, String>) -> Result<String> {
        let mut rendered = String::new();

        for fragment in &self.fragments {
            match fragment {
                TemplateFragment::Literal(text) => rendered.push_str(text),
                TemplateFragment::Text(placeholder) => {
                    rendered.push_str(&render_tera_expression(&placeholder.name, &self.values)?);
                }
                TemplateFragment::Expression(expression) => {
                    rendered.push_str(&render_tera_expression(expression, &self.values)?);
                }
                TemplateFragment::Ai(placeholder) => {
                    let value = ai_values.get(&placeholder.index).ok_or_else(|| {
                        anyhow!(
                            "missing AI replacement for fragment {} in {}",
                            placeholder.index,
                            self.source_path
                        )
                    })?;
                    rendered.push_str(value);
                }
            }
        }

        Ok(rendered)
    }

    pub fn write(&self, cwd: &Utf8Path, force: bool) -> Result<Utf8PathBuf> {
        if self.requires_agent() {
            bail!(
                "{} requires AI fragment resolution before it can be written",
                self.output_name
            );
        }

        let target = self.target_path(cwd);
        if target.exists() && !force {
            bail!("{target} already exists; rerun with --force to overwrite");
        }

        let rendered = self.render_final(&BTreeMap::new())?;
        fs::write(&target, rendered).with_context(|| format!("failed to write {}", target))?;
        Ok(target)
    }

    pub fn build_context_bundle(&self, cwd: &Utf8Path) -> ContextBundle {
        let repo_name = resolve_repo_name(&self.values, cwd);
        let facts = build_repo_context_facts(cwd, repo_name);
        ContextBundle {
            summary_lines: build_context_summary_lines(cwd, &facts, &self.values),
            snippets: collect_context_snippets(cwd),
            facts,
        }
    }

    pub fn build_ai_fragment_request(
        &self,
        cwd: &Utf8Path,
        context: &ContextBundle,
        fragment: &AiFragment,
        resolved_ai: &BTreeMap<usize, String>,
        repair_notes: &[String],
    ) -> Result<AiFragmentRequest> {
        Ok(AiFragmentRequest {
            target_path: self.target_path(cwd),
            target_file: self.output_name.clone(),
            template_source_path: self.source_path.clone(),
            values: self.values.clone(),
            fragment_index: fragment.placeholder.index,
            display_label: fragment.label.clone(),
            fragment_prompt: fragment.placeholder.prompt.clone(),
            active_sentinel: ai_sentinel(fragment.placeholder.index),
            document: self.render_document(fragment.placeholder.index, resolved_ai)?,
            context: context.clone(),
            readme_role: fragment.readme_role,
            repair_notes: repair_notes.to_vec(),
        })
    }

    pub fn is_readme(&self) -> bool {
        self.output_name == "README.md"
    }

    pub fn verify_readme(
        &self,
        rendered: &str,
        context: &ContextBundle,
        ai_values: &BTreeMap<usize, String>,
    ) -> ReadmeVerificationReport {
        if !self.is_readme() {
            return ReadmeVerificationReport::default();
        }

        let fragments = self.ai_fragments();
        let fragment_map = fragments
            .iter()
            .map(|fragment| (fragment.placeholder.index, fragment))
            .collect::<BTreeMap<_, _>>();
        let parsed = parse_readme_document(rendered);
        let mut findings = verify_readme_structure(rendered, &parsed);
        verify_badges(&mut findings, &parsed, context, &fragment_map);
        verify_fragment_values(&mut findings, &fragments, &fragment_map, context, ai_values);
        verify_static_readme_sections(&mut findings, self, ai_values, &parsed);
        ReadmeVerificationReport { findings }
    }

    fn expected_static_readme_sections(
        &self,
        ai_values: &BTreeMap<usize, String>,
    ) -> BTreeMap<String, String> {
        let skeleton = self
            .fragments
            .iter()
            .map(|fragment| match fragment {
                TemplateFragment::Literal(text) => text.clone(),
                TemplateFragment::Text(placeholder) => self
                    .values
                    .get(&placeholder.name)
                    .cloned()
                    .unwrap_or_default(),
                TemplateFragment::Expression(expression) => {
                    render_tera_expression(expression, &self.values).unwrap_or_default()
                }
                TemplateFragment::Ai(placeholder) => ai_values
                    .get(&placeholder.index)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect::<String>();
        let parsed = parse_readme_document(&skeleton);
        parsed
            .sections
            .into_iter()
            .filter(|section| matches!(section.title.as_str(), "Contributing" | "License"))
            .map(|section| (section.title, normalize_section_body(&section.body)))
            .collect()
    }

    fn render_document(
        &self,
        active_fragment: usize,
        resolved_ai: &BTreeMap<usize, String>,
    ) -> Result<String> {
        let mut rendered = String::new();

        for fragment in &self.fragments {
            match fragment {
                TemplateFragment::Literal(text) => rendered.push_str(text),
                TemplateFragment::Text(placeholder) => {
                    rendered.push_str(&render_tera_expression(&placeholder.name, &self.values)?);
                }
                TemplateFragment::Expression(expression) => {
                    rendered.push_str(&render_tera_expression(expression, &self.values)?);
                }
                TemplateFragment::Ai(placeholder) => {
                    if placeholder.index == active_fragment {
                        rendered.push_str(&ai_sentinel(placeholder.index));
                    } else if let Some(value) = resolved_ai.get(&placeholder.index) {
                        rendered.push_str(value);
                    } else {
                        write!(rendered, "{{{{ai:{}}}}}", placeholder.prompt)
                            .expect("writing to String should not fail");
                    }
                }
            }
        }

        Ok(rendered)
    }
}

fn resolve_repo_name(values: &BTreeMap<String, String>, cwd: &Utf8Path) -> String {
    values
        .get("repo_name")
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .or_else(|| {
            values
                .get("project_name")
                .filter(|value| !value.trim().is_empty())
                .cloned()
        })
        .unwrap_or_else(|| cwd.file_name().unwrap_or("project").to_owned())
}

fn render_optional_summary(value: Option<&str>) -> String {
    value.unwrap_or("(none)").to_owned()
}

fn render_list_summary(values: &[String], separator: &str) -> String {
    if values.is_empty() {
        "(none)".to_owned()
    } else {
        values.join(separator)
    }
}

fn build_context_summary_lines(
    cwd: &Utf8Path,
    facts: &RepoContextFacts,
    values: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut summary_lines = vec![
        format!("- Repo name: {}", facts.repo_name),
        format!("- Current directory: {cwd}"),
        format!("- Repo shape: {}", facts.repo_shape),
        format!(
            "- Verified CI workflows: {}",
            render_list_summary(&facts.ci_workflows, ", ")
        ),
        format!(
            "- Verified license source: {}",
            render_optional_summary(facts.license_source.as_deref())
        ),
        format!(
            "- Verified setup command: {}",
            render_optional_summary(facts.bootstrap_command.as_deref())
        ),
        format!(
            "- Verified run command: {}",
            render_optional_summary(facts.run_command.as_deref())
        ),
        format!(
            "- Verified test command: {}",
            render_optional_summary(facts.test_command.as_deref())
        ),
        format!(
            "- Docs present: {}",
            render_list_summary(&facts.docs_present, ", ")
        ),
        format!(
            "- Workspace inventory: {}",
            render_list_summary(&facts.workspace_inventory, "; ")
        ),
    ];
    for (name, value) in values {
        summary_lines.push(format!("- {name}: {}", render_summary_value(value)));
    }
    summary_lines
}

fn collect_context_snippets(cwd: &Utf8Path) -> Vec<ContextSnippet> {
    let mut snippets = [
        "Cargo.toml",
        "package.json",
        "pnpm-workspace.yaml",
        "pyproject.toml",
        "go.mod",
        "go.work",
        "justfile",
        "Makefile",
        "Dockerfile",
        "compose.yaml",
        "docker-compose.yml",
        "README.md",
        "CONTRIBUTING.md",
        "LICENSE",
        "docs/README.md",
    ]
    .into_iter()
    .filter_map(|candidate| read_context_snippet(&cwd.join(candidate), candidate))
    .collect::<Vec<_>>();

    if let Some(workflow_snippets) = collect_workflow_snippets(cwd) {
        snippets.extend(workflow_snippets);
    }

    if let Some(inventory) = collect_workspace_inventory(cwd) {
        snippets.push(ContextSnippet {
            path: "workspace-inventory.txt".to_owned(),
            content: inventory,
        });
    }

    snippets
}

fn verify_readme_structure(
    rendered: &str,
    parsed: &ParsedReadme,
) -> Vec<ReadmeVerificationFinding> {
    let mut findings = Vec::new();

    if rendered.contains("{{") || rendered.contains("}}") || rendered.contains("[[NANITE_") {
        findings.push(ReadmeVerificationFinding {
            fragment_index: None,
            fragment_label: None,
            repairable: false,
            message: "README still contains unresolved template placeholders".to_owned(),
        });
    }
    if !parsed.first_non_empty_is_h1 {
        findings.push(ReadmeVerificationFinding {
            fragment_index: None,
            fragment_label: None,
            repairable: false,
            message: "README must start with exactly one H1 title".to_owned(),
        });
    }
    if parsed.h1_count != 1 {
        findings.push(ReadmeVerificationFinding {
            fragment_index: None,
            fragment_label: None,
            repairable: false,
            message: "README must contain exactly one H1 heading".to_owned(),
        });
    }

    let expected_sections = ["Quick Start", "Usage", "Tests", "Contributing", "License"];
    let actual_sections = parsed
        .sections
        .iter()
        .map(|section| section.title.as_str())
        .collect::<Vec<_>>();
    if actual_sections != expected_sections {
        findings.push(ReadmeVerificationFinding {
            fragment_index: None,
            fragment_label: None,
            repairable: false,
            message: format!(
                "README top-level sections must be exactly: {}",
                expected_sections.join(", ")
            ),
        });
    }

    findings
}

fn verify_badges(
    findings: &mut Vec<ReadmeVerificationFinding>,
    parsed: &ParsedReadme,
    context: &ContextBundle,
    fragment_map: &BTreeMap<usize, &AiFragment>,
) {
    let badge_lines = parsed
        .preamble
        .iter()
        .filter(|line| looks_like_badge_line(line))
        .count();
    if badge_lines > 1 {
        findings.push(role_finding(
            ReadmeFragmentRole::Badges,
            fragment_map,
            true,
            "badge area must contain at most one badge line",
        ));
    }
    if badge_lines > 0
        && context.facts.ci_workflows.is_empty()
        && context.facts.license_source.is_none()
    {
        findings.push(role_finding(
            ReadmeFragmentRole::Badges,
            fragment_map,
            true,
            "badges require verified CI or license facts",
        ));
    }
}

fn verify_fragment_values(
    findings: &mut Vec<ReadmeVerificationFinding>,
    fragments: &[AiFragment],
    fragment_map: &BTreeMap<usize, &AiFragment>,
    context: &ContextBundle,
    ai_values: &BTreeMap<usize, String>,
) {
    for fragment in fragments {
        let Some(value) = ai_values.get(&fragment.placeholder.index) else {
            continue;
        };
        verify_fragment_value(findings, fragment, fragment_map, context, value);
    }
}

fn verify_fragment_value(
    findings: &mut Vec<ReadmeVerificationFinding>,
    fragment: &AiFragment,
    fragment_map: &BTreeMap<usize, &AiFragment>,
    context: &ContextBundle,
    value: &str,
) {
    match fragment.readme_role {
        Some(ReadmeFragmentRole::Badges) => verify_badge_fragment(findings, fragment_map, value),
        Some(ReadmeFragmentRole::Overview) => {
            verify_overview_fragment(findings, fragment_map, value);
        }
        Some(ReadmeFragmentRole::QuickStart) => verify_bullet_fragment(
            findings,
            fragment_map,
            ReadmeFragmentRole::QuickStart,
            value,
            2,
            3,
        ),
        Some(ReadmeFragmentRole::Usage) => verify_bullet_fragment(
            findings,
            fragment_map,
            ReadmeFragmentRole::Usage,
            value,
            2,
            3,
        ),
        Some(ReadmeFragmentRole::Tests) => {
            verify_tests_fragment(findings, fragment_map, context, value);
        }
        None => {}
    }

    if value.lines().any(|line| line.starts_with('#')) {
        let role = fragment.readme_role.unwrap_or(ReadmeFragmentRole::Overview);
        findings.push(role_finding(
            role,
            fragment_map,
            true,
            "AI fragments must not introduce headings",
        ));
    }
}

fn verify_badge_fragment(
    findings: &mut Vec<ReadmeVerificationFinding>,
    fragment_map: &BTreeMap<usize, &AiFragment>,
    value: &str,
) {
    if value.lines().filter(|line| !line.trim().is_empty()).count() > 1 {
        findings.push(role_finding(
            ReadmeFragmentRole::Badges,
            fragment_map,
            true,
            "badges must be a single markdown line or blank",
        ));
    }
}

fn verify_overview_fragment(
    findings: &mut Vec<ReadmeVerificationFinding>,
    fragment_map: &BTreeMap<usize, &AiFragment>,
    value: &str,
) {
    if value.lines().any(|line| line.trim_start().starts_with('-')) {
        findings.push(role_finding(
            ReadmeFragmentRole::Overview,
            fragment_map,
            true,
            "overview must be prose, not bullet points",
        ));
    }
    if !(2..=3).contains(&count_sentences(value)) {
        findings.push(role_finding(
            ReadmeFragmentRole::Overview,
            fragment_map,
            true,
            "overview must be 2 or 3 sentences",
        ));
    }
}

fn verify_tests_fragment(
    findings: &mut Vec<ReadmeVerificationFinding>,
    fragment_map: &BTreeMap<usize, &AiFragment>,
    context: &ContextBundle,
    value: &str,
) {
    verify_bullet_fragment(
        findings,
        fragment_map,
        ReadmeFragmentRole::Tests,
        value,
        1,
        3,
    );
    if context.facts.test_command.is_none()
        && !value
            .to_ascii_lowercase()
            .contains("no verified test command was found")
    {
        findings.push(role_finding(
            ReadmeFragmentRole::Tests,
            fragment_map,
            true,
            "tests must use the explicit fallback when no verified test command exists",
        ));
    }
}

fn verify_static_readme_sections(
    findings: &mut Vec<ReadmeVerificationFinding>,
    template: &PreparedTemplate,
    ai_values: &BTreeMap<usize, String>,
    parsed: &ParsedReadme,
) {
    let expected_sections = template.expected_static_readme_sections(ai_values);
    for title in ["Contributing", "License"] {
        let expected = expected_sections.get(title).cloned();
        let actual = parsed
            .sections
            .iter()
            .find(|section| section.title == title)
            .map(|section| normalize_section_body(&section.body));
        if expected != actual {
            findings.push(ReadmeVerificationFinding {
                fragment_index: None,
                fragment_label: Some(title.to_owned()),
                repairable: false,
                message: format!("{title} must stay identical to the template"),
            });
        }
    }
}

fn parse_template_fragments(body: &str, source_path: &Utf8Path) -> Result<Vec<TemplateFragment>> {
    let mut fragments = Vec::new();
    let mut cursor = 0;
    let mut ai_index = 0;

    while let Some(start_offset) = body[cursor..].find("{{") {
        let start = cursor + start_offset;
        if start > cursor {
            fragments.push(TemplateFragment::Literal(body[cursor..start].to_owned()));
        }

        let content_start = start + 2;
        let end_offset = body[content_start..]
            .find("}}")
            .ok_or_else(|| anyhow!("unterminated template placeholder in {}", source_path))?;
        let end = content_start + end_offset;
        let raw_inner = &body[content_start..end];
        if raw_inner.contains('\n') {
            bail!(
                "multiline template placeholders are not supported in {}",
                source_path
            );
        }

        let inner = raw_inner.trim();
        if inner.is_empty() {
            bail!("empty template placeholder in {}", source_path);
        }

        if let Some(prompt) = inner.strip_prefix("ai:") {
            let prompt = prompt.trim();
            if prompt.is_empty() {
                bail!("empty AI placeholder prompt in {}", source_path);
            }
            fragments.push(TemplateFragment::Ai(AiPlaceholder {
                index: ai_index,
                prompt: prompt.to_owned(),
            }));
            ai_index += 1;
        } else {
            let name = inner.trim();
            if is_valid_identifier(name) {
                fragments.push(TemplateFragment::Text(TextPlaceholder {
                    name: name.to_owned(),
                    prompt: humanize_identifier(name),
                }));
            } else if let Some(expression) = supported_tera_expression(name) {
                fragments.push(TemplateFragment::Expression(expression));
            } else {
                bail!("invalid placeholder `{name}` in {}", source_path);
            }
        }

        cursor = end + 2;
    }

    if cursor < body.len() {
        fragments.push(TemplateFragment::Literal(body[cursor..].to_owned()));
    }

    Ok(fragments)
}

fn is_valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }

    chars.all(|char| char.is_ascii_alphanumeric() || char == '_')
}

fn supported_tera_expression(value: &str) -> Option<String> {
    let compact = value
        .chars()
        .filter(|char| !char.is_whitespace())
        .collect::<String>();
    match compact.as_str() {
        "current_year()" => Some("current_year()".to_owned()),
        "repo_name()" => Some("repo_name()".to_owned()),
        _ => None,
    }
}

fn humanize_identifier(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            chars.next().map_or_else(String::new, |first| {
                let mut rendered = first.to_ascii_uppercase().to_string();
                rendered.push_str(chars.as_str());
                rendered
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_tera_expression(expression: &str, values: &BTreeMap<String, String>) -> Result<String> {
    validate_tera_expression(expression)?;
    let mut tera = build_template_engine(values);
    let rendered = tera
        .render_str(&format!("{{{{ {expression} }}}}"), &tera_context(values))
        .with_context(|| format!("failed to render `{expression}`"))?;
    Ok(rendered)
}

fn validate_tera_expression(expression: &str) -> Result<()> {
    let mut tera = build_template_engine(&BTreeMap::new());
    tera.add_raw_template("expression", &format!("{{{{ {expression} }}}}"))
        .with_context(|| format!("invalid template expression `{expression}`"))?;
    Ok(())
}

fn build_template_engine(values: &BTreeMap<String, String>) -> Tera {
    let mut tera = Tera::default();
    tera.autoescape_on(Vec::new());
    tera.register_function("current_year", CurrentYearFunction);
    tera.register_function(
        "repo_name",
        RepoNameFunction {
            repo_name: values
                .get("repo_name")
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| "project".to_owned()),
        },
    );
    tera
}

fn tera_context(values: &BTreeMap<String, String>) -> TeraContext {
    let mut context = TeraContext::new();
    for (name, value) in values {
        context.insert(name, value);
    }
    context
}

#[derive(Clone, Copy)]
struct CurrentYearFunction;

impl TeraFunction for CurrentYearFunction {
    fn call(
        &self,
        _args: &std::collections::HashMap<String, JsonValue>,
    ) -> tera::Result<JsonValue> {
        Ok(JsonValue::from(i64::from(OffsetDateTime::now_utc().year())))
    }
}

#[derive(Clone)]
struct RepoNameFunction {
    repo_name: String,
}

impl TeraFunction for RepoNameFunction {
    fn call(
        &self,
        _args: &std::collections::HashMap<String, JsonValue>,
    ) -> tera::Result<JsonValue> {
        Ok(JsonValue::from(self.repo_name.clone()))
    }
}

fn template_builtin_values(cwd: &Utf8Path) -> BTreeMap<String, String> {
    BTreeMap::from([(
        "repo_name".to_owned(),
        cwd.file_name().unwrap_or("project").to_owned(),
    )])
}

fn ai_sentinel(index: usize) -> String {
    format!("[[NANITE_FRAGMENT_{}]]", index + 1)
}

fn render_summary_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "(blank)".to_owned();
    }
    trimmed.to_owned()
}

fn detect_repo_shape(cwd: &Utf8Path) -> &'static str {
    if cwd.join("pnpm-workspace.yaml").exists()
        || cwd.join("go.work").exists()
        || cwd.join("turbo.json").exists()
        || cwd.join("nx.json").exists()
        || directory_has_entries(cwd.join("apps"))
        || directory_has_entries(cwd.join("packages"))
        || directory_has_entries(cwd.join("crates"))
    {
        return "monorepo_root";
    }

    let cargo = read_text(&cwd.join("Cargo.toml"));
    if cargo
        .as_deref()
        .is_some_and(|contents| contents.contains("[workspace]"))
    {
        return "monorepo_root";
    }

    let package = read_text(&cwd.join("package.json"));
    if package
        .as_deref()
        .is_some_and(|contents| contents.contains("\"workspaces\""))
    {
        return "monorepo_root";
    }

    "single_project"
}

fn build_repo_context_facts(cwd: &Utf8Path, repo_name: String) -> RepoContextFacts {
    let repo_shape = detect_repo_shape(cwd).to_owned();
    let ci_workflows = collect_workflow_names(cwd);
    let package_json = read_json(&cwd.join("package.json"));
    let package_manager = detect_package_manager(cwd, package_json.as_ref());
    let bootstrap_command = detect_bootstrap_command(cwd, package_json.as_ref(), package_manager);
    let run_command = detect_run_command(cwd, package_json.as_ref(), package_manager);
    let test_command = detect_test_command(cwd, package_json.as_ref(), package_manager);

    RepoContextFacts {
        repo_name,
        repo_shape,
        ci_workflows,
        license_source: detect_license_source(cwd),
        bootstrap_command,
        run_command,
        test_command,
        docs_present: collect_docs_present(cwd),
        workspace_inventory: collect_workspace_inventory_lines(cwd),
    }
}

fn directory_has_entries(path: Utf8PathBuf) -> bool {
    let Ok(read_dir) = fs::read_dir(path) else {
        return false;
    };
    read_dir.into_iter().flatten().next().is_some()
}

fn read_context_snippet(path: &Utf8Path, display_path: &str) -> Option<ContextSnippet> {
    if !path.is_file() {
        return None;
    }

    read_text(path).map(|content| ContextSnippet {
        path: display_path.to_owned(),
        content: truncate_for_context(&content),
    })
}

fn collect_workflow_snippets(cwd: &Utf8Path) -> Option<Vec<ContextSnippet>> {
    let workflows_root = cwd.join(".github/workflows");
    let entries = fs::read_dir(workflows_root.as_std_path()).ok()?;
    let mut files = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            if !path.is_file() {
                return None;
            }
            Some(path)
        })
        .collect::<Vec<_>>();
    files.sort();

    let snippets = files
        .into_iter()
        .take(MAX_WORKFLOW_SNIPPETS)
        .filter_map(|path| {
            let file_name = path.file_name()?.to_owned();
            read_context_snippet(&path, &format!(".github/workflows/{file_name}"))
        })
        .collect::<Vec<_>>();

    if snippets.is_empty() {
        None
    } else {
        Some(snippets)
    }
}

fn collect_workflow_names(cwd: &Utf8Path) -> Vec<String> {
    let workflows_root = cwd.join(".github/workflows");
    let Ok(entries) = fs::read_dir(workflows_root.as_std_path()) else {
        return Vec::new();
    };
    let mut names = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            if !path.is_file() {
                return None;
            }
            path.file_name().map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn collect_workspace_inventory(cwd: &Utf8Path) -> Option<String> {
    let mut sections = Vec::new();
    for (dir_name, label) in [
        ("apps", "apps"),
        ("packages", "packages"),
        ("crates", "crates"),
    ] {
        let root = cwd.join(dir_name);
        let Ok(entries) = fs::read_dir(root.as_std_path()) else {
            continue;
        };

        let mut names = entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if !entry.file_type().ok()?.is_dir() {
                    return None;
                }
                entry.file_name().into_string().ok()
            })
            .collect::<Vec<_>>();
        names.sort();
        names.truncate(MAX_INVENTORY_ENTRIES_PER_DIR);
        if names.is_empty() {
            continue;
        }

        let mut content = format!("[{label}]\n");
        for name in names {
            writeln!(content, "- {name}").expect("writing to String should not fail");
        }
        sections.push(content.trim_end().to_owned());
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

fn collect_workspace_inventory_lines(cwd: &Utf8Path) -> Vec<String> {
    collect_workspace_inventory(cwd)
        .map(|content| {
            content
                .split("\n\n")
                .map(|section| section.replace('\n', "; "))
                .collect()
        })
        .unwrap_or_default()
}

fn read_text(path: &Utf8Path) -> Option<String> {
    fs::read_to_string(path.as_std_path()).ok()
}

fn read_json(path: &Utf8Path) -> Option<JsonValue> {
    read_text(path).and_then(|text| serde_json::from_str(&text).ok())
}

fn collect_docs_present(cwd: &Utf8Path) -> Vec<String> {
    ["README.md", "CONTRIBUTING.md", "LICENSE", "docs/README.md"]
        .into_iter()
        .filter(|candidate| cwd.join(candidate).exists())
        .map(ToOwned::to_owned)
        .collect()
}

fn detect_license_source(cwd: &Utf8Path) -> Option<String> {
    for candidate in ["LICENSE", "LICENSE.md", "COPYING"] {
        if cwd.join(candidate).is_file() {
            return Some(candidate.to_owned());
        }
    }

    let cargo = read_text(&cwd.join("Cargo.toml"));
    if let Some(line) = cargo
        .as_deref()
        .and_then(|text| find_toml_string_value(text, "license"))
    {
        return Some(format!("Cargo.toml ({line})"));
    }

    let package = read_json(&cwd.join("package.json"));
    if let Some(license) = package
        .as_ref()
        .and_then(|json| json.get("license"))
        .and_then(JsonValue::as_str)
    {
        return Some(format!("package.json ({license})"));
    }

    None
}

fn detect_package_manager(
    cwd: &Utf8Path,
    package_json: Option<&JsonValue>,
) -> Option<&'static str> {
    if cwd.join("pnpm-workspace.yaml").exists() || cwd.join("pnpm-lock.yaml").exists() {
        return Some("pnpm");
    }
    if cwd.join("bun.lock").exists() || cwd.join("bun.lockb").exists() {
        return Some("bun");
    }
    if cwd.join("yarn.lock").exists() {
        return Some("yarn");
    }
    if let Some(manager) = package_json
        .and_then(|json| json.get("packageManager"))
        .and_then(JsonValue::as_str)
    {
        if manager.starts_with("pnpm@") {
            return Some("pnpm");
        }
        if manager.starts_with("yarn@") {
            return Some("yarn");
        }
        if manager.starts_with("bun@") {
            return Some("bun");
        }
        if manager.starts_with("npm@") {
            return Some("npm");
        }
    }
    if package_json.is_some() {
        return Some("npm");
    }

    None
}

fn detect_bootstrap_command(
    cwd: &Utf8Path,
    package_json: Option<&JsonValue>,
    package_manager: Option<&'static str>,
) -> Option<String> {
    if let Some(manager) = package_manager {
        return Some(
            match manager {
                "pnpm" => "pnpm install",
                "bun" => "bun install",
                "yarn" => "yarn install",
                _ => "npm install",
            }
            .to_owned(),
        );
    }
    if cwd.join("Cargo.toml").is_file() {
        return Some("cargo build".to_owned());
    }
    if cwd.join("pyproject.toml").is_file() {
        return Some("python -m pip install -e .".to_owned());
    }
    if cwd.join("go.mod").is_file() || cwd.join("go.work").is_file() {
        return Some("go build ./...".to_owned());
    }
    if package_json.is_some() {
        return Some("npm install".to_owned());
    }

    None
}

fn detect_run_command(
    cwd: &Utf8Path,
    package_json: Option<&JsonValue>,
    package_manager: Option<&'static str>,
) -> Option<String> {
    if let Some(target) = detect_named_target(&cwd.join("justfile"), &["dev", "run", "start"]) {
        return Some(format!("just {target}"));
    }
    if let Some(target) =
        detect_named_target(&cwd.join("Makefile"), &["dev", "run", "start", "serve"])
    {
        return Some(format!("make {target}"));
    }
    if let Some(scripts) = package_json.and_then(package_scripts) {
        if scripts.contains_key("dev") {
            return Some(package_manager_run_command(package_manager, "dev"));
        }
        if scripts.contains_key("start") {
            return Some(package_manager_run_command(package_manager, "start"));
        }
    }
    if cwd.join("Cargo.toml").is_file() {
        return Some("cargo run".to_owned());
    }

    None
}

fn detect_test_command(
    cwd: &Utf8Path,
    package_json: Option<&JsonValue>,
    package_manager: Option<&'static str>,
) -> Option<String> {
    if let Some(target) = detect_named_target(&cwd.join("justfile"), &["test", "check"]) {
        return Some(format!("just {target}"));
    }
    if let Some(target) = detect_named_target(&cwd.join("Makefile"), &["test", "check"]) {
        return Some(format!("make {target}"));
    }
    if let Some(scripts) = package_json.and_then(package_scripts) {
        if scripts.contains_key("test") {
            return Some(package_manager_run_command(package_manager, "test"));
        }
        if scripts.contains_key("check") {
            return Some(package_manager_run_command(package_manager, "check"));
        }
    }
    if cwd.join("Cargo.toml").is_file() {
        return Some("cargo test -q".to_owned());
    }
    if cwd.join("pyproject.toml").is_file() {
        return Some("pytest".to_owned());
    }
    if cwd.join("go.mod").is_file() || cwd.join("go.work").is_file() {
        return Some("go test ./...".to_owned());
    }

    None
}

fn package_scripts(package_json: &JsonValue) -> Option<&serde_json::Map<String, JsonValue>> {
    package_json.get("scripts")?.as_object()
}

fn package_manager_run_command(package_manager: Option<&str>, script: &str) -> String {
    match package_manager.unwrap_or("npm") {
        "pnpm" => format!("pnpm {script}"),
        "yarn" => format!("yarn {script}"),
        "bun" => format!("bun run {script}"),
        _ => format!("npm run {script}"),
    }
}

fn detect_named_target(path: &Utf8Path, names: &[&str]) -> Option<String> {
    let text = read_text(path)?;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some((name, _)) = trimmed.split_once(':') {
            let candidate = name.trim();
            if names.contains(&candidate) {
                return Some(candidate.to_owned());
            }
        }
    }
    None
}

fn find_toml_string_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let prefix = format!("{key} = ");
        if !trimmed.starts_with(&prefix) {
            return None;
        }
        let value = trimmed[prefix.len()..].trim();
        Some(value.trim_matches('"'))
    })
}

fn last_markdown_heading(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with('#') {
                return None;
            }
            let title = trimmed.trim_start_matches('#').trim();
            if title.is_empty() {
                return None;
            }
            Some(title.to_owned())
        })
        .next_back()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedReadme {
    first_non_empty_is_h1: bool,
    h1_count: usize,
    preamble: Vec<String>,
    sections: Vec<ReadmeSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadmeSection {
    title: String,
    body: Vec<String>,
}

fn parse_readme_document(markdown: &str) -> ParsedReadme {
    let lines = markdown.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let first_non_empty = lines.iter().find(|line| !line.trim().is_empty());
    let first_non_empty_is_h1 =
        first_non_empty.is_some_and(|line| line.trim_start().starts_with("# "));
    let h1_count = lines
        .iter()
        .filter(|line| line.trim_start().starts_with("# "))
        .count();

    let mut seen_first_h1 = false;
    let mut preamble = Vec::new();
    let mut sections = Vec::new();
    let mut current_section: Option<ReadmeSection> = None;

    for line in lines {
        let trimmed = line.trim_start();
        if trimmed.starts_with("# ") && !seen_first_h1 {
            seen_first_h1 = true;
            continue;
        }
        if !seen_first_h1 {
            continue;
        }
        if let Some(title) = trimmed.strip_prefix("## ") {
            if let Some(section) = current_section.take() {
                sections.push(section);
            }
            current_section = Some(ReadmeSection {
                title: title.trim().to_owned(),
                body: Vec::new(),
            });
            continue;
        }
        if let Some(section) = current_section.as_mut() {
            section.body.push(line);
        } else {
            preamble.push(line);
        }
    }

    if let Some(section) = current_section {
        sections.push(section);
    }

    ParsedReadme {
        first_non_empty_is_h1,
        h1_count,
        preamble,
        sections,
    }
}

fn normalize_section_body(lines: &[String]) -> String {
    lines.join("\n").trim().to_owned()
}

fn looks_like_badge_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("[![") || trimmed.starts_with("![")
}

fn count_sentences(text: &str) -> usize {
    text.split_terminator(['.', '!', '?'])
        .filter(|segment| !segment.trim().is_empty())
        .count()
}

fn bullet_lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn verify_bullet_fragment(
    findings: &mut Vec<ReadmeVerificationFinding>,
    fragment_map: &BTreeMap<usize, &AiFragment>,
    role: ReadmeFragmentRole,
    value: &str,
    min: usize,
    max: usize,
) {
    let bullets = bullet_lines(value);
    if bullets.iter().any(|line| !line.starts_with("- ")) {
        findings.push(role_finding(
            role,
            fragment_map,
            true,
            &format!("{} must use markdown bullet lines only", role.label()),
        ));
        return;
    }
    if !(min..=max).contains(&bullets.len()) {
        findings.push(role_finding(
            role,
            fragment_map,
            true,
            &format!("{} must contain {min} to {max} bullet lines", role.label()),
        ));
    }
}

fn role_finding(
    role: ReadmeFragmentRole,
    fragment_map: &BTreeMap<usize, &AiFragment>,
    repairable: bool,
    message: &str,
) -> ReadmeVerificationFinding {
    let fragment = fragment_map
        .values()
        .find(|fragment| fragment.readme_role == Some(role));
    ReadmeVerificationFinding {
        fragment_index: fragment.map(|fragment| fragment.placeholder.index),
        fragment_label: fragment.map(|fragment| fragment.label.clone()),
        repairable,
        message: message.to_owned(),
    }
}

impl ReadmeVerificationReport {
    pub fn is_valid(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn repairable_fragment_indexes(&self) -> Vec<usize> {
        self.findings
            .iter()
            .filter(|finding| finding.repairable)
            .filter_map(|finding| finding.fragment_index)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn has_non_repairable_findings(&self) -> bool {
        self.findings.iter().any(|finding| !finding.repairable)
    }

    pub fn render_messages(&self) -> Vec<String> {
        self.findings
            .iter()
            .map(|finding| {
                finding.fragment_label.as_ref().map_or_else(
                    || finding.message.clone(),
                    |label| format!("{label}: {}", finding.message),
                )
            })
            .collect()
    }
}

fn truncate_for_context(contents: &str) -> String {
    if contents.len() <= MAX_CONTEXT_SNIPPET_BYTES {
        return contents.to_owned();
    }

    let mut end = MAX_CONTEXT_SNIPPET_BYTES;
    while !contents.is_char_boundary(end) {
        end -= 1;
    }

    let mut truncated = contents[..end].to_owned();
    truncated.push_str("\n… [truncated]");
    truncated
}

#[cfg(test)]
mod tests {
    use super::{
        AiPlaceholder, PreparedTemplate, TemplateBundle, TemplateFragment, TemplateMetadata,
        TemplateRepository, TemplateVariant, TextPlaceholder, ai_sentinel,
        parse_template_fragments,
    };
    use crate::prompt::StaticPrompter;
    use camino::Utf8Path;
    use std::collections::BTreeMap;
    use std::fs;
    use time::OffsetDateTime;

    fn template(filename: &str, body: &str) -> TemplateVariant {
        TemplateVariant {
            output_name: filename.to_owned(),
            source_path: format!("/tmp/{filename}").into(),
            fragments: parse_template_fragments(body, Utf8Path::new("/tmp/template.md")).unwrap(),
        }
    }

    #[test]
    fn parses_filename_frontmatter_and_lists_templates() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("default")).unwrap();
        fs::write(
            root.join("default/README.md"),
            "---\nfilename: README.md\n---\n# {{project_name}}\n",
        )
        .unwrap();
        fs::write(
            root.join("default/LICENSE"),
            "---\nfilename: LICENSE\n---\nMIT\n",
        )
        .unwrap();

        let repository = TemplateRepository::load(&root).unwrap();

        assert_eq!(repository.output_names(), vec!["LICENSE", "README.md"]);
        assert_eq!(
            repository.selection_labels(),
            vec!["default -> LICENSE, README.md".to_owned()]
        );
        assert_eq!(
            repository
                .bundle_by_selection_label("default -> LICENSE, README.md")
                .unwrap()
                .templates
                .len(),
            2
        );
    }

    #[test]
    fn rejects_missing_frontmatter_filename() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("default")).unwrap();
        fs::write(root.join("default/README.md"), "---\n---\n# test\n").unwrap();

        let error = TemplateRepository::load(&root).unwrap_err().to_string();

        assert!(error.contains("failed to parse"));
    }

    #[test]
    fn parses_literal_text_and_ai_fragments_in_order() {
        let fragments = parse_template_fragments(
            "# {{project_name}}\n\n{{ai:write summary}}\n",
            Utf8Path::new("/tmp/template.md"),
        )
        .unwrap();

        assert_eq!(
            fragments,
            vec![
                TemplateFragment::Literal("# ".to_owned()),
                TemplateFragment::Text(TextPlaceholder {
                    name: "project_name".to_owned(),
                    prompt: "Project Name".to_owned(),
                }),
                TemplateFragment::Literal("\n\n".to_owned()),
                TemplateFragment::Ai(AiPlaceholder {
                    index: 0,
                    prompt: "write summary".to_owned(),
                }),
                TemplateFragment::Literal("\n".to_owned()),
            ]
        );
    }

    #[test]
    fn parses_supported_tera_function_expressions() {
        let fragments = parse_template_fragments(
            "Copyright (c) {{ current_year() }} {{ repo_name() }}\n",
            Utf8Path::new("/tmp/template.md"),
        )
        .unwrap();

        assert_eq!(
            fragments,
            vec![
                TemplateFragment::Literal("Copyright (c) ".to_owned()),
                TemplateFragment::Expression("current_year()".to_owned()),
                TemplateFragment::Literal(" ".to_owned()),
                TemplateFragment::Expression("repo_name()".to_owned()),
                TemplateFragment::Literal("\n".to_owned()),
            ]
        );
    }

    #[test]
    fn rejects_invalid_identifier_placeholders() {
        let error = parse_template_fragments("{{project-name}}", Utf8Path::new("/tmp/template.md"))
            .unwrap_err()
            .to_string();

        assert!(error.contains("invalid placeholder"));
    }

    #[test]
    fn rejects_malformed_ai_placeholders() {
        let error = parse_template_fragments("{{ai:}}", Utf8Path::new("/tmp/template.md"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("empty AI placeholder prompt"));

        let error =
            parse_template_fragments("{{ai:hello\nworld}}", Utf8Path::new("/tmp/template.md"))
                .unwrap_err()
                .to_string();
        assert!(error.contains("multiline template placeholders"));
    }

    #[test]
    fn repeated_text_placeholders_prompt_once_and_reuse() {
        let mut prompter = StaticPrompter::new(BTreeMap::from([(
            "project_name".to_owned(),
            "nanite".to_owned(),
        )]));
        let prepared = template(
            "README.md",
            "# {{project_name}}\n\n{{project_name}} keeps things tidy.\n",
        )
        .prepare(&mut prompter)
        .unwrap();

        assert_eq!(prepared.values["project_name"], "nanite");
        assert_eq!(
            prepared.render_final(&BTreeMap::new()).unwrap(),
            "# nanite\n\nnanite keeps things tidy.\n"
        );
    }

    #[test]
    fn renders_current_year_without_prompting() {
        let mut prompter = StaticPrompter::new(BTreeMap::from([(
            "author".to_owned(),
            "Jane Doe".to_owned(),
        )]));
        let prepared = template(
            "LICENSE",
            "Copyright (c) {{ current_year() }} {{ author }}\n",
        )
        .prepare(&mut prompter)
        .unwrap();

        let rendered = prepared.render_final(&BTreeMap::new()).unwrap();
        let current_year = OffsetDateTime::now_utc().year();

        assert_eq!(rendered, format!("Copyright (c) {current_year} Jane Doe\n"));
    }

    #[test]
    fn renders_repo_name_from_current_path_without_prompting() {
        let mut prompter = StaticPrompter::new(BTreeMap::new());
        let prepared = template("README.md", "# {{ repo_name() }}\n")
            .prepare_for_path(Utf8Path::new("/tmp/nanite"), &mut prompter)
            .unwrap();

        let rendered = prepared.render_final(&BTreeMap::new()).unwrap();

        assert_eq!(rendered, "# nanite\n");
        assert_eq!(prepared.values["repo_name"], "nanite");
    }

    #[test]
    fn builds_ai_document_with_active_sentinel_and_prior_replacements() {
        let mut prompter = StaticPrompter::new(BTreeMap::from([(
            "project_name".to_owned(),
            "nanite".to_owned(),
        )]));
        let prepared = template(
            "README.md",
            "# {{project_name}}\n\n{{ai:first block}}\n\n## Usage\n\n{{ai:second block}}\n",
        )
        .prepare(&mut prompter)
        .unwrap();
        let context = prepared.build_context_bundle(Utf8Path::new("/tmp/project"));
        let second = prepared.ai_fragments()[1].clone();
        let mut resolved = BTreeMap::new();
        resolved.insert(0, "Resolved summary.".to_owned());

        let request = prepared
            .build_ai_fragment_request(
                Utf8Path::new("/tmp/project"),
                &context,
                &second,
                &resolved,
                &[],
            )
            .unwrap();

        assert!(request.document.contains("Resolved summary."));
        assert!(request.document.contains(&ai_sentinel(1)));
        assert!(!request.document.contains("{{ai:first block}}"));
    }

    #[test]
    fn final_render_requires_all_ai_replacements() {
        let prepared = PreparedTemplate {
            output_name: "README.md".to_owned(),
            source_path: "/tmp/README.md".into(),
            fragments: vec![TemplateFragment::Ai(AiPlaceholder {
                index: 0,
                prompt: "write summary".to_owned(),
            })],
            values: BTreeMap::new(),
        };

        let error = prepared
            .render_final(&BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing AI replacement"));
    }

    #[test]
    fn selection_labels_show_bundle_files_and_output_mapping() {
        let repository = TemplateRepository {
            bundles: vec![TemplateBundle {
                name: "default".to_owned(),
                source_path: "/tmp/templates/default".into(),
                templates: vec![
                    TemplateVariant {
                        output_name: "README.md".to_owned(),
                        source_path: "/tmp/templates/default/README.md".into(),
                        fragments: vec![],
                    },
                    TemplateVariant {
                        output_name: "AGENTS.md".to_owned(),
                        source_path: "/tmp/templates/default/agent-guide.md".into(),
                        fragments: vec![],
                    },
                ],
            }],
        };

        assert_eq!(
            repository.selection_labels(),
            vec!["default -> README.md, agent-guide.md -> AGENTS.md".to_owned()]
        );
    }

    #[test]
    fn bundle_prepare_prompts_once_across_multiple_files() {
        let mut prompter = StaticPrompter::new(BTreeMap::from([(
            "project_name".to_owned(),
            "nanite".to_owned(),
        )]));
        let bundle = TemplateBundle {
            name: "default".to_owned(),
            source_path: "/tmp/default".into(),
            templates: vec![
                template("AGENTS.md", "# {{project_name}}\n"),
                template("README.md", "# {{project_name}}\n"),
            ],
        };

        let prepared = bundle.prepare(&mut prompter).unwrap();

        assert_eq!(prepared.values["project_name"], "nanite");
        assert_eq!(prepared.templates.len(), 2);
        assert_eq!(prepared.templates[0].values["project_name"], "nanite");
        assert_eq!(prepared.templates[1].values["project_name"], "nanite");
    }

    #[test]
    fn template_metadata_is_minimal() {
        let metadata: TemplateMetadata = serde_yaml::from_str("filename: README.md\n").unwrap();
        assert_eq!(metadata.filename, "README.md");
    }

    #[test]
    fn readme_verifier_accepts_canonical_structure() {
        let mut prompter = StaticPrompter::new(BTreeMap::from([(
            "project_name".to_owned(),
            "nanite".to_owned(),
        )]));
        let prepared = template(
            "README.md",
            "# {{project_name}}\n\n{{ai:badges}}\n\n{{ai:overview}}\n\n## Quick Start\n\n{{ai:quick start}}\n\n## Usage\n\n{{ai:usage}}\n\n## Tests\n\n{{ai:tests}}\n\n## Contributing\n\n- Follow `CONTRIBUTING.md` when the repository provides it.\n- Keep changes focused and run the relevant checks before handing work off.\n\n## License\n\n- Refer to the repository license files and metadata for the current license terms.\n",
        )
        .prepare(&mut prompter)
        .unwrap();
        let context = prepared.build_context_bundle(Utf8Path::new("/tmp/project"));
        let ai_values = BTreeMap::from([
            (0, String::new()),
            (
                1,
                "nanite keeps project files, docs, and workspace routines aligned around one local workflow.\n\nIt reduces repeated setup work and keeps repository bootstrapping predictable.".to_owned(),
            ),
            (
                2,
                "- Install dependencies with the verified setup command.\n- Start from the repository root and use the verified run path.\n".to_owned(),
            ),
            (
                3,
                "- Use the documented repository workflow from the repo brief.\n- Keep commands and local structure consistent with the generated docs.\n".to_owned(),
            ),
            (
                4,
                "- No verified test command was found.\n".to_owned(),
            ),
        ]);

        let rendered = prepared.render_final(&ai_values).unwrap();
        let report = prepared.verify_readme(&rendered, &context, &ai_values);

        assert!(report.is_valid(), "{:?}", report.findings);
    }

    #[test]
    fn readme_verifier_flags_invalid_overview_for_repair() {
        let mut prompter = StaticPrompter::new(BTreeMap::from([(
            "project_name".to_owned(),
            "nanite".to_owned(),
        )]));
        let prepared = template(
            "README.md",
            "# {{project_name}}\n\n{{ai:badges}}\n\n{{ai:overview}}\n\n## Quick Start\n\n{{ai:quick start}}\n\n## Usage\n\n{{ai:usage}}\n\n## Tests\n\n{{ai:tests}}\n\n## Contributing\n\n- Follow `CONTRIBUTING.md` when the repository provides it.\n- Keep changes focused and run the relevant checks before handing work off.\n\n## License\n\n- Refer to the repository license files and metadata for the current license terms.\n",
        )
        .prepare(&mut prompter)
        .unwrap();
        let context = prepared.build_context_bundle(Utf8Path::new("/tmp/project"));
        let ai_values = BTreeMap::from([
            (0, String::new()),
            (
                1,
                "nanite is a tool. It keeps work moving. It helps with setup. It also standardizes workflows.".to_owned(),
            ),
            (
                2,
                "- Install dependencies with the verified setup command.\n- Start from the repository root and use the verified run path.\n".to_owned(),
            ),
            (
                3,
                "- Use the documented repository workflow from the repo brief.\n- Keep commands and local structure consistent with the generated docs.\n".to_owned(),
            ),
            (
                4,
                "- No verified test command was found.\n".to_owned(),
            ),
        ]);

        let rendered = prepared.render_final(&ai_values).unwrap();
        let report = prepared.verify_readme(&rendered, &context, &ai_values);

        assert!(!report.is_valid());
        assert!(report.repairable_fragment_indexes().contains(&1));
        assert!(
            report
                .render_messages()
                .iter()
                .any(|message| message.contains("overview must be 2 or 3 sentences"))
        );
    }
}
