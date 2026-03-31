use crate::remote::{RemoteSpec, parse_remote};
use crate::workspace::{destination_for, remove_existing_path};
use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use std::fs;

pub fn remove_repo(workspace_root: &Utf8Path, target: &str) -> Result<Utf8PathBuf> {
    let destination = resolve_remove_destination(workspace_root, target)?;
    if !destination.exists() {
        bail!("{destination} does not exist");
    }
    if destination == workspace_root {
        bail!("refusing to remove workspace root {workspace_root}");
    }

    remove_existing_path(&destination)?;
    prune_empty_repo_parents(workspace_root, &destination)?;
    Ok(destination)
}

pub fn resolve_repo_remove_target(workspace_root: &Utf8Path, target: &str) -> Result<Utf8PathBuf> {
    resolve_remove_destination(workspace_root, target)
}

fn resolve_remove_destination(workspace_root: &Utf8Path, target: &str) -> Result<Utf8PathBuf> {
    let target = target.trim();
    if target.is_empty() {
        bail!("repo remove target must not be empty");
    }

    if let Ok(spec) = parse_remote(target) {
        return Ok(destination_for(workspace_root, &spec));
    }

    let path = Utf8Path::new(target);
    if path.is_absolute() {
        let absolute = path.to_owned();
        super::scan::relative_spec(workspace_root, &absolute)?;
        return Ok(absolute);
    }

    let spec = parse_workspace_relative_spec(target)?;
    Ok(destination_for(workspace_root, &spec))
}

fn parse_workspace_relative_spec(target: &str) -> Result<RemoteSpec> {
    let trimmed = target.trim().trim_start_matches('/').trim_end_matches('/');
    let segments = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.len() < 2 {
        bail!("workspace repo targets must look like <host>/<repo-path>");
    }
    for segment in &segments {
        if *segment == "." || *segment == ".." {
            bail!("workspace repo target contains an invalid segment: {segment}");
        }
    }

    Ok(RemoteSpec {
        host: segments[0].to_owned(),
        repo_path: segments[1..].join("/"),
    })
}

fn prune_empty_repo_parents(workspace_root: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    let mut current = destination.parent();
    while let Some(parent) = current {
        if parent == workspace_root {
            break;
        }
        if !parent.starts_with(workspace_root) {
            bail!("{parent} is not inside {workspace_root}");
        }
        if !directory_is_empty(parent)? {
            break;
        }
        fs::remove_dir(parent).with_context(|| format!("failed to remove empty {}", parent))?;
        current = parent.parent();
    }
    Ok(())
}

fn directory_is_empty(path: &Utf8Path) -> Result<bool> {
    let mut entries =
        fs::read_dir(path).with_context(|| format!("failed to read directory {}", path))?;
    Ok(entries.next().is_none())
}
