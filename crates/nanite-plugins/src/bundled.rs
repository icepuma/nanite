//! Compiled wasm artifacts for the first-party plugins, embedded into
//! the nanite binary at build time. The build script asserts these
//! exist; running `just build-plugins` refreshes them.

pub const GITHUB_ORG_WASM: &[u8] = include_bytes!("../../../content/plugins/github-org.wasm");

pub const GITLAB_GROUP_WASM: &[u8] = include_bytes!("../../../content/plugins/gitlab-group.wasm");
