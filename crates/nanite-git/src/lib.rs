#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::uninlined_format_args
)]

mod copy;
mod remote;
mod workspace;

pub use remote::{RemoteSpec, parse_remote};
pub use workspace::{
    clone_repo, configured_author_name, git_origin, import_repo, remove_repo,
    resolve_repo_remove_target, scan_workspace,
};
