use crate::copy::copy_dir_recursive;
use crate::remote::{RemoteSpec, parse_remote};
use anyhow::{Context, Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use gix::config::tree::User;
use indicatif::ProgressBar;
use nanite_core::{ProjectRecord, SourceKind};
use std::collections::VecDeque;
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use time::OffsetDateTime;

pub fn clone_repo(
    workspace_root: &Utf8Path,
    remote: &str,
    force: bool,
    progress_bar: Option<ProgressBar>,
) -> Result<ProjectRecord> {
    let spec = parse_remote(remote)?;
    let destination = destination_for(workspace_root, &spec);
    prepare_clone_destination(&destination, force)?;

    let should_interrupt = AtomicBool::new(false);
    match progress_bar {
        Some(progress_bar) => {
            let mut progress = CloneProgress::new(progress_bar);
            perform_clone(remote, &destination, &mut progress, &should_interrupt)?;
        }
        None => {
            let mut progress = gix::progress::Discard;
            perform_clone(remote, &destination, &mut progress, &should_interrupt)?;
        }
    }

    Ok(record_from_spec(
        spec,
        destination,
        remote.to_owned(),
        SourceKind::Clone,
    ))
}

fn prepare_clone_destination(destination: &Utf8Path, force: bool) -> Result<()> {
    if destination.exists() {
        if !force {
            bail!("{destination} already exists; rerun with --force to overwrite");
        }
        remove_existing_path(destination)?;
    }

    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("failed to determine clone destination for {destination}"))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent))?;
    Ok(())
}

fn remove_existing_path(path: &Utf8Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("failed to inspect {}", path))?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path))?;
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path))?;
    }
    Ok(())
}

fn perform_clone<P: gix::NestedProgress>(
    remote: &str,
    destination: &Utf8Path,
    progress: &mut P,
    should_interrupt: &AtomicBool,
) -> Result<()>
where
    P::SubProgress: 'static,
{
    let (mut checkout, _fetch_outcome) = gix::prepare_clone(remote, destination.as_std_path())
        .with_context(|| format!("failed to prepare clone of {remote} into {destination}"))?
        .with_remote_name("origin")
        .context("failed to configure clone remote name")?
        .fetch_then_checkout(&mut *progress, should_interrupt)
        .with_context(|| format!("failed to fetch {remote}"))?;
    let (_repo, _checkout_outcome) = checkout
        .main_worktree(&mut *progress, should_interrupt)
        .with_context(|| format!("failed to checkout clone at {destination}"))?;
    Ok(())
}

#[derive(Clone)]
struct CloneProgress {
    bar: ProgressBar,
    counter: Arc<AtomicUsize>,
    name: Arc<Mutex<Option<String>>>,
}

impl CloneProgress {
    fn new(bar: ProgressBar) -> Self {
        bar.enable_steady_tick(Duration::from_millis(100));
        Self {
            bar,
            counter: Arc::new(AtomicUsize::default()),
            name: Arc::new(Mutex::new(None)),
        }
    }

    fn refresh_message(&self, message: Option<&str>) {
        let current = self.counter.load(Ordering::Relaxed) as u64;
        self.bar.set_position(current);
        let name = self.name.lock().ok().and_then(|guard| guard.clone());
        let rendered = match (
            name.as_deref().filter(|value| !value.is_empty()),
            message.filter(|value| !value.is_empty()),
        ) {
            (Some(name), Some(message)) => format!("{name}: {message}"),
            (Some(name), None) => name.to_owned(),
            (None, Some(message)) => message.to_owned(),
            (None, None) => "cloning".to_owned(),
        };
        self.bar.set_message(rendered);
    }
}

impl gix::Count for CloneProgress {
    fn set(&self, step: usize) {
        self.counter.store(step, Ordering::Relaxed);
        self.bar.set_position(step as u64);
    }

    fn step(&self) -> usize {
        self.counter.load(Ordering::Relaxed)
    }

    fn inc_by(&self, step: usize) {
        let next = self.counter.fetch_add(step, Ordering::Relaxed) + step;
        self.bar.set_position(next as u64);
    }

    fn counter(&self) -> gix::progress::StepShared {
        self.counter.clone()
    }
}

