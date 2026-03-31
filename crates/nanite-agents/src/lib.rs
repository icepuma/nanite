mod fs;
mod load;
mod model;
mod render;
mod sync;

pub use load::load_skills;
pub use model::{
    CanonicalSkill, ClaudeOverrides, FileDiff, ProviderOverrides, SkillMetadata, SkillProvider,
    SyncAction, SyncItem, SyncReason, SyncReport, SyncTarget,
};
pub use sync::{sync_claude, sync_codex};

#[cfg(test)]
mod tests {
    use super::fs::{ensure_symlink, remove_path};
    use super::{CanonicalSkill, SkillMetadata, SyncAction, SyncReason, sync_claude, sync_codex};
    use camino::Utf8PathBuf;
    use std::fs;
    use tempfile::tempdir;

    use super::{ClaudeOverrides, ProviderOverrides};

    fn sample_skill() -> CanonicalSkill {
        CanonicalSkill {
            slug: "example-skill".to_owned(),
            metadata: SkillMetadata {
                name: "example-skill".to_owned(),
                description: "Summarize a repository".to_owned(),
                triggers: vec!["summarize repo".to_owned()],
                tags: vec!["analysis".to_owned()],
                providers: ProviderOverrides {
                    claude: ClaudeOverrides::default(),
                },
            },
            body: "Read the repository and summarize it.\n".to_owned(),
            raw_document: concat!(
                "---\n",
                "name: example-skill\n",
                "description: Summarize a repository\n",
                "triggers:\n",
                "  - summarize repo\n",
                "tags:\n",
                "  - analysis\n",
                "---\n",
                "Read the repository and summarize it.\n"
            )
            .to_owned(),
            resources: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn codex_sync_reports_create_then_unchanged() {
        let tempdir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let render = root.join("render");
        let install = root.join("install");
        let skills = vec![sample_skill()];

        let first = sync_codex(&skills, &render, &install, true).unwrap();
        let second = sync_codex(&skills, &render, &install, false).unwrap();

        assert_eq!(first.items[0].action, SyncAction::Create);
        assert_eq!(second.items[0].action, SyncAction::Unchanged);
        assert_eq!(
            fs::read_to_string(render.join("example-skill/SKILL.md")).unwrap(),
            sample_skill().raw_document
        );
    }

    #[test]
    fn claude_sync_reports_create_for_missing_bundle() {
        let tempdir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let report = sync_claude(&[sample_skill()], &[root], false).unwrap();

        assert_eq!(report.items[0].action, SyncAction::Create);
    }

    #[test]
    fn codex_sync_reports_content_drift() {
        let tempdir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let render = root.join("render");
        let install = root.join("install");
        let skills = vec![sample_skill()];

        sync_codex(&skills, &render, &install, true).unwrap();
        fs::write(render.join("example-skill/SKILL.md"), "stale\n").unwrap();

        let report = sync_codex(&skills, &render, &install, false).unwrap();
        let target = &report.items[0].targets[0];

        assert_eq!(report.items[0].action, SyncAction::Override);
        assert!(matches!(
            target.reasons.as_slice(),
            [SyncReason::ContentChanged { diff }]
                if diff.changed == vec![Utf8PathBuf::from("SKILL.md")]
        ));
    }

    #[test]
    fn codex_sync_reports_wrong_symlink_targets() {
        let tempdir = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let render = root.join("render");
        let install = root.join("install");
        let wrong = root.join("wrong/example-skill");
        let skills = vec![sample_skill()];

        sync_codex(&skills, &render, &install, true).unwrap();
        fs::create_dir_all(&wrong).unwrap();
        fs::write(wrong.join("SKILL.md"), sample_skill().raw_document).unwrap();
        remove_path(&install.join("example-skill")).unwrap();
        ensure_symlink(&wrong, &install.join("example-skill")).unwrap();

        let report = sync_codex(&skills, &render, &install, false).unwrap();

        assert_eq!(report.items[0].action, SyncAction::Override);
        assert!(matches!(
            report.items[0].targets[0].reasons.as_slice(),
            [SyncReason::WrongSymlink { .. }]
        ));
    }
}
