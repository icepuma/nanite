mod fs;
mod load;
mod model;
mod render;
mod sync;

pub use load::load_skills;
pub use model::{
    CanonicalSkill, FileDiff, SkillMetadata, SyncAction, SyncItem, SyncReason, SyncReport,
    SyncTarget,
};
pub use sync::sync_skills;

#[cfg(test)]
mod tests {
    use super::fs::{ensure_symlink, remove_path};
    use super::{CanonicalSkill, SkillMetadata, SyncAction, SyncReason, sync_skills};
    use camino::Utf8PathBuf;
    use std::fs;
    use tempfile::tempdir;

    fn sample_skill() -> CanonicalSkill {
        CanonicalSkill {
            slug: "example-skill".to_owned(),
            metadata: SkillMetadata {
                name: "example-skill".to_owned(),
                description: "Summarize a repository".to_owned(),
            },
            body: "Read the repository and summarize it.\n".to_owned(),
            resources: std::collections::BTreeMap::new(),
        }
    }

    fn rendered_skill_md(skill: &CanonicalSkill) -> String {
        format!(
            "---\nname: {name}\ndescription: {description}\n---\n{body}",
            name = skill.metadata.name,
            description = skill.metadata.description,
            body = skill.body,
        )
    }

    #[test]
    fn sync_reports_create_then_unchanged() {
        let tempdir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let render = root.join("render");
        let install = root.join("install");
        let skills = vec![sample_skill()];

        let first = sync_skills(&skills, &render, &install, true).unwrap();
        let second = sync_skills(&skills, &render, &install, false).unwrap();

        assert_eq!(first.items[0].action, SyncAction::Create);
        assert_eq!(second.items[0].action, SyncAction::Unchanged);
        assert_eq!(
            fs::read_to_string(render.join("example-skill/SKILL.md")).unwrap(),
            rendered_skill_md(&sample_skill())
        );
    }

    #[test]
    fn sync_reports_content_drift() {
        let tempdir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let render = root.join("render");
        let install = root.join("install");
        let skills = vec![sample_skill()];

        sync_skills(&skills, &render, &install, true).unwrap();
        fs::write(render.join("example-skill/SKILL.md"), "stale\n").unwrap();

        let report = sync_skills(&skills, &render, &install, false).unwrap();
        let target = &report.items[0].targets[0];

        assert_eq!(report.items[0].action, SyncAction::Override);
        assert!(matches!(
            target.reasons.as_slice(),
            [SyncReason::ContentChanged { diff }]
                if diff.changed == vec![Utf8PathBuf::from("SKILL.md")]
        ));
    }

    #[test]
    fn sync_reports_wrong_symlink_targets() {
        let tempdir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let render = root.join("render");
        let install = root.join("install");
        let wrong = root.join("wrong/example-skill");
        let skills = vec![sample_skill()];

        sync_skills(&skills, &render, &install, true).unwrap();
        fs::create_dir_all(&wrong).unwrap();
        fs::write(wrong.join("SKILL.md"), rendered_skill_md(&sample_skill())).unwrap();
        remove_path(&install.join("example-skill")).unwrap();
        ensure_symlink(&wrong, &install.join("example-skill")).unwrap();

        let report = sync_skills(&skills, &render, &install, false).unwrap();

        assert_eq!(report.items[0].action, SyncAction::Override);
        assert!(matches!(
            report.items[0].targets[0].reasons.as_slice(),
            [SyncReason::WrongSymlink { .. }]
        ));
    }
}