impl gix::Progress for CloneProgress {
    fn init(&mut self, max: Option<usize>, _unit: Option<gix::progress::Unit>) {
        if let Some(max) = max {
            self.bar.set_length(max as u64);
        }
    }

    fn set_max(&mut self, max: Option<usize>) -> Option<usize> {
        if let Some(max) = max {
            self.bar.set_length(max as u64);
        }
        None
    }

    fn set_name(&mut self, name: String) {
        if let Ok(mut current) = self.name.lock() {
            *current = Some(name);
        }
        self.refresh_message(None);
    }

    fn name(&self) -> Option<String> {
        self.name.lock().ok().and_then(|guard| guard.clone())
    }

    fn id(&self) -> gix::progress::Id {
        gix::progress::UNKNOWN
    }

    fn message(&self, _level: gix::progress::MessageLevel, message: String) {
        self.refresh_message(Some(&message));
    }
}

impl gix::NestedProgress for CloneProgress {
    type SubProgress = Self;

    fn add_child(&mut self, name: impl Into<String>) -> Self::SubProgress {
        let child = self.clone();
        if let Ok(mut current) = child.name.lock() {
            *current = Some(name.into());
        }
        child.refresh_message(None);
        child
    }

    fn add_child_with_id(
        &mut self,
        name: impl Into<String>,
        _id: gix::progress::Id,
    ) -> Self::SubProgress {
        let child = self.clone();
        if let Ok(mut current) = child.name.lock() {
            *current = Some(name.into());
        }
        child.refresh_message(None);
        child
    }
}

pub fn import_repo(
    workspace_root: &Utf8Path,
    source: &Utf8Path,
    git_binary: &str,
) -> Result<ProjectRecord> {
    if !source.exists() {
        bail!("{source} does not exist");
    }
    if !source.is_dir() {
        bail!("{source} is not a directory");
    }

    let origin_remote = git_origin(git_binary, source)?;
    let spec = if let Some(remote) = origin_remote.as_deref() {
        parse_remote(remote)?
    } else {
        RemoteSpec {
            host: "local".to_owned(),
            repo_path: source
                .file_name()
                .ok_or_else(|| anyhow!("{source} does not have a valid basename"))?
                .to_owned(),
        }
    };

    let destination = destination_for(workspace_root, &spec);
    if destination.exists() {
        bail!("{destination} already exists");
    }

    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("failed to determine import destination for {destination}"))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent))?;
    copy_dir_recursive(source, &destination)?;

    Ok(record_from_spec(
        spec,
        destination,
        origin_remote.unwrap_or_else(|| source.to_owned().into_string()),
        SourceKind::Import,
    ))
}

pub fn remove_repo(workspace_root: &Utf8Path, target: &str) -> Result<Utf8PathBuf> {
    let destination = resolve_remove_destination(workspace_root, target)?;
    if !destination.exists() {
        bail!("{destination} does not exist");
    }
    if destination == workspace_root {
        bail!("refusing to remove workspace root {workspace_root}");
    }

    remove_existing_path(&destination)?;
    prune_empty_repo_parents(workspace_root, &destination)?;
    Ok(destination)
}

pub fn configured_author_name(cwd: &Utf8Path) -> Result<Option<String>> {
    let repo = match gix::discover(cwd.as_std_path()) {
        Ok(repo) => repo,
        Err(_) => return Ok(None),
    };

    let author = repo
        .config_snapshot()
        .string(User::NAME)
        .map(|value| value.to_string())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    Ok(author)
}

pub fn resolve_repo_remove_target(workspace_root: &Utf8Path, target: &str) -> Result<Utf8PathBuf> {
    resolve_remove_destination(workspace_root, target)
}

pub fn scan_workspace(git_binary: &str, workspace_root: &Utf8Path) -> Result<Vec<ProjectRecord>> {
    let repositories = discover_git_repositories(workspace_root)?;
    repositories
        .into_iter()
        .map(|path| {
            let origin = git_origin(git_binary, &path)?;
            let spec = if let Some(remote) = origin.as_deref() {
                parse_remote(remote)?
            } else {
                relative_spec(workspace_root, &path)?
            };

            Ok(record_from_spec(
                spec,
                path.clone(),
                origin.unwrap_or_else(|| path.to_string()),
                SourceKind::Scan,
            ))
        })
        .collect()
}

