use crate::templates::TextPlaceholder;
use anyhow::Result;
use std::collections::BTreeMap;

pub trait Prompter {
    /// Resolves a value for the provided placeholder.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing prompt implementation cannot provide a
    /// value for the placeholder.
    fn prompt(&mut self, placeholder: &TextPlaceholder) -> Result<String>;
}

#[derive(Debug, Default)]
pub struct StaticPrompter {
    answers: BTreeMap<String, String>,
}

impl StaticPrompter {
    #[must_use]
    pub const fn new(answers: BTreeMap<String, String>) -> Self {
        Self { answers }
    }
}

impl Prompter for StaticPrompter {
    fn prompt(&mut self, placeholder: &TextPlaceholder) -> Result<String> {
        Ok(self
            .answers
            .get(&placeholder.name)
            .cloned()
            .unwrap_or_default())
    }
}
