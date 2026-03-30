#![allow(
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::similar_names,
    clippy::uninlined_format_args
)]

use anyhow::{Context, Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use nanite_core::frontmatter::parse_frontmatter;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillProvider {
    Codex,
    Claude,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub providers: ProviderOverrides,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProviderOverrides {
    #[serde(default)]
    pub claude: ClaudeOverrides,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ClaudeOverrides {
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalSkill {
    pub slug: String,
    pub metadata: SkillMetadata,
    pub body: String,
    pub raw_document: String,
    pub resources: BTreeMap<Utf8PathBuf, Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    Create,
    Override,
    Unchanged,
}

impl SyncAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Override => "override",
            Self::Unchanged => "unchanged",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub added: Vec<Utf8PathBuf>,
    pub changed: Vec<Utf8PathBuf>,
    pub removed: Vec<Utf8PathBuf>,
}

impl FileDiff {
    fn from_rendered(rendered: &BTreeMap<Utf8PathBuf, Vec<u8>>) -> Self {
        Self {
            added: rendered.keys().cloned().collect(),
            changed: Vec::new(),
            removed: Vec::new(),
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncReason {
    Missing { diff: FileDiff },
    ContentChanged { diff: FileDiff },
    WrongSymlink { expected: String, actual: String },
    NotSymlink,
    NotDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncTarget {
    pub path: Utf8PathBuf,
    pub reasons: Vec<SyncReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncItem {
    pub slug: String,
    pub action: SyncAction,
    pub targets: Vec<SyncTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub items: Vec<SyncItem>,
}

pub fn load_skills(skills_root: &Utf8Path) -> Result<Vec<CanonicalSkill>> {
    let entries =
        fs::read_dir(skills_root).with_context(|| format!("failed to read {}", skills_root))?;
    let mut skills = Vec::new();

    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let skill_dir = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| anyhow!("non-UTF-8 skill path: {}", path.display()))?;
        let slug = skill_dir
            .file_name()
            .ok_or_else(|| anyhow!("failed to derive a skill name for {skill_dir}"))?
            .to_owned();
        let skill_file = skill_dir.join("SKILL.md");
        let raw = fs::read_to_string(skill_file.as_std_path())
            .with_context(|| format!("failed to read {}", skill_file))?;
        let document = parse_frontmatter::<SkillMetadata>(&raw)
            .with_context(|| format!("failed to parse {}", skill_file))?;

        let mut resources = BTreeMap::new();
        collect_resources(&skill_dir, Utf8Path::new(""), &mut resources)?;

        skills.push(CanonicalSkill {
            slug,
            metadata: document.metadata,
            body: document.body,
            raw_document: raw,
            resources,
        });
    }

    skills.sort_by(|left, right| left.slug.cmp(&right.slug));
    Ok(skills)
}

pub fn sync_codex(
    skills: &[CanonicalSkill],
    render_root: &Utf8Path,
    install_root: &Utf8Path,
    apply: bool,
) -> Result<SyncReport> {
    fs::create_dir_all(render_root).with_context(|| format!("failed to create {}", render_root))?;
    fs::create_dir_all(install_root)
        .with_context(|| format!("failed to create {}", install_root))?;

    let mut items = Vec::new();
    for skill in skills {
        let rendered = render_codex_skill(skill);
        let render_path = render_root.join(&skill.slug);
        let install_path = install_root.join(&skill.slug);
        let target = inspect_codex_target(&render_path, &install_path, &rendered)?;
        let action = classify_sync(&[target.clone()]);
        if apply && action != SyncAction::Unchanged {
            write_rendered_tree(&render_path, &rendered)?;
            ensure_symlink(&render_path, &install_path)?;
        }

        items.push(SyncItem {
            slug: skill.slug.clone(),
            action,
            targets: vec![target],
        });
    }

    Ok(SyncReport { items })
}

pub fn sync_claude(
    skills: &[CanonicalSkill],
    plugin_seed_dirs: &[Utf8PathBuf],
    apply: bool,
) -> Result<SyncReport> {
    let mut items = Vec::new();
    for skill in skills {
        let rendered = render_claude_skill(skill);
        let mut targets = Vec::new();

        for seed_dir in plugin_seed_dirs {
            let target = seed_dir.join("nanite-skills/skills").join(&skill.slug);
            targets.push(inspect_directory_target(&target, &rendered)?);
        }

        let action = classify_sync(&targets);

        if apply {
            for seed_dir in plugin_seed_dirs {
                write_claude_plugin(seed_dir, skill, &rendered)?;
            }
        }

        items.push(SyncItem {
            slug: skill.slug.clone(),
            action,
            targets,
        });
    }

    Ok(SyncReport { items })
}

fn render_codex_skill(skill: &CanonicalSkill) -> BTreeMap<Utf8PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert(
        Utf8PathBuf::from("SKILL.md"),
        skill.raw_document.as_bytes().to_vec(),
    );
    for (relative_path, contents) in &skill.resources {
        files.insert(relative_path.clone(), contents.clone());
    }
    files
}

fn render_claude_skill(skill: &CanonicalSkill) -> BTreeMap<Utf8PathBuf, Vec<u8>> {
    let description = skill
        .metadata
        .providers
        .claude
        .description
        .clone()
        .unwrap_or_else(|| skill.metadata.description.clone());
    let header = format!("---\ndescription: {description}\n---\n");

    let mut files = BTreeMap::new();
    files.insert(
        Utf8PathBuf::from("SKILL.md"),
        format!("{header}{}", skill.body).into_bytes(),
    );
    for (relative_path, contents) in &skill.resources {
        files.insert(relative_path.clone(), contents.clone());
    }
    files
}

fn inspect_codex_target(
    render_path: &Utf8Path,
    install_path: &Utf8Path,
    rendered: &BTreeMap<Utf8PathBuf, Vec<u8>>,
) -> Result<SyncTarget> {
    let mut reasons = Vec::new();

    match fs::symlink_metadata(install_path.as_std_path()) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                let actual_target = fs::read_link(install_path.as_std_path())
                    .with_context(|| format!("failed to read {}", install_path))?;
                if actual_target != render_path.as_std_path() {
                    reasons.push(SyncReason::WrongSymlink {
                        expected: render_path.as_str().to_owned(),
                        actual: actual_target.display().to_string(),
                    });
                }
            } else {
                reasons.push(SyncReason::NotSymlink);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            reasons.push(SyncReason::Missing {
                diff: FileDiff::from_rendered(rendered),
            });
            return Ok(SyncTarget {
                path: install_path.to_owned(),
                reasons,
            });
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", install_path));
        }
    }

    match existing_target_kind(install_path)? {
        ExistingTargetKind::Missing => {
            reasons.push(SyncReason::Missing {
                diff: FileDiff::from_rendered(rendered),
            });
        }
        ExistingTargetKind::Directory => {
            let diff = diff_existing_tree(install_path, rendered)?;
            if !diff.is_empty() {
                reasons.push(SyncReason::ContentChanged { diff });
            }
        }
        ExistingTargetKind::NotDirectory => {
            reasons.push(SyncReason::NotDirectory);
        }
    }

    Ok(SyncTarget {
        path: install_path.to_owned(),
        reasons,
    })
}

fn inspect_directory_target(
    target: &Utf8Path,
    rendered: &BTreeMap<Utf8PathBuf, Vec<u8>>,
) -> Result<SyncTarget> {
    let mut reasons = Vec::new();

    match existing_target_kind(target)? {
        ExistingTargetKind::Missing => {
            reasons.push(SyncReason::Missing {
                diff: FileDiff::from_rendered(rendered),
            });
        }
        ExistingTargetKind::Directory => {
            let diff = diff_existing_tree(target, rendered)?;
            if !diff.is_empty() {
                reasons.push(SyncReason::ContentChanged { diff });
            }
        }
        ExistingTargetKind::NotDirectory => reasons.push(SyncReason::NotDirectory),
    }

    Ok(SyncTarget {
        path: target.to_owned(),
        reasons,
    })
}

fn classify_sync(targets: &[SyncTarget]) -> SyncAction {
    if targets.iter().all(|target| target.reasons.is_empty()) {
        return SyncAction::Unchanged;
    }

    if targets
        .iter()
        .all(|target| matches!(target.reasons.as_slice(), [SyncReason::Missing { .. }]))
    {
        return SyncAction::Create;
    }

    SyncAction::Override
}

fn write_claude_plugin(
    seed_dir: &Utf8Path,
    skill: &CanonicalSkill,
    rendered: &BTreeMap<Utf8PathBuf, Vec<u8>>,
) -> Result<()> {
    let plugin_root = seed_dir.join("nanite-skills");
    let manifest_dir = plugin_root.join(".claude-plugin");
    let skill_dir = plugin_root.join("skills").join(&skill.slug);

    fs::create_dir_all(&manifest_dir)
        .with_context(|| format!("failed to create {}", manifest_dir))?;
    fs::create_dir_all(plugin_root.join("skills"))
        .with_context(|| format!("failed to create {}", plugin_root.join("skills")))?;
    fs::write(
        manifest_dir.join("plugin.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": "nanite-skills",
            "description": "Nanite-managed skill bundle",
            "author": { "name": "Nanite" }
        }))?,
    )
    .with_context(|| format!("failed to write {}", manifest_dir.join("plugin.json")))?;

    write_rendered_tree(&skill_dir, rendered)
}

fn collect_resources(
    skill_root: &Utf8Path,
    relative_root: &Utf8Path,
    resources: &mut BTreeMap<Utf8PathBuf, Vec<u8>>,
) -> Result<()> {
    let current_root = if relative_root.as_str().is_empty() {
        skill_root.to_owned()
    } else {
        skill_root.join(relative_root)
    };

    for entry in
        fs::read_dir(&current_root).with_context(|| format!("failed to read {}", current_root))?
    {
        let entry = entry?;
        let file_name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("skill resource names must be UTF-8"))?;
        if relative_root.as_str().is_empty() && file_name == "SKILL.md" {
            continue;
        }

        let relative_path = if relative_root.as_str().is_empty() {
            Utf8PathBuf::from(&file_name)
        } else {
            relative_root.join(&file_name)
        };
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| anyhow!("non-UTF-8 path encountered: {}", path.display()))?;
        if entry.file_type()?.is_dir() {
            collect_resources(skill_root, &relative_path, resources)?;
            continue;
        }

        resources.insert(
            relative_path,
            fs::read(&path).with_context(|| format!("failed to read {}", path))?,
        );
    }

    Ok(())
}

