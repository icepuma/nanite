mod context;
mod model;
mod parse;
mod render;
mod repository;
mod verify;

pub use model::{
    AiFragment, AiFragmentRequest, AiPlaceholder, ContextBundle, ContextSnippet, PreparedBundle,
    PreparedTemplate, ReadmeFragmentRole, ReadmeVerificationFinding, ReadmeVerificationReport,
    RepoContextFacts, TemplateBundle, TemplateFragment, TemplateMetadata, TemplateRepository,
    TemplateVariant, TextPlaceholder,
};
