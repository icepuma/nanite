//! Bulk-clone flow: list repos via a WASM plugin, confirm with the
//! user, then run sequential `clone_repo` calls.
//!
//! Behavior is entirely interactive — there are no flags to override
//! prompts. If any destination already exists the user is asked once
//! up-front whether to overwrite all conflicts. Sources of truth:
//! plugin output (filtered host-side for archived/forks; we currently
//! drop both by default) and interactive prompts.
//!
//! ## Concurrency
//!
//! v1 clones sequentially. Parallelism is a planned follow-up — it
//! requires decoupling the existing single-clone TUI renderer from
//! worker threads so concurrent `terminal.draw()` calls do not
//! corrupt the screen.

use anyhow::{Result, bail};
use camino::Utf8PathBuf;
use nanite_core::{Registry, SourceKind};
use nanite_git::{clone_repo, destination_for, parse_remote};
use nanite_plugins::{Dispatch, ListInput, LogLevel, PluginError, PluginId, RepoEntry, list_repos};
use std::io::{self, IsTerminal};

use crate::context::ContextState;
use crate::repo::clone_progress_display;
use crate::ui::{confirm, confirm_with_listing};

#[derive(Debug, Default)]
pub struct BulkSummary {
    pub cloned: usize,
    pub skipped: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
}

/// Resolves a `BulkProbe` dispatch by asking the relevant plugin to
/// enumerate. Returns the resolved Dispatch along with a pre-fetched
/// listing if it resolved to Bulk (avoids a second API call).
pub fn resolve_probe(plugin: PluginId, target: &str) -> Result<(Dispatch, Option<Vec<RepoEntry>>)> {
    let outcome = list_repos(plugin, fresh_input(target), |_, _| ())?;
    match outcome {
        Ok(entries) if entries.is_empty() => Ok((Dispatch::Single, None)),
        Ok(entries) => Ok((
            Dispatch::Bulk {
                plugin,
                target: target.to_owned(),
            },
            Some(entries),
        )),
        Err(PluginError::NotFound(_)) => Ok((Dispatch::Single, None)),
        Err(err) => Err(plugin_error_into_anyhow(plugin, err)),
    }
}

/// Runs a full bulk-clone flow end-to-end: enumerate via plugin,
/// confirm, prompt for overrides if any destinations exist, clone,
/// register, summarize.
///
/// If `prefetched` is `Some`, it is used as the listing instead of
/// calling the plugin again (used after a probe resolution).
pub fn run_bulk_clone(
    context: &ContextState,
    registry: &mut Registry,
    plugin: PluginId,
    target: &str,
    prefetched: Option<Vec<RepoEntry>>,
) -> Result<BulkSummary> {
    require_tty("bulk clone")?;

    let entries = if let Some(entries) = prefetched {
        entries
    } else {
        let outcome = list_repos(plugin, fresh_input(target), log_plugin_event)?;
        outcome.map_err(|err| plugin_error_into_anyhow(plugin, err))?
    };

    print_plan(context, plugin, target, entries.len());

    if entries.is_empty() {
        println!("nothing to clone");
        return Ok(BulkSummary::default());
    }

    let names: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
    if !confirm_with_listing(
        &format!("Clone all {} repos?", entries.len()),
        &names,
        false,
    )? {
        println!("aborted; nothing cloned");
        return Ok(BulkSummary::default());
    }

    let repos_root = context.workspace_paths.repos_root();
    let existing = scan_existing(repos_root, &entries);
    let force_existing = if existing.is_empty() {
        false
    } else {
        let prompt = if existing.len() == 1 {
            "1 destination already exists. Overwrite it?".to_owned()
        } else {
            format!(
                "{} destinations already exist. Overwrite them?",
                existing.len()
            )
        };
        confirm(&prompt, false)?
    };

    let mut summary = BulkSummary::default();
    let total = entries.len();
    for (idx, entry) in entries.iter().enumerate() {
        let dest_exists = existing
            .iter()
            .any(|p| p == &destination_of(repos_root, &entry.clone_url));
        if dest_exists && !force_existing {
            summary
                .skipped
                .push((entry.clone_url.clone(), "already exists".to_owned()));
            eprintln!(
                "[{}/{total}] skip {} (already exists)",
                idx + 1,
                entry.clone_url
            );
            continue;
        }
        eprintln!("[{}/{total}] cloning {}", idx + 1, entry.clone_url);
        clone_one(
            repos_root,
            registry,
            &entry.clone_url,
            plugin,
            target,
            dest_exists,
            &mut summary,
        );
    }

    print_summary(&summary);
    Ok(summary)
}