fn diff_existing_tree(
    target_dir: &Utf8Path,
    rendered: &BTreeMap<Utf8PathBuf, Vec<u8>>,
) -> Result<FileDiff> {
    let mut existing = BTreeMap::new();
    collect_files(target_dir, Utf8Path::new(""), &mut existing)?;
    Ok(diff_trees(&existing, rendered))
}

fn diff_trees(
    existing: &BTreeMap<Utf8PathBuf, Vec<u8>>,
    rendered: &BTreeMap<Utf8PathBuf, Vec<u8>>,
) -> FileDiff {
    let added = rendered
        .keys()
        .filter(|path| !existing.contains_key(*path))
        .cloned()
        .collect();
    let changed = rendered
        .iter()
        .filter_map(|(path, contents)| match existing.get(path) {
            Some(current) if current != contents => Some(path.clone()),
            _ => None,
        })
        .collect();
    let removed = existing
        .keys()
        .filter(|path| !rendered.contains_key(*path))
        .cloned()
        .collect();

    FileDiff {
        added,
        changed,
        removed,
    }
}

fn collect_files(
    root: &Utf8Path,
    relative_root: &Utf8Path,
    files: &mut BTreeMap<Utf8PathBuf, Vec<u8>>,
) -> Result<()> {
    let current_root = if relative_root.as_str().is_empty() {
        root.to_owned()
    } else {
        root.join(relative_root)
    };

    for entry in
        fs::read_dir(&current_root).with_context(|| format!("failed to read {}", current_root))?
    {
        let entry = entry?;
        let file_name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("file names must be UTF-8"))?;
        let relative_path = if relative_root.as_str().is_empty() {
            Utf8PathBuf::from(&file_name)
        } else {
            relative_root.join(&file_name)
        };
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| anyhow!("non-UTF-8 path encountered: {}", path.display()))?;
        if entry.file_type()?.is_dir() {
            collect_files(root, &relative_path, files)?;
        } else {
            files.insert(relative_path, fs::read(&path)?);
        }
    }

    Ok(())
}

