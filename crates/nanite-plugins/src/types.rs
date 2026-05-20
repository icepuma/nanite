//! Host-side mirrors of the WIT types and conversions to/from the
//! generated `wasmtime::component::bindgen!` representations.
//!
//! The generated types are tied to the macro expansion site, so we
//! keep them internal to `runtime.rs` and convert to these public
//! types at the API boundary.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginId {
    GithubOrg,
    GitlabGroup,
}

impl PluginId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GithubOrg => "github-org",
            Self::GitlabGroup => "gitlab-group",
        }
    }

    /// Environment variables the host will expose to the plugin via
    /// the `nanite:plugin/host.get-env` import. Anything outside this
    /// list is invisible to the guest even if set in the host process.
    #[must_use]
    pub const fn env_allowlist(self) -> &'static [&'static str] {
        match self {
            Self::GithubOrg => &["GITHUB_TOKEN"],
            Self::GitlabGroup => &["GITLAB_TOKEN"],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListInput {
    pub target: String,
    pub include_archived: bool,
    pub include_forks: bool,
    pub exclude_patterns: Vec<String>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    pub name: String,
    pub clone_url: String,
    pub default_branch: Option<String>,
    pub archived: bool,
    pub fork: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub supported_hosts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RateLimit {
    pub retry_after_seconds: Option<u32>,
    pub message: String,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PluginError {
    #[error("authentication required: {0}")]
    AuthRequired(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("rate limited: {}", .0.message)]
    RateLimited(RateLimit),
    #[error("network error: {0}")]
    Network(String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("internal plugin error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
