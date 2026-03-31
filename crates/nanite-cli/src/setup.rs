use crate::util::{
    canonicalize_utf8, copy_dir_contents, ensure_setup_target_is_empty, resolve_cli_path,
};
use anyhow::{Context, Result};
use camino::Utf8Path;
use nanite_core::{AppPaths, Config, WorkspacePaths};
use std::fs;

pub fn command_setup(
    app_paths: &AppPaths,
    bundled_content_root: &Utf8Path,
    path: &str,
) -> Result<()> {
    if let Some(config) = Config::load_optional(app_paths)? {
        anyhow::bail!(
            "nanite is already configured for {}; remove {} to reconfigure",
            config.workspace_root,
            app_paths.config_file()
        );
    }

    let workspace_root = resolve_cli_path(path)?;
    ensure_setup_target_is_empty(&workspace_root)?;
    fs::create_dir_all(workspace_root.as_std_path())
        .with_context(|| format!("failed to create {workspace_root}"))?;
    let workspace_root = canonicalize_utf8(&workspace_root)?;
    let workspace_paths = WorkspacePaths::new(workspace_root.clone());

    fs::create_dir_all(workspace_paths.templates_root())
        .with_context(|| format!("failed to create {}", workspace_paths.templates_root()))?;
    fs::create_dir_all(workspace_paths.skills_root())
        .with_context(|| format!("failed to create {}", workspace_paths.skills_root()))?;
    fs::create_dir_all(workspace_paths.repos_root())
        .with_context(|| format!("failed to create {}", workspace_paths.repos_root()))?;

    copy_dir_contents(
        &bundled_content_root.join("templates"),
        workspace_paths.templates_root(),
    )?;
    copy_dir_contents(
        &bundled_content_root.join("skills"),
        workspace_paths.skills_root(),
    )?;

    let defaults = Config::default_for(app_paths)?;
    Config {
        workspace_root: workspace_root.clone(),
        agent: defaults.agent,
    }
    .save(app_paths)?;

    println!("configured {workspace_root}");
    Ok(())
}
