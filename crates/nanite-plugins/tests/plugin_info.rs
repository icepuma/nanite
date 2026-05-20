//! ABI smoke test: instantiates each bundled plugin via the real
//! wasmtime runtime and calls `info()`. This is the first thing that
//! breaks when wasmtime/wit-bindgen/wasi-http versions drift apart;
//! catching it here is much cheaper than discovering it at runtime.

use nanite_plugins::{PluginId, plugin_info};

#[test]
fn github_org_plugin_reports_info() {
    let info = plugin_info(PluginId::GithubOrg).expect("instantiate github-org plugin");
    assert_eq!(info.name, "github-org");
    assert_eq!(info.supported_hosts, vec!["github.com".to_owned()]);
    assert!(!info.version.is_empty(), "version must not be empty");
}

#[test]
fn gitlab_group_plugin_reports_info() {
    let info = plugin_info(PluginId::GitlabGroup).expect("instantiate gitlab-group plugin");
    assert_eq!(info.name, "gitlab-group");
    assert_eq!(info.supported_hosts, vec!["gitlab.com".to_owned()]);
    assert!(!info.version.is_empty(), "version must not be empty");
}
