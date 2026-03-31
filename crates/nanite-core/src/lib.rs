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
    AiFragment, AiFragmentRequest, AiPlaceholder, ContextBundle, ContextSnippet, PreparedBundle,
    PreparedTemplate, ReadmeFragmentRole, ReadmeVerificationFinding, ReadmeVerificationReport,
    RepoContextFacts, TemplateBundle, TemplateFragment, TemplateMetadata, TemplateRepository,
    TemplateVariant, TextPlaceholder,
};
pub use workspace::WorkspacePaths;
