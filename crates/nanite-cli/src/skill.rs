use crate::cli::{ProviderArg, SkillCommands};
use crate::context::ContextState;
use anyhow::Result;
use nanite_agents::{
    FileDiff, SyncAction, SyncReason, SyncReport, SyncTarget, load_skills, sync_claude, sync_codex,
};
use std::io::{self, IsTerminal};

pub fn command_skill(context: &ContextState, command: SkillCommands) -> Result<()> {
    match command {
        SkillCommands::Sync { provider, apply } => {
            let skills = load_skills(context.workspace_paths.skills_root())?;
            let report = match provider {
                ProviderArg::Codex => sync_codex(
                    &skills,
                    &context.app_paths.codex_render_root(),
                    &context.app_paths.codex_skills_root(),
                    apply,
                )?,
                ProviderArg::Claude => {
                    let seed_root = context.app_paths.claude_plugin_seed_root();
                    sync_claude(&skills, std::slice::from_ref(&seed_root), apply)?
                }
            };
            print_sync_report(provider, apply, &report);
        }
    }

    Ok(())
}

fn print_sync_report(provider: ProviderArg, apply: bool, report: &SyncReport) {
    let theme = CliTheme::detect();
    let title = if apply {
        format!("sync {} skills", provider.as_str())
    } else {
        format!("sync {} skills (dry run)", provider.as_str())
    };
    let create_count = report
        .items
        .iter()
        .filter(|item| item.action == SyncAction::Create)
        .count();
    let update_count = report
        .items
        .iter()
        .filter(|item| item.action == SyncAction::Override)
        .count();
    let ok_count = report
        .items
        .iter()
        .filter(|item| item.action == SyncAction::Unchanged)
        .count();

    println!("{}", theme.bold(&title));
    println!(
        "{} {}  {} {}  {} {}",
        theme.green(&create_count.to_string()),
        theme.dim("create"),
        theme.yellow(&update_count.to_string()),
        theme.dim("update"),
        theme.blue(&ok_count.to_string()),
        theme.dim("ok"),
    );

    for item in &report.items {
        println!();
        print_sync_item(item, &theme);
    }
}

fn print_sync_item(item: &nanite_agents::SyncItem, theme: &CliTheme) {
    println!(
        "{} {}",
        format_action_badge(item.action, theme),
        theme.bold(&item.slug)
    );

    for target in &item.targets {
        print_sync_target(target, theme);
    }
}

fn print_sync_target(target: &SyncTarget, theme: &CliTheme) {
    println!("  {} {}", theme.dim("path"), target.path);
    if target.reasons.is_empty() {
        println!("  {} {}", theme.dim("state"), theme.blue("up to date"));
        return;
    }

    for reason in &target.reasons {
        print_sync_reason(reason, theme);
    }
}

fn print_sync_reason(reason: &SyncReason, theme: &CliTheme) {
    match reason {
        SyncReason::Missing { diff } => {
            println!("  {} {}", theme.dim("state"), theme.green("missing"));
            print_file_diff(diff, theme);
        }
        SyncReason::ContentChanged { diff } => {
            println!(
                "  {} {}",
                theme.dim("state"),
                theme.yellow("content changed")
            );
            print_file_diff(diff, theme);
        }
        SyncReason::WrongSymlink { expected, actual } => {
            println!(
                "  {} {}",
                theme.dim("state"),
                theme.yellow("symlink target changed")
            );
            println!("  {} {}", theme.dim("actual"), actual);
            println!("  {} {}", theme.dim("expect"), expected);
        }
        SyncReason::NotSymlink => {
            println!(
                "  {} {}",
                theme.dim("state"),
                theme.red("exists, but is not a symlink")
            );
        }
        SyncReason::NotDirectory => {
            println!(
                "  {} {}",
                theme.dim("state"),
                theme.red("exists, but is not a directory")
            );
        }
    }
}

fn print_file_diff(diff: &FileDiff, theme: &CliTheme) {
    if diff.is_empty() {
        return;
    }

    println!("  {}", theme.dim("diff"));
    for path in &diff.added {
        println!("    {} {}", theme.green("+"), path);
    }
    for path in &diff.changed {
        println!("    {} {}", theme.yellow("~"), path);
    }
    for path in &diff.removed {
        println!("    {} {}", theme.red("-"), path);
    }
}

fn format_action_badge(action: SyncAction, theme: &CliTheme) -> String {
    match action {
        SyncAction::Create => theme.green("[create]"),
        SyncAction::Override => theme.yellow("[update]"),
        SyncAction::Unchanged => theme.blue("[ok]"),
    }
}

struct CliTheme {
    color: bool,
}

impl CliTheme {
    fn detect() -> Self {
        Self {
            color: io::stdout().is_terminal()
                && std::env::var_os("NO_COLOR").is_none()
                && std::env::var("TERM").map_or(true, |term| term != "dumb"),
        }
    }

    fn bold(&self, value: &str) -> String {
        self.paint("1", value)
    }

    fn dim(&self, value: &str) -> String {
        self.paint("2", value)
    }

    fn blue(&self, value: &str) -> String {
        self.paint("34", value)
    }

    fn green(&self, value: &str) -> String {
        self.paint("32", value)
    }

    fn red(&self, value: &str) -> String {
        self.paint("31", value)
    }

    fn yellow(&self, value: &str) -> String {
        self.paint("33", value)
    }

    fn paint(&self, code: &str, value: &str) -> String {
        if self.color {
            return format!("\u{1b}[{code}m{value}\u{1b}[0m");
        }

        value.to_owned()
    }
}
