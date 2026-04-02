mod copy;
mod remote;
mod workspace;

pub use remote::{RemoteSpec, parse_remote};
pub use workspace::{
    CloneProgressDisplay, SharedCloneProgressDisplay, clone_repo, configured_author_email,
    configured_author_name, git_origin, import_repo, remove_repo, resolve_repo_remove_target,
    scan_workspace,
};