fn clone_one(
    repos_root: &camino::Utf8Path,
    registry: &mut Registry,
    clone_url: &str,
    plugin: PluginId,
    target: &str,
    force: bool,
    summary: &mut BulkSummary,
) {
    let progress = match clone_progress_display(clone_url) {
        Ok(p) => p,
        Err(err) => {
            summary
                .failed
                .push((clone_url.to_owned(), format!("progress setup: {err}")));
            return;
        }
    };

    let result = clone_repo(repos_root, clone_url, force, progress.clone());
    drop(progress);

    match result {
        Ok(mut record) => {
            record.source_kind = SourceKind::BulkClone {
                plugin: plugin.as_str().to_owned(),
                target: target.to_owned(),
            };
            registry.upsert(record);
            summary.cloned += 1;
        }
        Err(err) => {
            summary
                .failed
                .push((clone_url.to_owned(), format!("{err:#}")));
        }
    }
}

fn fresh_input(target: &str) -> ListInput {
    ListInput {
        target: target.to_owned(),
        include_archived: false,
        include_forks: false,
        exclude_patterns: Vec::new(),
        page_size: None,
    }
}

fn destination_of(repos_root: &camino::Utf8Path, clone_url: &str) -> Utf8PathBuf {
    parse_remote(clone_url).map_or_else(
        |_| repos_root.join("__unparseable__"),
        |spec| destination_for(repos_root, &spec),
    )
}

fn scan_existing(repos_root: &camino::Utf8Path, entries: &[RepoEntry]) -> Vec<Utf8PathBuf> {
    entries
        .iter()
        .map(|e| destination_of(repos_root, &e.clone_url))
        .filter(|path| path.exists())
        .collect()
}

fn print_plan(context: &ContextState, plugin: PluginId, target: &str, kept: usize) {
    eprintln!(
        "{} plugin matched {target} — {kept} repo{} to clone",
        plugin.as_str(),
        if kept == 1 { "" } else { "s" },
    );
    eprintln!("  destination: {}/", context.workspace_paths.repos_root());
}

fn print_summary(summary: &BulkSummary) {
    println!(
        "cloned {}, skipped {}, failed {}",
        summary.cloned,
        summary.skipped.len(),
        summary.failed.len(),
    );
    for (url, reason) in &summary.skipped {
        println!("  ~ {url} ({reason})");
    }
    for (url, reason) in &summary.failed {
        println!("  x {url} — {reason}");
    }
}

fn require_tty(action: &str) -> Result<()> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        Ok(())
    } else {
        bail!("{action} needs an interactive TTY")
    }
}

fn log_plugin_event(level: LogLevel, message: &str) {
    if matches!(level, LogLevel::Warn | LogLevel::Error) {
        eprintln!("[plugin {level:?}] {message}");
    }
}

fn plugin_error_into_anyhow(plugin: PluginId, err: PluginError) -> anyhow::Error {
    match err {
        PluginError::AuthRequired(msg) => {
            let token_var = match plugin {
                PluginId::GithubOrg => "GITHUB_TOKEN",
                PluginId::GitlabGroup => "GITLAB_TOKEN",
            };
            anyhow::anyhow!(
                "authentication required for {}: {msg}. set the {token_var} environment variable",
                plugin.as_str(),
            )
        }
        PluginError::NotFound(target) => {
            anyhow::anyhow!("{}: {} not found", plugin.as_str(), target)
        }
        PluginError::RateLimited(limit) => {
            let retry = limit
                .retry_after_seconds
                .map_or_else(|| "unknown".to_owned(), |s| format!("{s}s"));
            anyhow::anyhow!(
                "{} rate limited: {} (retry after {retry})",
                plugin.as_str(),
                limit.message,
            )
        }
        PluginError::Network(msg) => {
            anyhow::anyhow!("{} network error: {msg}", plugin.as_str())
        }
        PluginError::Decode(msg) => {
            anyhow::anyhow!("{} decode error: {msg}", plugin.as_str())
        }
        PluginError::Unsupported(msg) => {
            anyhow::anyhow!("{} unsupported: {msg}", plugin.as_str())
        }
        PluginError::Internal(msg) => {
            anyhow::anyhow!("{} internal error: {msg}", plugin.as_str())
        }
    }
}
