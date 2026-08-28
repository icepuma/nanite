use crate::cli::Cli;
use crate::jump::{jumpto_fzf_args, render_jumpto_candidates};
use crate::shell::render_fish_init;
use clap::Parser;
use nanite_core::{ProjectRecord, SourceKind};
use time::OffsetDateTime;

#[test]
fn fish_init_includes_wrapper_and_completions() {
    let script = render_fish_init();

    assert!(script.contains("function jumpto"));
    assert!(script.contains("complete -c jumpto"));
    assert!(!script.contains("CODEX_HOME"));
}

#[test]
fn repo_clone_takes_only_remote() {
    let cli = Cli::parse_from(["nanite", "repo", "clone", "owner/repo"]);

    match cli.command {
        crate::cli::Commands::Repo {
            command: crate::cli::RepoCommands::Clone { remote },
        } => {
            assert_eq!(remote, "owner/repo");
        }
        _ => panic!("expected repo clone command"),
    }
}

#[test]
fn repo_clone_rejects_unknown_flag() {
    let parsed = Cli::try_parse_from(["nanite", "repo", "clone", "--force", "owner/repo"]);
    assert!(parsed.is_err(), "--force should no longer be accepted");
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
