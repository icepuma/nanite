use crate::copy::copy_dir_recursive;
use crate::remote::parse_remote;
use crate::workspace::{
    SharedCloneProgressDisplay, destination_for, record_from_spec, remove_existing_path,
};
use anyhow::{Context, Result, anyhow, bail};
use camino::Utf8Path;
use nanite_core::{ProjectRecord, SourceKind};
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Clones a git remote into the Nanite workspace.
///
/// # Errors
///
/// Returns an error when the remote cannot be parsed, the destination cannot be
/// prepared, or the clone itself fails.
pub fn clone_repo(
    workspace_root: &Utf8Path,
    remote: &str,
    force: bool,
    progress_display: Option<SharedCloneProgressDisplay>,
) -> Result<ProjectRecord> {
    let spec = parse_remote(remote)?;
    let destination = destination_for(workspace_root, &spec);
    prepare_clone_destination(&destination, force)?;

    let should_interrupt = AtomicBool::new(false);
    if let Some(progress_display) = progress_display {
        let mut progress = CloneProgress::new(progress_display);
        perform_clone(remote, &destination, &mut progress, &should_interrupt)?;
    } else {
        let mut progress = gix::progress::Discard;
        perform_clone(remote, &destination, &mut progress, &should_interrupt)?;
    }

    Ok(record_from_spec(
        spec,
        destination,
        remote.to_owned(),
        SourceKind::Clone,
    ))
}

/// Imports an existing local repository into the Nanite workspace.
///
/// # Errors
///
/// Returns an error when the source path is invalid, the repository metadata
/// cannot be derived, or the repository tree cannot be copied.
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

    let origin_remote = crate::workspace::git_origin(git_binary, source)?;
    let spec = if let Some(remote) = origin_remote.as_deref() {
        parse_remote(remote)?
    } else {
        crate::remote::RemoteSpec {
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
    fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    copy_dir_recursive(source, &destination)?;

    Ok(record_from_spec(
        spec,
        destination,
        origin_remote.unwrap_or_else(|| source.to_owned().into_string()),
        SourceKind::Import,
    ))
}

/// Prepares a clone destination, optionally removing an existing directory first.
///
/// # Errors
///
/// Returns an error when an existing destination is not allowed, cannot be
/// removed, or the destination parent directory cannot be created.
pub(super) fn prepare_clone_destination(destination: &Utf8Path, force: bool) -> Result<()> {
    if destination.exists() {
        if !force {
            bail!("{destination} already exists; rerun with --force to overwrite");
        }
        remove_existing_path(destination)?;
    }

    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("failed to determine clone destination for {destination}"))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
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
    display: SharedCloneProgressDisplay,
    counter: Arc<AtomicUsize>,
    name: Arc<Mutex<Option<String>>>,
}

impl CloneProgress {
    fn new(display: SharedCloneProgressDisplay) -> Self {
        Self {
            display,
            counter: Arc::new(AtomicUsize::default()),
            name: Arc::new(Mutex::new(None)),
        }
    }

    fn refresh_message(&self, message: Option<&str>) {
        let current = self.counter.load(Ordering::Relaxed) as u64;
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
        if let Ok(mut display) = self.display.lock() {
            display.set_position(usize::try_from(current).unwrap_or(usize::MAX));
            display.set_message(&rendered);
        }
    }
}

impl gix::Count for CloneProgress {
    fn set(&self, step: usize) {
        self.counter.store(step, Ordering::Relaxed);
        if let Ok(mut display) = self.display.lock() {
            display.set_position(step);
        }
    }

    fn step(&self) -> usize {
        self.counter.load(Ordering::Relaxed)
    }

    fn inc_by(&self, step: usize) {
        let next = self.counter.fetch_add(step, Ordering::Relaxed) + step;
        if let Ok(mut display) = self.display.lock() {
            display.set_position(next);
        }
    }

    fn counter(&self) -> gix::progress::StepShared {
        self.counter.clone()
    }
}

impl gix::Progress for CloneProgress {
    fn init(&mut self, max: Option<usize>, _unit: Option<gix::progress::Unit>) {
        if let Ok(mut display) = self.display.lock() {
            display.set_total(max);
        }
    }

    fn set_max(&mut self, max: Option<usize>) -> Option<usize> {
        if let Ok(mut display) = self.display.lock() {
            display.set_total(max);
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
