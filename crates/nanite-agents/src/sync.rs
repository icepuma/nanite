use crate::fs::{
    ExistingTargetKind, diff_existing_tree, ensure_symlink, existing_target_kind,
    write_rendered_tree,
};
use crate::model::{
    CanonicalSkill, FileDiff, SyncAction, SyncItem, SyncReason, SyncReport, SyncTarget,
};
use crate::render::{render_claude_skill, render_codex_skill};
use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use std::fs;

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
        let action = classify_sync(std::slice::from_ref(&target));
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

fn inspect_codex_target(
    render_path: &Utf8Path,
    install_path: &Utf8Path,
    rendered: &std::collections::BTreeMap<Utf8PathBuf, Vec<u8>>,
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
    rendered: &std::collections::BTreeMap<Utf8PathBuf, Vec<u8>>,
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
    rendered: &std::collections::BTreeMap<Utf8PathBuf, Vec<u8>>,
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
