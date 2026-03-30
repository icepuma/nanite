use crate::templates::TextPlaceholder;
use anyhow::Result;
use std::collections::BTreeMap;

pub trait Prompter {
    fn prompt(&mut self, placeholder: &TextPlaceholder) -> Result<String>;
}

#[derive(Debug, Default)]
pub struct StaticPrompter {
    answers: BTreeMap<String, String>,
}

impl StaticPrompter {
    pub fn new(answers: BTreeMap<String, String>) -> Self {
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
