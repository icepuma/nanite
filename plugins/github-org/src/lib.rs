//! `github-org` clone plugin.
//!
//! Enumerates every repository under a GitHub namespace and returns
//! clone URLs to the nanite host. The host performs the actual git
//! clone via gix.
//!
//! Handles **both organizations and individual users**:
//! `/orgs/{target}/repos` is tried first, and on 404 the plugin falls
//! back to `/users/{target}/repos`. For private repos: org members
//! with a `GITHUB_TOKEN` get private org repos; for user namespaces
//! private repos only come back when the token belongs to that user.

wit_bindgen::generate!({
    path: "../../content/wit/nanite-plugin",
    world: "clone-plugin",
    generate_all,
});

use exports::nanite::plugin::plugin::{Guest, RepoEntry};
use nanite::plugin::host::{self, LogLevel};
use nanite::plugin::types::{ListInput, PluginError, PluginInfo, RateLimit};

const API_BASE: &str = "https://api.github.com";
const MAX_PAGE_SIZE: u32 = 100;
const PLUGIN_NAME: &str = "github-org";

struct GithubOrg;

impl Guest for GithubOrg {
    fn info() -> PluginInfo {
        PluginInfo {
            name: PLUGIN_NAME.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            supported_hosts: vec!["github.com".into()],
        }
    }

    fn list_repos(input: ListInput) -> Result<Vec<RepoEntry>, PluginError> {
        let token = host::get_env("GITHUB_TOKEN");
        let user_agent = plugin_shared::user_agent(PLUGIN_NAME);
        let page_size = input
            .page_size
            .unwrap_or(MAX_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE);

        host::log(LogLevel::Debug, &format!("listing repos for {}", input.target));

        // Try the org endpoint first. On 404 (target isn't an org)
        // fall back to the user endpoint. Anything else propagates.
        let outcome = fetch_namespace(
            "orgs",
            &input,
            page_size,
            token.as_deref(),
            &user_agent,
        );
        let entries = match outcome {
            Ok(entries) => entries,
            Err(PluginError::NotFound(_)) => {
                host::log(
                    LogLevel::Debug,
                    &format!("{} is not an org; trying /users endpoint", input.target),
                );
                fetch_namespace("users", &input, page_size, token.as_deref(), &user_agent)?
            }
            Err(other) => return Err(other),
        };

        host::log(
            LogLevel::Info,
            &format!(
                "github-org found {} repos for {}",
                entries.len(),
                input.target
            ),
        );

        Ok(entries)
    }
}

fn fetch_namespace(
    kind: &str,
    input: &ListInput,
    page_size: u32,
    token: Option<&str>,
    user_agent: &str,
) -> Result<Vec<RepoEntry>, PluginError> {
    let mut out: Vec<RepoEntry> = Vec::new();
    let mut page: u32 = 1;
    loop {
        let url = format!(
            "{API_BASE}/{kind}/{target}/repos?per_page={page_size}&page={page}&type=all",
            target = input.target,
        );
        let mut request = waki::Client::new()
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", user_agent);
        if let Some(t) = token {
            let value = format!("Bearer {t}");
            request = request.header("Authorization", &value);
        }

        let response = request.send().map_err(network)?;
        let status = response.status_code();
        let link_header = response
            .headers()
            .get("link")
            .or_else(|| response.headers().get("Link"))
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = response.body().map_err(decode)?;

        match status {
            200 => {}
            401 => {
                return Err(PluginError::AuthRequired(
                    "GITHUB_TOKEN is missing or invalid".into(),
                ));
            }
            403 => {
                return Err(classify_403(&body));
            }
            404 => {
                return Err(PluginError::NotFound(input.target.clone()));
            }
            429 => {
                return Err(PluginError::RateLimited(RateLimit {
                    retry_after_seconds: None,
                    message: "GitHub returned 429 Too Many Requests".into(),
                }));
            }
            code => {
                return Err(PluginError::Network(format!("GitHub returned HTTP {code}")));
            }
        }

        let repos: Vec<ApiRepo> = serde_json::from_slice(&body)
            .map_err(|e| PluginError::Decode(format!("failed to decode repos page {page}: {e}")))?;
        let page_len = repos.len();
        for repo in repos {
            if !input.include_archived && repo.archived {
                continue;
            }
            if !input.include_forks && repo.fork {
                continue;
            }
            out.push(RepoEntry {
                name: repo.name,
                clone_url: repo.clone_url,
                default_branch: repo.default_branch,
                archived: repo.archived,
                fork: repo.fork,
                description: repo.description,
            });
        }

        if !plugin_shared::has_next_link(link_header.as_deref()) || page_len == 0 {
            break;
        }
        page = page.checked_add(1).unwrap_or(u32::MAX);
        if page == u32::MAX {
            break;
        }
    }
    Ok(out)
}

#[derive(serde::Deserialize)]
struct ApiRepo {
    name: String,
    clone_url: String,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    description: Option<String>,
}

fn network<E: core::fmt::Display>(err: E) -> PluginError {
    PluginError::Network(err.to_string())
}

fn decode<E: core::fmt::Display>(err: E) -> PluginError {
    PluginError::Decode(err.to_string())
}

fn classify_403(body: &[u8]) -> PluginError {
    let body_text = core::str::from_utf8(body).unwrap_or("");
    if body_text.contains("rate limit") || body_text.contains("API rate limit") {
        return PluginError::RateLimited(RateLimit {
            retry_after_seconds: None,
            message: "GitHub API rate limit exceeded".into(),
        });
    }
    PluginError::AuthRequired(
        "GitHub returned 403 — token lacks permission to read this namespace's repos".into(),
    )
}

export!(GithubOrg);
