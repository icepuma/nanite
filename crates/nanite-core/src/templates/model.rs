use camino::Utf8PathBuf;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TemplateMetadata {
    pub filename: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPlaceholder {
    pub name: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiPlaceholder {
    pub index: usize,
    pub prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadmeFragmentRole {
    Badges,
    Overview,
    QuickStart,
    Usage,
    Tests,
}

impl ReadmeFragmentRole {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Badges => "Badges",
            Self::Overview => "Overview",
            Self::QuickStart => "Quick Start",
            Self::Usage => "Usage",
            Self::Tests => "Tests",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiFragment {
    pub placeholder: AiPlaceholder,
    pub label: String,
    pub readme_role: Option<ReadmeFragmentRole>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateFragment {
    Literal(String),
    Text(TextPlaceholder),
    Expression(String),
    Ai(AiPlaceholder),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSnippet {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoContextFacts {
    pub repo_name: String,
    pub repo_shape: String,
    pub ci_workflows: Vec<String>,
    pub license_source: Option<String>,
    pub bootstrap_command: Option<String>,
    pub run_command: Option<String>,
    pub test_command: Option<String>,
    pub docs_present: Vec<String>,
    pub workspace_inventory: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBundle {
    pub facts: RepoContextFacts,
    pub summary_lines: Vec<String>,
    pub snippets: Vec<ContextSnippet>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiFragmentRequest {
    pub target_path: Utf8PathBuf,
    pub target_file: String,
    pub template_source_path: Utf8PathBuf,
    pub values: BTreeMap<String, String>,
    pub fragment_index: usize,
    pub display_label: String,
    pub fragment_prompt: String,
    pub active_sentinel: String,
    pub document: String,
    pub context: ContextBundle,
    pub readme_role: Option<ReadmeFragmentRole>,
    pub repair_notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadmeVerificationFinding {
    pub fragment_index: Option<usize>,
    pub fragment_label: Option<String>,
    pub repairable: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReadmeVerificationReport {
    pub findings: Vec<ReadmeVerificationFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateVariant {
    pub output_name: String,
    pub source_path: Utf8PathBuf,
    pub fragments: Vec<TemplateFragment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateBundle {
    pub name: String,
    pub source_path: Utf8PathBuf,
    pub templates: Vec<TemplateVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTemplate {
    pub output_name: String,
    pub source_path: Utf8PathBuf,
    pub fragments: Vec<TemplateFragment>,
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedBundle {
    pub name: String,
    pub source_path: Utf8PathBuf,
    pub templates: Vec<PreparedTemplate>,
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRepository {
    pub(crate) bundles: Vec<TemplateBundle>,
}
