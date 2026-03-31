use crate::model::{CanonicalSkill, SkillMetadata};
use anyhow::{Context, Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use nanite_core::frontmatter::parse_frontmatter;
use std::collections::BTreeMap;
use std::fs;

pub fn load_skills(skills_root: &Utf8Path) -> Result<Vec<CanonicalSkill>> {
    let entries =
        fs::read_dir(skills_root).with_context(|| format!("failed to read {}", skills_root))?;
    let mut skills = Vec::new();

    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let skill_dir = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| anyhow!("non-UTF-8 skill path: {}", path.display()))?;
        let slug = skill_dir
            .file_name()
            .ok_or_else(|| anyhow!("failed to derive a skill name for {skill_dir}"))?
            .to_owned();
        let skill_file = skill_dir.join("SKILL.md");
        let raw = fs::read_to_string(skill_file.as_std_path())
            .with_context(|| format!("failed to read {}", skill_file))?;
        let document = parse_frontmatter::<SkillMetadata>(&raw)
            .with_context(|| format!("failed to parse {}", skill_file))?;

        let mut resources = BTreeMap::new();
        collect_resources(&skill_dir, Utf8Path::new(""), &mut resources)?;

        skills.push(CanonicalSkill {
            slug,
            metadata: document.metadata,
            body: document.body,
            raw_document: raw,
            resources,
        });
    }

    skills.sort_by(|left, right| left.slug.cmp(&right.slug));
    Ok(skills)
}

fn collect_resources(
    skill_root: &Utf8Path,
    relative_root: &Utf8Path,
    resources: &mut BTreeMap<Utf8PathBuf, Vec<u8>>,
) -> Result<()> {
    let current_root = if relative_root.as_str().is_empty() {
        skill_root.to_owned()
    } else {
        skill_root.join(relative_root)
    };

    for entry in
        fs::read_dir(&current_root).with_context(|| format!("failed to read {}", current_root))?
    {
        let entry = entry?;
        let file_name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("skill resource names must be UTF-8"))?;
        if relative_root.as_str().is_empty() && file_name == "SKILL.md" {
            continue;
        }

        let relative_path = if relative_root.as_str().is_empty() {
            Utf8PathBuf::from(&file_name)
        } else {
            relative_root.join(&file_name)
        };
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| anyhow!("non-UTF-8 path encountered: {}", path.display()))?;
        if entry.file_type()?.is_dir() {
            collect_resources(skill_root, &relative_path, resources)?;
            continue;
        }

        resources.insert(
            relative_path,
            fs::read(&path).with_context(|| format!("failed to read {}", path))?,
        );
    }

    Ok(())
}
