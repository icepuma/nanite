#![allow(
    clippy::missing_const_for_fn,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::struct_field_names,
    clippy::uninlined_format_args
)]

pub mod app_paths;
pub mod config;
pub mod frontmatter;
pub mod prompt;
pub mod registry;
pub mod templates;
pub mod workspace;

pub use app_paths::AppPaths;
pub use config::{AgentKind, Config};
pub use prompt::Prompter;
pub use registry::{ProjectRecord, Registry, SourceKind};
pub use templates::{
    AiFragment, AiFragmentRequest, AiPlaceholder, ContextBundle, ContextSnippet, PreparedTemplate,
    PreparedBundle, ReadmeFragmentRole, ReadmeVerificationFinding, ReadmeVerificationReport,
    RepoContextFacts, TemplateBundle, TemplateFragment, TemplateMetadata, TemplateRepository,
    TemplateVariant, TextPlaceholder,
};
pub use workspace::WorkspacePaths;
