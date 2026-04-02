mod cli;
mod context;
mod generate;
pub(crate) mod gitignore_catalog;
mod init;
mod jump;
pub(crate) mod license_catalog;
mod repo;
mod setup;
mod shell;
mod skill;
#[cfg(test)]
mod tests;
mod ui;
mod util;

use anyhow::Result;
use clap::Parser;
use nanite_core::AppPaths;

pub use cli::build_cli;

/// Runs the Nanite CLI and returns the process exit code.
///
/// # Errors
///
/// Returns an error when application paths, configuration, or a selected
/// subcommand cannot be resolved or executed successfully.
pub fn run() -> Result<i32> {
    run_with(cli::Cli::parse())
}

fn run_with(cli: cli::Cli) -> Result<i32> {
    let app_paths = AppPaths::discover()?;
    let git_binary = std::env::var("NANITE_GIT").unwrap_or_else(|_| "git".to_owned());
    let fzf_binary = std::env::var("NANITE_FZF").unwrap_or_else(|_| "fzf".to_owned());

    match cli.command {
        cli::Commands::Setup { path } => {
            setup::command_setup(&app_paths, &util::bundled_content_root()?, &path)?;
            Ok(0)
        }
        cli::Commands::Init { force } => {
            let context = context::ContextState::load(&app_paths, &git_binary, &fzf_binary)?;
            init::command_init(&context, force)?;
            Ok(0)
        }
        cli::Commands::Generate { command } => {
            generate::command_generate(command, &git_binary)?;
            Ok(0)
        }
        cli::Commands::Repo { command } => {
            let context = context::ContextState::load(&app_paths, &git_binary, &fzf_binary)?;
            repo::command_repo(&context, command)?;
            Ok(0)
        }
        cli::Commands::Skill { command } => {
            let context = context::ContextState::load(&app_paths, &git_binary, &fzf_binary)?;
            skill::command_skill(&context, command)?;
            Ok(0)
        }
        cli::Commands::Jumpto { query } => {
            let context = context::ContextState::load(&app_paths, &git_binary, &fzf_binary)?;
            Ok(
                jump::command_jumpto(&context, query.as_deref())?.map_or(1, |path| {
                    println!("{path}");
                    0
                }),
            )
        }
        cli::Commands::Shell { command } => {
            let context = context::ContextState::load(&app_paths, &git_binary, &fzf_binary)?;
            shell::command_shell(&context, command);
            Ok(0)
        }
        cli::Commands::CompleteJumpto => {
            let context = context::ContextState::load(&app_paths, &git_binary, &fzf_binary)?;
            jump::command_complete_jumpto(&context)?;
            Ok(0)
        }
        cli::Commands::CompleteRepoRemove => {
            let context = context::ContextState::load(&app_paths, &git_binary, &fzf_binary)?;
            jump::command_complete_repo_remove(&context)?;
            Ok(0)
        }
    }
}
