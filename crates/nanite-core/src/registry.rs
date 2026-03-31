use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use time::OffsetDateTime;
use time::serde::rfc3339;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRecord {
    pub name: String,
    pub host: String,
    pub repo_path: String,
    pub path: Utf8PathBuf,
    pub origin: String,
    pub source_kind: SourceKind,
    #[serde(with = "rfc3339")]
    pub last_seen: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Clone,
    Import,
    Scan,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registry {
    projects: BTreeMap<String, ProjectRecord>,
}

impl Registry {
    /// Loads the project registry from disk.
    ///
    /// # Errors
    ///
    /// Returns an error when the registry file exists but cannot be read or
    /// parsed as JSON.
    pub fn load(path: &Utf8Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
        serde_json::from_str(&raw).with_context(|| format!("failed to parse {path}"))
    }

    /// Saves the project registry to disk.
    ///
    /// # Errors
    ///
    /// Returns an error when the parent directory cannot be created, the
    /// registry cannot be serialized, or the destination file cannot be written.
    pub fn save(&self, path: &Utf8Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
        }

        let raw = serde_json::to_string_pretty(self)?;
        fs::write(path, raw).with_context(|| format!("failed to write {path}"))?;
        Ok(())
    }

    pub fn upsert(&mut self, record: ProjectRecord) {
        self.projects.insert(record.path.to_string(), record);
    }

    pub fn remove_path(&mut self, path: &Utf8Path) -> Option<ProjectRecord> {
        self.projects.remove(path.as_str())
    }

    #[must_use]
    pub fn entries(&self) -> Vec<&ProjectRecord> {
        self.projects.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{ProjectRecord, Registry, SourceKind};
    use camino::Utf8PathBuf;
    use time::macros::datetime;

    #[test]
    fn serializes_registry_entries() {
        let mut registry = Registry::default();
        registry.upsert(ProjectRecord {
            name: "nanite".to_owned(),
            host: "github.com".to_owned(),
            repo_path: "icepuma/nanite".to_owned(),
            path: Utf8PathBuf::from("/tmp/workspace/github.com/icepuma/nanite"),
            origin: "https://github.com/icepuma/nanite.git".to_owned(),
            source_kind: SourceKind::Clone,
            last_seen: datetime!(2026-03-23 00:00:00 UTC),
        });

        let raw = serde_json::to_string(&registry).unwrap();

        assert!(raw.contains("github.com"));
        assert!(raw.contains("icepuma/nanite"));
    }
}
