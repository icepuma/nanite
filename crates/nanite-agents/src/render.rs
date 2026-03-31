use crate::model::CanonicalSkill;
use camino::Utf8PathBuf;
use std::collections::BTreeMap;

pub fn render_codex_skill(skill: &CanonicalSkill) -> BTreeMap<Utf8PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert(
        Utf8PathBuf::from("SKILL.md"),
        skill.raw_document.as_bytes().to_vec(),
    );
    for (relative_path, contents) in &skill.resources {
        files.insert(relative_path.clone(), contents.clone());
    }
    files
}

pub fn render_claude_skill(skill: &CanonicalSkill) -> BTreeMap<Utf8PathBuf, Vec<u8>> {
    let description = skill
        .metadata
        .providers
        .claude
        .description
        .clone()
        .unwrap_or_else(|| skill.metadata.description.clone());
    let header = format!("---\ndescription: {description}\n---\n");

    let mut files = BTreeMap::new();
    files.insert(
        Utf8PathBuf::from("SKILL.md"),
        format!("{header}{}", skill.body).into_bytes(),
    );
    for (relative_path, contents) in &skill.resources {
        files.insert(relative_path.clone(), contents.clone());
    }
    files
}
