//! URL classification: decide whether a remote should go through the
//! existing single-repo gix path or be routed to a clone plugin.
//!
//! The heuristic is intentionally narrow:
//!   - `.git`-suffixed inputs are always Single (unambiguous repo URL).
//!   - `github.com/<one-segment>` is Bulk(github-org). Two or more
//!     segments under github.com is Single.
//!   - `gitlab.com/<one-or-more-segments>` is *ambiguous* on URL
//!     alone — `group/sub` looks identical to `group/project`. The
//!     classifier flags this as `BulkProbe` so the caller can ask the
//!     gitlab-group plugin to probe `/api/v4/projects/{path}` and
//!     decide. The probe is wired in `lib.rs`.
//!
//! Anything else is Single. The user can always force with `--bulk`
//! or `--single`.

use nanite_git::RemoteSpec;
use std::path::Path;

use crate::types::PluginId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatch {
    Single,
    Bulk {
        plugin: PluginId,
        target: String,
    },
    /// `gitlab.com/...` — could be a group or a project. The CLI
    /// resolves this via an API probe.
    BulkProbe {
        plugin: PluginId,
        target: String,
    },
}

/// Classifies a parsed remote based purely on its URL shape.
///
/// `original` is the raw user input — used to detect a `.git` suffix
/// that `parse_remote` strips during normalization.
#[must_use]
pub fn detect_bulk_from(spec: &RemoteSpec, original: &str) -> Dispatch {
    let trimmed = original.trim_end_matches('/');
    if Path::new(trimmed)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("git"))
    {
        return Dispatch::Single;
    }

    let host = spec.host.as_str();
    let segments: Vec<&str> = spec
        .repo_path
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    if segments.is_empty() {
        return Dispatch::Single;
    }

    match host {
        "github.com" if segments.len() == 1 => Dispatch::Bulk {
            plugin: PluginId::GithubOrg,
            target: segments[0].to_owned(),
        },
        "gitlab.com" => Dispatch::BulkProbe {
            plugin: PluginId::GitlabGroup,
            target: segments.join("/"),
        },
        _ => Dispatch::Single,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nanite_git::parse_remote;

    fn classify(input: &str) -> Dispatch {
        let spec = parse_remote(input).expect("parse_remote");
        detect_bulk_from(&spec, input)
    }

    #[test]
    fn github_org_url_is_bulk() {
        assert_eq!(
            classify("https://github.com/icepuma"),
            Dispatch::Bulk {
                plugin: PluginId::GithubOrg,
                target: "icepuma".to_owned(),
            },
        );
    }

    #[test]
    fn github_user_repo_url_is_single() {
        assert_eq!(
            classify("https://github.com/icepuma/nanite"),
            Dispatch::Single,
        );
    }

    #[test]
    fn dot_git_suffix_is_always_single() {
        assert_eq!(classify("https://github.com/icepuma.git"), Dispatch::Single);
        assert_eq!(
            classify("https://github.com/icepuma/nanite.git"),
            Dispatch::Single,
        );
    }

    #[test]
    fn ssh_github_org_is_bulk() {
        // Pure `host:path` (no scheme, no `user@`) is read by `url::Url`
        // as `scheme=host, path=path`, which fails `parse_remote` for
        // missing host. The realistic schemeless form is the SCP one
        // with a user prefix.
        assert_eq!(
            classify("git@github.com:icepuma"),
            Dispatch::Bulk {
                plugin: PluginId::GithubOrg,
                target: "icepuma".to_owned(),
            },
        );
    }

    #[test]
    fn ssh_github_repo_is_single() {
        assert_eq!(
            classify("git@github.com:icepuma/nanite.git"),
            Dispatch::Single,
        );
    }

    #[test]
    fn gitlab_single_segment_is_probe() {
        assert_eq!(
            classify("https://gitlab.com/icepuma"),
            Dispatch::BulkProbe {
                plugin: PluginId::GitlabGroup,
                target: "icepuma".to_owned(),
            },
        );
    }

    #[test]
    fn gitlab_nested_path_is_probe() {
        assert_eq!(
            classify("https://gitlab.com/group/sub/leaf"),
            Dispatch::BulkProbe {
                plugin: PluginId::GitlabGroup,
                target: "group/sub/leaf".to_owned(),
            },
        );
    }

    #[test]
    fn other_hosts_are_single() {
        assert_eq!(classify("https://bitbucket.org/icepuma"), Dispatch::Single,);
        assert_eq!(
            classify("https://example.com/some/team/repo"),
            Dispatch::Single,
        );
    }
}
