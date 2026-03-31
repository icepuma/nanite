use crate::model::FileDiff;
use anyhow::{Context, Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use std::collections::BTreeMap;
use std::fs;

pub fn diff_existing_tree(
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
        fs::read_dir(&current_root).with_context(|| format!("failed to read {current_root}"))?
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

pub enum ExistingTargetKind {
    Missing,
    Directory,
    NotDirectory,
}

pub fn existing_target_kind(path: &Utf8Path) -> Result<ExistingTargetKind> {
    if !path.exists() {
        return Ok(ExistingTargetKind::Missing);
    }

    let metadata =
        fs::metadata(path.as_std_path()).with_context(|| format!("failed to inspect {path}"))?;
    if metadata.is_dir() {
        return Ok(ExistingTargetKind::Directory);
    }

    Ok(ExistingTargetKind::NotDirectory)
}

pub fn write_rendered_tree(
    target_dir: &Utf8Path,
    rendered: &BTreeMap<Utf8PathBuf, Vec<u8>>,
) -> Result<()> {
    if target_dir.exists() {
        fs::remove_dir_all(target_dir).with_context(|| format!("failed to remove {target_dir}"))?;
    }
    fs::create_dir_all(target_dir).with_context(|| format!("failed to create {target_dir}"))?;

    for (relative_path, contents) in rendered {
        let target = target_dir.join(relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
        }
        fs::write(&target, contents).with_context(|| format!("failed to write {target}"))?;
    }

    Ok(())
}

pub fn ensure_symlink(target: &Utf8Path, link_path: &Utf8Path) -> Result<()> {
    if link_path.exists() || link_path.as_std_path().symlink_metadata().is_ok() {
        remove_path(link_path)?;
    }

    if let Some(parent) = link_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        symlink(target, link_path).with_context(|| format!("failed to link {link_path}"))?;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_dir;

        symlink_dir(target, link_path).with_context(|| format!("failed to link {link_path}"))?;
    }

    Ok(())
}

pub fn remove_path(path: &Utf8Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("failed to inspect {path}"))?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {path}"))?;
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove {path}"))?;
    }
    Ok(())
}
