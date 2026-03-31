use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use std::fs;

pub fn copy_dir_recursive(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    if !source.is_dir() {
        bail!("{source} is not a directory");
    }

    fs::create_dir_all(destination).with_context(|| format!("failed to create {destination}"))?;

    for entry in fs::read_dir(source).with_context(|| format!("failed to read {source}"))? {
        let entry = entry?;
        let path = camino::Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| anyhow::anyhow!("non-UTF-8 path encountered: {}", path.display()))?;
        let name = path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("failed to determine file name for {path}"))?;
        let target = destination.join(name);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("failed to read metadata for {path}"))?;

        if metadata.file_type().is_dir() {
            copy_dir_recursive(&path, &target)?;
            continue;
        }

        if metadata.file_type().is_symlink() {
            copy_symlink(&path, &target)?;
            continue;
        }

        fs::copy(&path, &target).with_context(|| format!("failed to copy {path} to {target}"))?;
    }

    Ok(())
}

fn copy_symlink(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    let target =
        fs::read_link(source).with_context(|| format!("failed to inspect symlink {source}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        if destination.exists() {
            remove_existing(destination)?;
        }
        symlink(&target, destination)
            .with_context(|| format!("failed to create symlink {destination}"))?;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        if destination.exists() {
            remove_existing(destination)?;
        }
        let target_metadata =
            fs::metadata(source).with_context(|| format!("failed to inspect {source}"))?;
        if target_metadata.is_dir() {
            symlink_dir(&target, destination)
                .with_context(|| format!("failed to create symlink {destination}"))?;
        } else {
            symlink_file(&target, destination)
                .with_context(|| format!("failed to create symlink {destination}"))?;
        }
    }

    Ok(())
}

fn remove_existing(path: &Utf8Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("failed to inspect {path}"))?;
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {path}"))?;
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove {path}"))?;
    }
    Ok(())
}
