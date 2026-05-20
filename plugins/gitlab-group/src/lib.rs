//! `gitlab-group` clone plugin.
//!
//! Enumerates every project in a GitLab group, recursively walking
//! subgroups (`include_subgroups=true`), and returns clone URLs to the
//! nanite host. The host performs the actual git clone via gix.
//!
//! Auth: optional `GITLAB_TOKEN` (PRIVATE-TOKEN header). Required for
//! private groups and subgroups.

wit_bindgen::generate!({
    path: "../../content/wit/nanite-plugin",
    world: "clone-plugin",
    generate_all,
});

use exports::nanite::plugin::plugin::{Guest, RepoEntry};
use nanite::plugin::host::{self, LogLevel};
use nanite::plugin::types::{ListInput, PluginError, PluginInfo, RateLimit};

const API_BASE: &str = "https://gitlab.com/api/v4";
const MAX_PAGE_SIZE: u32 = 100;
const PLUGIN_NAME: &str = "gitlab-group";

struct GitlabGroup;

impl Guest for GitlabGroup {
    fn info() -> PluginInfo {
        PluginInfo {
            name: PLUGIN_NAME.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            supported_hosts: vec!["gitlab.com".into()],
        }
    }

    fn list_repos(input: ListInput) -> Result<Vec<RepoEntry>, PluginError> {
        let token = host::get_env("GITLAB_TOKEN");
        let user_agent = plugin_shared::user_agent(PLUGIN_NAME);
        let page_size = input
            .page_size
            .unwrap_or(MAX_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE);
        let encoded_target = plugin_shared::encode_path_segment(&input.target);

        host::log(
            LogLevel::Debug,
            &format!("listing projects for group {}", input.target),
        );

        let mut out: Vec<RepoEntry> = Vec::new();
        let mut page: u32 = 1;
        loop {
            let url = format!(
                "{API_BASE}/groups/{encoded_target}/projects?include_subgroups=true&per_page={page_size}&page={page}&archived={archived}",
                archived = input.include_archived,
            );
            let mut request = waki::Client::new()
                .get(&url)
                .header("Accept", "application/json")
                .header("User-Agent", &user_agent);
            if let Some(ref t) = token {
                request = request.header("PRIVATE-TOKEN", t);
            }

            let response = request.send().map_err(network)?;
            let status = response.status_code();
            let next_page_header = response
                .headers()
                .get("x-next-page")
                .or_else(|| response.headers().get("X-Next-Page"))
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let body = response.body().map_err(decode)?;

            match status {
                200 => {}
                401 => {
                    return Err(PluginError::AuthRequired(
                        "GITLAB_TOKEN is missing or invalid".into(),
                    ));
                }
                403 => {
                    return Err(PluginError::AuthRequired(
                        "GitLab returned 403 — token lacks permission to read this group's projects"
                            .into(),
                    ));
                }
                404 => {
                    return Err(PluginError::NotFound(input.target.clone()));
                }
                429 => {
                    return Err(PluginError::RateLimited(RateLimit {
                        retry_after_seconds: None,
                        message: "GitLab returned 429 Too Many Requests".into(),
                    }));
                }
                code => {
                    return Err(PluginError::Network(format!("GitLab returned HTTP {code}")));
                }
            }

            let projects: Vec<ApiProject> = serde_json::from_slice(&body).map_err(|e| {
                PluginError::Decode(format!("failed to decode projects page {page}: {e}"))
            })?;
            let page_len = projects.len();
            for project in projects {
                let fork = project.forked_from_project.is_some();
                if !input.include_forks && fork {
                    continue;
                }
                out.push(RepoEntry {
                    name: project.path,
                    clone_url: project.http_url_to_repo,
                    default_branch: project.default_branch,
                    archived: project.archived,
                    fork,
                    description: project.description,
                });
            }

            let next_page_present = next_page_header
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !next_page_present || page_len == 0 {
                break;
            }
            page = page.checked_add(1).unwrap_or(u32::MAX);
            if page == u32::MAX {
                break;
            }
        }

        host::log(
            LogLevel::Info,
            &format!(
                "gitlab-group found {} projects for {}",
                out.len(),
                input.target
            ),
        );

        Ok(out)
    }
}

#[derive(serde::Deserialize)]
struct ApiProject {
    path: String,
    http_url_to_repo: String,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    forked_from_project: Option<serde_json::Value>,
}

fn network<E: core::fmt::Display>(err: E) -> PluginError {
    PluginError::Network(err.to_string())
}

fn decode<E: core::fmt::Display>(err: E) -> PluginError {
    PluginError::Decode(err.to_string())
}

export!(GitlabGroup);
