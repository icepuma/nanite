use anyhow::{Context, Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use nanite_core::{AppPaths, Registry};
use std::fs;

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

pub fn canonicalize_utf8(path: &Utf8Path) -> Result<Utf8PathBuf> {
    utf8_from_path_buf(
        fs::canonicalize(path.as_std_path())
            .with_context(|| format!("failed to resolve {path}"))?,
    )
}

pub fn utf8_from_path_buf(path: std::path::PathBuf) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path).map_err(|path| anyhow!("non-UTF-8 path: {}", path.display()))
}
