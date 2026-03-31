use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillProvider {
    Codex,
    Claude,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub providers: ProviderOverrides,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProviderOverrides {
    #[serde(default)]
    pub claude: ClaudeOverrides,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ClaudeOverrides {
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalSkill {
    pub slug: String,
    pub metadata: SkillMetadata,
    pub body: String,
    pub raw_document: String,
    pub resources: BTreeMap<Utf8PathBuf, Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    Create,
    Override,
    Unchanged,
}

impl SyncAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Override => "override",
            Self::Unchanged => "unchanged",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub added: Vec<Utf8PathBuf>,
    pub changed: Vec<Utf8PathBuf>,
    pub removed: Vec<Utf8PathBuf>,
}

impl FileDiff {
    pub(crate) fn from_rendered(rendered: &BTreeMap<Utf8PathBuf, Vec<u8>>) -> Self {
        Self {
            added: rendered.keys().cloned().collect(),
            changed: Vec::new(),
            removed: Vec::new(),
        }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncReason {
    Missing { diff: FileDiff },
    ContentChanged { diff: FileDiff },
    WrongSymlink { expected: String, actual: String },
    NotSymlink,
    NotDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncTarget {
    pub path: Utf8PathBuf,
    pub reasons: Vec<SyncReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncItem {
    pub slug: String,
    pub action: SyncAction,
    pub targets: Vec<SyncTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub items: Vec<SyncItem>,
}
