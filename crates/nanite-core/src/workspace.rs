use camino::{Utf8Path, Utf8PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePaths {
    root: Utf8PathBuf,
    repos_root: Utf8PathBuf,
}

impl WorkspacePaths {
    #[must_use]
    pub fn new(root: Utf8PathBuf) -> Self {
        Self {
            repos_root: root.join("repos"),
            root,
        }
    }

    #[must_use]
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    #[must_use]
    pub fn repos_root(&self) -> &Utf8Path {
        &self.repos_root
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspacePaths;
    use camino::Utf8PathBuf;

    #[test]
    fn derives_fixed_workspace_layout() {
        let paths = WorkspacePaths::new(Utf8PathBuf::from("/tmp/nanite"));

        assert_eq!(paths.root().as_str(), "/tmp/nanite");
        assert_eq!(paths.repos_root().as_str(), "/tmp/nanite/repos");
    }
}
