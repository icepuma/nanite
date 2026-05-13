use crate::model::CanonicalSkill;
use camino::Utf8PathBuf;
use std::collections::BTreeMap;

pub fn render_skill(skill: &CanonicalSkill) -> BTreeMap<Utf8PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert(
        Utf8PathBuf::from("SKILL.md"),
        render_skill_md(skill).into_bytes(),
    );
    for (relative_path, contents) in &skill.resources {
        files.insert(relative_path.clone(), contents.clone());
    }
    files
}

fn render_skill_md(skill: &CanonicalSkill) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\n---\n{body}",
        name = skill.metadata.name,
        description = skill.metadata.description,
        body = skill.body,
    )
}