enum ExistingTargetKind {
    Missing,
    Directory,
    NotDirectory,
}

fn existing_target_kind(path: &Utf8Path) -> Result<ExistingTargetKind> {
    if !path.exists() {
        return Ok(ExistingTargetKind::Missing);
    }

    let metadata =
        fs::metadata(path.as_std_path()).with_context(|| format!("failed to inspect {}", path))?;
    if metadata.is_dir() {
        return Ok(ExistingTargetKind::Directory);
    }

    Ok(ExistingTargetKind::NotDirectory)
}

fn write_rendered_tree(
    target_dir: &Utf8Path,
    rendered: &BTreeMap<Utf8PathBuf, Vec<u8>>,
) -> Result<()> {
    if target_dir.exists() {
        fs::remove_dir_all(target_dir)
            .with_context(|| format!("failed to remove {}", target_dir))?;
    }
    fs::create_dir_all(target_dir).with_context(|| format!("failed to create {}", target_dir))?;

    for (relative_path, contents) in rendered {
        let target = target_dir.join(relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent))?;
        }
        fs::write(&target, contents).with_context(|| format!("failed to write {}", target))?;
    }

    Ok(())
}

fn ensure_symlink(target: &Utf8Path, link_path: &Utf8Path) -> Result<()> {
    if link_path.exists() || link_path.as_std_path().symlink_metadata().is_ok() {
        remove_path(link_path)?;
    }

    if let Some(parent) = link_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        symlink(target, link_path).with_context(|| format!("failed to link {}", link_path))?;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_dir;

        symlink_dir(target, link_path).with_context(|| format!("failed to link {}", link_path))?;
    }

    Ok(())
}

