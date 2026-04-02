use anyhow::{Context, Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use nanite_core::{AppPaths, Registry};
use std::fs;

pub fn command_available(command: &str) -> bool {
    which::which(command).is_ok()
}

pub fn current_directory() -> Result<Utf8PathBuf> {
    utf8_from_path_buf(std::env::current_dir().context("failed to resolve the current directory")?)
}

pub fn resolve_cli_path(value: &str) -> Result<Utf8PathBuf> {
    let path = Utf8PathBuf::from(value);
    if path.is_absolute() {
        return Ok(path);
    }

    Ok(current_directory()?.join(path))
}

pub fn load_registry(app_paths: &AppPaths) -> Result<Registry> {
    Registry::load(&app_paths.registry_file())
}

pub fn bundled_content_root() -> Result<Utf8PathBuf> {
    let manifest_dir = Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir.join("../..");
    let root = fs::canonicalize(root).context("failed to resolve workspace root")?;
    Ok(utf8_from_path_buf(root)?.join("content"))
}

pub fn ensure_setup_target_is_empty(path: &Utf8Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_dir() {
        anyhow::bail!("{path} is not a directory");
    }

    let mut entries =
        fs::read_dir(path).with_context(|| format!("failed to read workspace root {path}"))?;
    if entries.next().transpose()?.is_some() {
        anyhow::bail!("{path} is not empty");
    }

    Ok(())
}

pub fn copy_dir_contents(source_root: &Utf8Path, target_root: &Utf8Path) -> Result<()> {
    let entries =
        fs::read_dir(source_root).with_context(|| format!("failed to read {source_root}"))?;
    for entry in entries {
        let entry = entry?;
        let source_path = utf8_from_path_buf(entry.path())?;
        let target_path = target_root.join(
            source_path
                .file_name()
                .ok_or_else(|| anyhow!("failed to determine file name for {source_path}"))?,
        );

        if entry.file_type()?.is_dir() {
            fs::create_dir_all(target_path.as_std_path())
                .with_context(|| format!("failed to create {target_path}"))?;
            copy_dir_contents(&source_path, &target_path)?;
        } else {
            fs::copy(source_path.as_std_path(), target_path.as_std_path())
                .with_context(|| format!("failed to copy {source_path} to {target_path}"))?;
        }
    }

    Ok(())
}

pub fn canonicalize_utf8(path: &Utf8Path) -> Result<Utf8PathBuf> {
    utf8_from_path_buf(
        fs::canonicalize(path.as_std_path())
            .with_context(|| format!("failed to resolve {path}"))?,
    )
}

pub fn utf8_from_path_buf(path: std::path::PathBuf) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path).map_err(|path| anyhow!("non-UTF-8 path: {}", path.display()))
}

pub fn escape_fish_string(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}
