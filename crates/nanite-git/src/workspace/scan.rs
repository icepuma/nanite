use crate::remote::{RemoteSpec, parse_remote};
use crate::workspace::record_from_spec;
use anyhow::{Context, Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use gix::config::tree::User;
use nanite_core::{ProjectRecord, SourceKind};
use std::collections::VecDeque;
use std::fs;
use std::process::Command;

pub fn configured_author_name(cwd: &Utf8Path) -> Result<Option<String>> {
    let Ok(repo) = gix::discover(cwd.as_std_path()) else {
        return Ok(None);
    };

    let author = repo
        .config_snapshot()
        .string(User::NAME)
        .map(|value| value.to_string())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    Ok(author)
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

pub fn relative_spec(workspace_root: &Utf8Path, repo_path: &Utf8Path) -> Result<RemoteSpec> {
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
