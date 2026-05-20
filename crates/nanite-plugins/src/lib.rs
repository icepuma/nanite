//! Host runtime for nanite's WASM clone plugins.
//!
//! The CLI hands a parsed remote URL to [`detect_bulk_from`]; if the
//! result is [`Dispatch::Bulk`] or [`Dispatch::BulkProbe`], the CLI
//! calls [`list_repos`] to enumerate the org/group via the bundled
//! WASM plugin and then clones each entry with `nanite_git`.

mod bundled;
mod dispatch;
mod host_impl;
mod runtime;
mod types;

pub use dispatch::{Dispatch, detect_bulk_from};
pub use types::{ListInput, LogLevel, PluginError, PluginId, PluginInfo, RateLimit, RepoEntry};

use anyhow::{Context, Result};

use crate::runtime::{HostState, PluginRuntime};

const fn wasm_for(plugin: PluginId) -> &'static [u8] {
    match plugin {
        PluginId::GithubOrg => bundled::GITHUB_ORG_WASM,
        PluginId::GitlabGroup => bundled::GITLAB_GROUP_WASM,
    }
}

/// Calls the bundled plugin's `info()` export. Cheap; used by tests
/// and any future `nanite plugins list` command.
///
/// # Errors
///
/// Returns an error if the wasm module fails to instantiate.
pub fn plugin_info(plugin: PluginId) -> Result<PluginInfo> {
    let runtime = PluginRuntime::new().context("build plugin runtime")?;
    let state = HostState::new(plugin.env_allowlist().to_vec(), Box::new(|_, _| ()));
    runtime.run_info(wasm_for(plugin), state)
}

/// Enumerates repositories for the given bulk-clone target.
///
/// `log_sink` receives any structured log lines the plugin emits via
/// `nanite:plugin/host.log`. The CLI routes these into its bulk
/// progress UI.
///
/// # Errors
///
/// Returns an error when the wasm fails to instantiate or traps. A
/// structured plugin error (auth-required, rate-limited, etc.) is
/// returned as `Ok(Err(PluginError))` so the caller can match on it.
pub fn list_repos<F>(
    plugin: PluginId,
    input: ListInput,
    log_sink: F,
) -> Result<Result<Vec<RepoEntry>, PluginError>>
where
    F: Fn(LogLevel, &str) + Send + Sync + 'static,
{
    let runtime = PluginRuntime::new().context("build plugin runtime")?;
    let state = HostState::new(plugin.env_allowlist().to_vec(), Box::new(log_sink));
    runtime.run_list_repos(wasm_for(plugin), state, input)
}