fn remove_path(path: &Utf8Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("failed to inspect {}", path))?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path))?;
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CanonicalSkill, SkillMetadata, SyncAction, SyncReason, ensure_symlink, remove_path,
        sync_claude, sync_codex,
    };
    use camino::Utf8PathBuf;
    use std::fs;
    use tempfile::tempdir;

    fn sample_skill() -> CanonicalSkill {
        CanonicalSkill {
            slug: "example-skill".to_owned(),
            metadata: SkillMetadata {
                name: "example-skill".to_owned(),
                description: "Summarize a repository".to_owned(),
                triggers: vec!["summarize repo".to_owned()],
                tags: vec!["analysis".to_owned()],
                providers: super::ProviderOverrides::default(),
            },
            body: "Read the repository and summarize it.\n".to_owned(),
            raw_document: concat!(
                "---\n",
                "name: example-skill\n",
                "description: Summarize a repository\n",
                "triggers:\n",
                "  - summarize repo\n",
                "tags:\n",
                "  - analysis\n",
                "---\n",
                "Read the repository and summarize it.\n"
            )
            .to_owned(),
            resources: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn codex_sync_reports_create_then_unchanged() {
        let tempdir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let render = root.join("render");
        let install = root.join("install");
        let skills = vec![sample_skill()];

        let first = sync_codex(&skills, &render, &install, true).unwrap();
        let second = sync_codex(&skills, &render, &install, false).unwrap();

        assert_eq!(first.items[0].action, SyncAction::Create);
        assert_eq!(second.items[0].action, SyncAction::Unchanged);
        assert_eq!(
            fs::read_to_string(render.join("example-skill/SKILL.md")).unwrap(),
            sample_skill().raw_document
        );
    }

    #[test]
    fn claude_sync_reports_create_for_missing_bundle() {
        let tempdir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let report = sync_claude(&[sample_skill()], &[root], false).unwrap();

        assert_eq!(report.items[0].action, SyncAction::Create);
    }

    #[test]
    fn codex_sync_reports_content_drift() {
        let tempdir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let render = root.join("render");
        let install = root.join("install");
        let skills = vec![sample_skill()];

        sync_codex(&skills, &render, &install, true).unwrap();
        fs::write(render.join("example-skill/SKILL.md"), "stale\n").unwrap();

        let report = sync_codex(&skills, &render, &install, false).unwrap();
        let target = &report.items[0].targets[0];

        assert_eq!(report.items[0].action, SyncAction::Override);
        assert!(matches!(
            target.reasons.as_slice(),
            [SyncReason::ContentChanged { diff }]
                if diff.changed == vec![Utf8PathBuf::from("SKILL.md")]
        ));
    }

    #[test]
    fn codex_sync_reports_wrong_symlink_targets() {
        let tempdir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let render = root.join("render");
        let install = root.join("install");
        let wrong = root.join("wrong/example-skill");
        let skills = vec![sample_skill()];

        sync_codex(&skills, &render, &install, true).unwrap();
        fs::create_dir_all(&wrong).unwrap();
        fs::write(wrong.join("SKILL.md"), sample_skill().raw_document).unwrap();
        remove_path(&install.join("example-skill")).unwrap();
        ensure_symlink(&wrong, &install.join("example-skill")).unwrap();

        let report = sync_codex(&skills, &render, &install, false).unwrap();

        assert_eq!(report.items[0].action, SyncAction::Override);
        assert!(matches!(
            report.items[0].targets[0].reasons.as_slice(),
            [SyncReason::WrongSymlink { .. }]
        ));
    }
}
