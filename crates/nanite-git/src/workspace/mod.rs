mod clone;
mod remove;
mod scan;

pub use clone::{clone_repo, import_repo};
pub use remove::{remove_repo, resolve_repo_remove_target};
pub use scan::{configured_author_name, git_origin, scan_workspace};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use nanite_core::{ProjectRecord, SourceKind};
use std::fs;
use time::OffsetDateTime;

use crate::remote::RemoteSpec;

fn remove_existing_path(path: &Utf8Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("failed to inspect {path}"))?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {path}"))?;
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove {path}"))?;
    }
    Ok(())
}

fn destination_for(workspace_root: &Utf8Path, spec: &RemoteSpec) -> Utf8PathBuf {
    workspace_root.join(&spec.host).join(&spec.repo_path)
}

fn record_from_spec(
    spec: RemoteSpec,
    destination: Utf8PathBuf,
    origin: String,
    source_kind: SourceKind,
) -> ProjectRecord {
    ProjectRecord {
        name: spec.name().to_owned(),
        host: spec.host,
        repo_path: spec.repo_path,
        path: destination,
        origin,
        source_kind,
        last_seen: OffsetDateTime::now_utc(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        clone::prepare_clone_destination, configured_author_name, destination_for, remove_repo,
        scan::relative_spec,
    };
    use crate::remote::RemoteSpec;
    use camino::{Utf8Path, Utf8PathBuf};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn keeps_full_remote_path_depth_in_destination() {
        let destination = destination_for(
            Utf8Path::new("/tmp/workspace"),
            &RemoteSpec {
                host: "github.com".to_owned(),
                repo_path: "platform/team/project".to_owned(),
            },
        );

        assert_eq!(
            destination.as_str(),
            "/tmp/workspace/github.com/platform/team/project"
        );
    }

    #[test]
    fn derives_relative_spec_from_scanned_repo_path() {
        let spec = relative_spec(
            Utf8Path::new("/tmp/workspace"),
            Utf8Path::new("/tmp/workspace/github.com/icepuma/nanite"),
        )
        .unwrap();

        assert_eq!(spec.host, "github.com");
        assert_eq!(spec.repo_path, "icepuma/nanite");
    }

    #[test]
    fn clone_destination_requires_force_to_overwrite() {
        let (_tempdir, destination) = existing_destination();

        let error = prepare_clone_destination(&destination, false)
            .unwrap_err()
            .to_string();

        assert!(error.contains("--force"));
        assert!(destination.exists());
    }

    #[test]
    fn clone_destination_force_removes_existing_directory() {
        let (_tempdir, destination) = existing_destination();

        prepare_clone_destination(&destination, true).unwrap();

        assert!(!destination.exists());
        assert!(destination.parent().unwrap().exists());
    }

    #[test]
    fn remove_repo_prunes_empty_parent_directories() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace_root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let repo = workspace_root.join("github.com/icepuma/nanite");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("README.md"), "fixture\n").unwrap();

        let removed = remove_repo(&workspace_root, "github.com/icepuma/nanite").unwrap();

        assert_eq!(removed, repo);
        assert!(!repo.exists());
        assert!(!workspace_root.join("github.com/icepuma").exists());
        assert!(!workspace_root.join("github.com").exists());
        assert!(workspace_root.exists());
    }

    #[test]
    fn remove_repo_keeps_non_empty_parent_directories() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace_root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let nanite = workspace_root.join("github.com/icepuma/nanite");
        let other = workspace_root.join("github.com/icepuma/other");
        fs::create_dir_all(&nanite).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::write(nanite.join("README.md"), "fixture\n").unwrap();
        fs::write(other.join("README.md"), "fixture\n").unwrap();

        remove_repo(&workspace_root, "github.com/icepuma/nanite").unwrap();

        assert!(!nanite.exists());
        assert!(other.exists());
        assert!(workspace_root.join("github.com/icepuma").exists());
        assert!(workspace_root.join("github.com").exists());
    }

    #[test]
    fn configured_author_name_reads_user_name_from_git_config() {
        let tempdir = tempfile::tempdir().unwrap();
        let repo_path = Utf8PathBuf::from_path_buf(tempdir.path().join("repo")).unwrap();
        let repo = gix::init(repo_path.as_std_path()).unwrap();
        let config_path = Utf8PathBuf::from_path_buf(repo.git_dir().join("config")).unwrap();
        fs::write(
            &config_path,
            "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n\tlogallrefupdates = true\n[user]\n\tname = Jane Doe\n",
        )
        .unwrap();

        let author = configured_author_name(&repo_path).unwrap();

        assert_eq!(author.as_deref(), Some("Jane Doe"));
    }

    fn existing_destination() -> (TempDir, Utf8PathBuf) {
        let tempdir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let destination = root.join("github.com/icepuma/nanite");
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("README.md"), "existing").unwrap();
        (tempdir, destination)
    }
}
