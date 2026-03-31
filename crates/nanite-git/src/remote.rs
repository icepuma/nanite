use anyhow::{Result, anyhow, bail};
use regex::Regex;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSpec {
    pub host: String,
    pub repo_path: String,
}

impl RemoteSpec {
    #[must_use]
    pub fn name(&self) -> &str {
        self.repo_path
            .rsplit('/')
            .next()
            .unwrap_or(self.repo_path.as_str())
    }
}

/// Parses a git remote into a normalized host and repository path.
///
/// # Errors
///
/// Returns an error when the remote is empty, uses an unsupported format, or
/// contains an invalid repository path.
pub fn parse_remote(remote: &str) -> Result<RemoteSpec> {
    let remote = remote.trim();
    if remote.is_empty() {
        bail!("remote must not be empty");
    }

    if let Ok(url) = Url::parse(remote) {
        return parse_url(&url);
    }

    parse_scp(remote)
}

fn parse_url(url: &Url) -> Result<RemoteSpec> {
    let host = match url.scheme() {
        "file" => url.host_str().unwrap_or("local").to_owned(),
        _ => url
            .host_str()
            .ok_or_else(|| anyhow!("remote URL is missing a host"))?
            .to_owned(),
    };
    let repo_path = normalize_repo_path(url.path())?;

    Ok(RemoteSpec { host, repo_path })
}

fn parse_scp(remote: &str) -> Result<RemoteSpec> {
    let regex = Regex::new(r"^(?:[^@]+@)?(?P<host>[^:]+):(?P<path>.+)$")?;
    let captures = regex
        .captures(remote)
        .ok_or_else(|| anyhow!("unsupported git remote format: {remote}"))?;

    let host = captures
        .name("host")
        .ok_or_else(|| anyhow!("unsupported git remote format: {remote}"))?
        .as_str()
        .to_owned();
    let repo_path = normalize_repo_path(
        captures
            .name("path")
            .ok_or_else(|| anyhow!("unsupported git remote format: {remote}"))?
            .as_str(),
    )?;

    Ok(RemoteSpec { host, repo_path })
}

fn normalize_repo_path(path: &str) -> Result<String> {
    let trimmed = path.trim_start_matches('/').trim_end_matches(".git");
    let segments = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        bail!("git remote path must not be empty");
    }

    for segment in &segments {
        if *segment == "." || *segment == ".." {
            bail!("git remote path contains an invalid segment: {segment}");
        }
    }

    Ok(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::parse_remote;

    #[test]
    fn parses_https_remote() {
        let remote = parse_remote("https://github.com/icepuma/nanite.git").unwrap();

        assert_eq!(remote.host, "github.com");
        assert_eq!(remote.repo_path, "icepuma/nanite");
    }

    #[test]
    fn parses_ssh_remote() {
        let remote = parse_remote("git@github.com:icepuma/nanite.git").unwrap();

        assert_eq!(remote.host, "github.com");
        assert_eq!(remote.repo_path, "icepuma/nanite");
    }

    #[test]
    fn preserves_arbitrary_path_depth() {
        let remote =
            parse_remote("ssh://git@example.com/platform/team/project/nanite.git").unwrap();

        assert_eq!(remote.host, "example.com");
        assert_eq!(remote.repo_path, "platform/team/project/nanite");
    }

    #[test]
    fn parses_file_remote_without_host() {
        let remote = parse_remote("file:///tmp/platform/team/project/nanite.git").unwrap();

        assert_eq!(remote.host, "local");
        assert_eq!(remote.repo_path, "tmp/platform/team/project/nanite");
    }
}
