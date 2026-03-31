use crate::cli::RepoCommands;
use crate::context::ContextState;
use crate::ui::inquire_render_config;
use crate::util::{load_registry, resolve_cli_path};
use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use indicatif::{ProgressBar, ProgressStyle};
use inquire::Confirm;
use nanite_git::{
    clone_repo, import_repo, remove_repo, resolve_repo_remove_target, scan_workspace,
};
use std::io::{self, IsTerminal};

pub fn command_repo(context: &ContextState, command: RepoCommands) -> Result<()> {
    let mut registry = load_registry(&context.app_paths)?;

    match command {
        RepoCommands::Clone { remote, force } => {
            let progress = clone_progress_bar(&remote);
            let result = clone_repo(
                context.workspace_paths.repos_root(),
                &remote,
                force,
                progress.clone(),
            );
            if let Some(progress) = &progress {
                progress.finish_and_clear();
            }
            let record = result?;
            println!("cloned {}", record.path);
            registry.upsert(record);
        }
        RepoCommands::Import { source } => {
            let source = resolve_cli_path(&source)?;
            let record = import_repo(
                context.workspace_paths.repos_root(),
                &source,
                &context.git_binary,
            )?;
            println!("imported {}", record.path);
            registry.upsert(record);
        }
        RepoCommands::Remove { target, yes } => {
            let destination =
                resolve_repo_remove_target(context.workspace_paths.repos_root(), &target)?;
            confirm_repo_removal(&destination, yes)?;
            let removed_path = remove_repo(context.workspace_paths.repos_root(), &target)?;
            println!("removed {removed_path}");
            registry.remove_path(&removed_path);
        }
        RepoCommands::Refresh => {
            let records =
                scan_workspace(&context.git_binary, context.workspace_paths.repos_root())?;
            let count = records.len();
            for record in records {
                registry.upsert(record);
            }
            println!("refreshed {count} repositories");
        }
    }

    registry.save(&context.app_paths.registry_file())
}

fn clone_progress_bar(remote: &str) -> Option<ProgressBar> {
    if !io::stdout().is_terminal() {
        return None;
    }

    let bar = ProgressBar::new(0);
    let style = ProgressStyle::with_template(
        "{spinner:.cyan} Cloning {msg} [{wide_bar:.cyan/blue}] {pos}/{len}",
    )
    .unwrap_or_else(|_| ProgressStyle::default_bar())
    .progress_chars("=> ");
    bar.set_style(style);
    bar.set_message(remote.to_owned());
    Some(bar)
}

fn confirm_repo_removal(path: &Utf8Path, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }

    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        let confirmed = Confirm::new(&format!("Remove repository at {path}?"))
            .with_render_config(inquire_render_config())
            .with_default(false)
            .prompt()
            .with_context(|| format!("failed to confirm removal of {path}"))?;
        if confirmed {
            return Ok(());
        }
        bail!("aborted removal of {path}");
    }

    bail!("repo remove requires confirmation; rerun with --yes");
}