pub fn git_origin(git_binary: &str, repo_path: &Utf8Path) -> Result<Option<String>> {
    let output = Command::new(git_binary)
        .args(["-C", repo_path.as_str(), "remote", "get-url", "origin"])
        .output()
        .with_context(|| format!("failed to spawn {git_binary}"))?;
    if !output.status.success() {
        return Ok(None);
    }

    let remote = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if remote.is_empty() {
        return Ok(None);
    }

    Ok(Some(remote))
}

fn discover_git_repositories(workspace_root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let mut queue = VecDeque::from([workspace_root.to_owned()]);
    let mut repositories = Vec::new();

    while let Some(directory) = queue.pop_front() {
        if directory.join(".git").exists() {
            repositories.push(directory);
            continue;
        }

        for entry in
            fs::read_dir(&directory).with_context(|| format!("failed to read {}", directory))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if !file_type.is_dir() {
                continue;
            }

            let path = Utf8PathBuf::from_path_buf(entry.path())
                .map_err(|path| anyhow!("non-UTF-8 path encountered: {}", path.display()))?;
            if path.file_name() == Some(".git") {
                continue;
            }
            queue.push_back(path);
        }
    }

    Ok(repositories)
}

fn relative_spec(workspace_root: &Utf8Path, repo_path: &Utf8Path) -> Result<RemoteSpec> {
    let relative = repo_path
        .strip_prefix(workspace_root)
        .map_err(|_| anyhow!("{repo_path} is not inside {workspace_root}"))?;
    let segments = relative.iter().collect::<Vec<_>>();
    if segments.is_empty() {
        bail!("failed to derive a repo path for {repo_path}");
    }

    if segments.len() == 1 {
        return Ok(RemoteSpec {
            host: "local".to_owned(),
            repo_path: segments[0].to_owned(),
        });
    }

    Ok(RemoteSpec {
        host: segments[0].to_owned(),
        repo_path: segments[1..].join("/"),
    })
}

fn resolve_remove_destination(workspace_root: &Utf8Path, target: &str) -> Result<Utf8PathBuf> {
    let target = target.trim();
    if target.is_empty() {
        bail!("repo remove target must not be empty");
    }

    if let Ok(spec) = parse_remote(target) {
        return Ok(destination_for(workspace_root, &spec));
    }

    let path = Utf8Path::new(target);
    if path.is_absolute() {
        let absolute = path.to_owned();
        relative_spec(workspace_root, &absolute)?;
        return Ok(absolute);
    }

    let spec = parse_workspace_relative_spec(target)?;
    Ok(destination_for(workspace_root, &spec))
}

fn parse_workspace_relative_spec(target: &str) -> Result<RemoteSpec> {
    let trimmed = target.trim().trim_start_matches('/').trim_end_matches('/');
    let segments = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.len() < 2 {
        bail!("workspace repo targets must look like <host>/<repo-path>");
    }
    for segment in &segments {
        if *segment == "." || *segment == ".." {
            bail!("workspace repo target contains an invalid segment: {segment}");
        }
    }

    Ok(RemoteSpec {
        host: segments[0].to_owned(),
        repo_path: segments[1..].join("/"),
    })
}

fn prune_empty_repo_parents(workspace_root: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    let mut current = destination.parent();
    while let Some(parent) = current {
        if parent == workspace_root {
            break;
        }
        if !parent.starts_with(workspace_root) {
            bail!("{parent} is not inside {workspace_root}");
        }
        if !directory_is_empty(parent)? {
            break;
        }
        fs::remove_dir(parent).with_context(|| format!("failed to remove empty {}", parent))?;
        current = parent.parent();
    }
    Ok(())
}

fn directory_is_empty(path: &Utf8Path) -> Result<bool> {
    let mut entries =
        fs::read_dir(path).with_context(|| format!("failed to read directory {}", path))?;
    Ok(entries.next().is_none())
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
        configured_author_name, destination_for, prepare_clone_destination, relative_spec,
        remove_repo,
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
