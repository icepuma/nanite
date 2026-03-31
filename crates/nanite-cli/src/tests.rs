use crate::cli::Cli;
use crate::context::ContextState;
use crate::init::InitProgress;
use crate::jump::{jumpto_fzf_args, render_jumpto_candidates};
use crate::shell::render_fish_init;
use clap::Parser;
use nanite_core::{
    AgentKind, AppPaths, Config, PreparedTemplate, ProjectRecord, SourceKind, TemplateFragment,
};
use std::collections::HashMap;
use std::ffi::OsString;
use time::OffsetDateTime;

#[test]
fn fish_init_includes_wrapper_and_env_export() {
    let env = HashMap::from([("HOME".to_owned(), "/tmp/home".to_owned())]);
    let context = ContextState {
        app_paths: AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap(),
        config: Config {
            workspace_root: camino::Utf8PathBuf::from("/tmp/home/development"),
            agent: AgentKind::Codex,
        },
        workspace_paths: nanite_core::WorkspacePaths::new(camino::Utf8PathBuf::from(
            "/tmp/home/development",
        )),
        git_binary: "git".to_owned(),
        fzf_binary: "fzf".to_owned(),
    };

    let script = render_fish_init(&context);

    assert!(script.contains("function jumpto"));
    assert!(script.contains("CODEX_HOME"));
    assert!(script.contains("CLAUDE_CODE_PLUGIN_SEED_DIR"));
    assert!(script.contains("complete -c jumpto"));
}

#[test]
fn init_progress_renders_readme_checklist() {
    let prepared = nanite_core::PreparedBundle {
        name: "default".to_owned(),
        source_path: "/tmp/default".into(),
        templates: vec![PreparedTemplate {
            output_name: "README.md".to_owned(),
            source_path: "/tmp/default/README.md".into(),
            fragments: vec![
                TemplateFragment::Literal("# test\n\n".to_owned()),
                TemplateFragment::Ai(nanite_core::AiPlaceholder {
                    index: 0,
                    prompt: "badges".to_owned(),
                }),
                TemplateFragment::Literal("\n\n".to_owned()),
                TemplateFragment::Ai(nanite_core::AiPlaceholder {
                    index: 1,
                    prompt: "overview".to_owned(),
                }),
            ],
            values: std::collections::BTreeMap::new(),
        }],
        values: std::collections::BTreeMap::new(),
    };

    let mut progress = InitProgress::new(&prepared);
    progress.mark_done(InitProgress::select_step_index(), None);
    progress.start(progress.generate_step_index(), None);
    let rendered = progress.rendered();

    assert!(rendered.contains("working"));
    assert!(rendered.contains("Generate AI fragments"));
}

#[test]
fn repo_clone_accepts_force_flag() {
    let cli = Cli::parse_from(["nanite", "repo", "clone", "--force", "owner/repo"]);

    match cli.command {
        crate::cli::Commands::Repo {
            command: crate::cli::RepoCommands::Clone { remote, force },
        } => {
            assert_eq!(remote, "owner/repo");
            assert!(force);
        }
        _ => panic!("expected repo clone command"),
    }
}

#[test]
fn jumpto_uses_styled_fzf_arguments() {
    let args = jumpto_fzf_args();

    assert!(args.contains(&"--layout=reverse"));
    assert!(args.contains(&"--border"));
    assert!(args.contains(&"--with-nth=1"));
    assert!(args.contains(&"--prompt=jumpto > "));
    assert!(args.contains(&"--header=Open a repository"));
    assert!(args.contains(
        &"--color=border:8,header:12,prompt:10,pointer:14,marker:11,info:8,spinner:10,hl:14,hl+:14"
    ));
}

#[test]
fn jumpto_candidates_align_name_and_repo_columns() {
    let records = [
        ProjectRecord {
            name: "nanite".to_owned(),
            host: "github.com".to_owned(),
            repo_path: "icepuma/nanite".to_owned(),
            path: camino::Utf8PathBuf::from("/tmp/github.com/icepuma/nanite"),
            origin: "https://github.com/icepuma/nanite.git".to_owned(),
            source_kind: SourceKind::Clone,
            last_seen: OffsetDateTime::now_utc(),
        },
        ProjectRecord {
            name: "rawkode-academy".to_owned(),
            host: "github.com".to_owned(),
            repo_path: "rawkode-academy/rawkode-academy".to_owned(),
            path: camino::Utf8PathBuf::from("/tmp/github.com/rawkode-academy/rawkode-academy"),
            origin: "https://github.com/rawkode-academy/rawkode-academy.git".to_owned(),
            source_kind: SourceKind::Clone,
            last_seen: OffsetDateTime::now_utc(),
        },
    ];

    let rendered = render_jumpto_candidates(records.iter().collect());

    assert_eq!(
        rendered[0],
        "nanite           github.com/icepuma/nanite\t/tmp/github.com/icepuma/nanite"
    );
    assert_eq!(
        rendered[1],
        "rawkode-academy  github.com/rawkode-academy/rawkode-academy\t/tmp/github.com/rawkode-academy/rawkode-academy"
    );
}
