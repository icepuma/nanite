use assert_cmd::Command;
use camino::Utf8PathBuf;
use nanite_core::{ProjectRecord, Registry, SourceKind};
use nanite_git::parse_remote;
use std::fs;
use std::process::Command as ProcessCommand;
use tempfile::TempDir;
use time::OffsetDateTime;

const FAKE_GIT_SCRIPT: &str = r#"#!/bin/sh
set -eu
if [ "$1" = "-C" ]; then
    repo="$2"
    shift 2
    if [ "$1" = "remote" ] && [ "$2" = "get-url" ] && [ "$3" = "origin" ]; then
        if [ -f "$repo/.git_origin" ]; then
            cat "$repo/.git_origin"
            exit 0
        fi
        exit 2
    fi
fi
echo "unsupported fake git invocation" >&2
exit 1
"#;

const FAKE_FZF_SCRIPT: &str = r#"#!/bin/sh
set -eu
query=""
while [ $# -gt 0 ]; do
    case "$1" in
        -q)
            query="$2"
            shift 2
            ;;
        *)
            shift
            ;;
    esac
done
input=$(cat)
if [ -z "$query" ]; then
    printf '%s\n' "$input" | head -n 1
    exit 0
fi
match=$(printf '%s\n' "$input" | grep -F "$query" | head -n 1 || true)
if [ -z "$match" ]; then
    exit 1
fi
printf '%s\n' "$match"
"#;

struct TestEnv {
    _tempdir: TempDir,
    config_dir: Utf8PathBuf,
    fzf_script: Utf8PathBuf,
    git_script: Utf8PathBuf,
    home_dir: Utf8PathBuf,
    state_dir: Utf8PathBuf,
    workspace_root: Utf8PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let tempdir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let home_dir = root.join("home");
        let config_dir = root.join("config");
        let state_dir = root.join("state");
        let workspace_root = home_dir.join("development");
        fs::create_dir_all(&home_dir).unwrap();
        fs::create_dir_all(&config_dir).unwrap();
        fs::create_dir_all(&state_dir).unwrap();

        let git_script = root.join("fake-git.sh");
        let fzf_script = root.join("fake-fzf.sh");
        write_script(&git_script, FAKE_GIT_SCRIPT);
        write_script(&fzf_script, FAKE_FZF_SCRIPT);

        Self {
            _tempdir: tempdir,
            config_dir,
            fzf_script,
            git_script,
            home_dir,
            state_dir,
            workspace_root,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("nanite").unwrap();
        command
            .env("HOME", &self.home_dir)
            .env("NANITE_CONFIG_DIR", &self.config_dir)
            .env("NANITE_STATE_DIR", &self.state_dir)
            .env("NANITE_GIT", &self.git_script)
            .env("NANITE_FZF", &self.fzf_script);
        command
    }

    fn registry_path(&self) -> Utf8PathBuf {
        self.state_dir.join("registry.json")
    }

    fn repos_root(&self) -> Utf8PathBuf {
        self.workspace_root.join("repos")
    }

    fn setup(&self) {
        self.command()
            .args(["setup", self.workspace_root.as_str()])
            .assert()
            .success();
    }

    fn assert_unconfigured_failure(&self, args: &[&str]) {
        self.command()
            .args(args)
            .assert()
            .failure()
            .stderr(predicates::str::contains("run 'nanite setup <path>' first"));
    }
}

#[test]
fn setup_creates_the_workspace_layout() {
    let env = TestEnv::new();
    let expected_root = Utf8PathBuf::from_path_buf(fs::canonicalize(&env.home_dir).unwrap())
        .unwrap()
        .join("development");

    env.command()
        .args(["setup", env.workspace_root.as_str()])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "configured {expected_root}"
        )));

    assert!(expected_root.join("repos").exists());
    assert!(
        fs::read_to_string(env.config_dir.join("config.toml"))
            .unwrap()
            .contains(expected_root.as_str())
    );
}

#[test]
fn setup_succeeds_for_existing_empty_directory() {
    let env = TestEnv::new();
    fs::create_dir_all(&env.workspace_root).unwrap();

    env.command()
        .args(["setup", env.workspace_root.as_str()])
        .assert()
        .success();
}

#[test]
fn setup_fails_for_existing_non_empty_directory() {
    let env = TestEnv::new();
    fs::create_dir_all(&env.workspace_root).unwrap();
    fs::write(env.workspace_root.join("note.txt"), "occupied\n").unwrap();

    env.command()
        .args(["setup", env.workspace_root.as_str()])
        .assert()
        .failure()
        .stderr(predicates::str::contains("is not empty"));
}

#[test]
fn setup_fails_when_nanite_is_already_configured() {
    let env = TestEnv::new();
    env.setup();

    env.command()
        .args(["setup", env.home_dir.join("other-workspace").as_str()])
        .assert()
        .failure()
        .stderr(predicates::str::contains("already configured"));
}

#[test]
fn commands_fail_before_setup() {
    let env = TestEnv::new();

    env.assert_unconfigured_failure(&["repo", "refresh"]);
    env.assert_unconfigured_failure(&["repo", "clone", "https://example.com/a/b.git"]);
    env.assert_unconfigured_failure(&["repo", "remove", "github.com/example/tool"]);
    env.assert_unconfigured_failure(&["repo", "import", "imports/toolbox"]);
    env.assert_unconfigured_failure(&["jumpto"]);
    env.assert_unconfigured_failure(&["shell", "init", "fish"]);
}

#[test]
fn repo_clone_and_scan_update_the_registry() {
    let env = TestEnv::new();
    env.setup();
    let remote_repo = create_bare_remote(&env);
    let remote_url = format!("file://{}", remote_repo.as_str());
    let spec = parse_remote(&remote_url).unwrap();
    let cloned_path = env.repos_root().join(&spec.host).join(&spec.repo_path);

    env.command()
        .args(["repo", "clone", &remote_url])
        .assert()
        .success();

    assert!(cloned_path.join(".git").exists());
    assert_eq!(
        fs::read_to_string(cloned_path.join("README.md")).unwrap(),
        "fixture\n"
    );

    let manual_repo = env.repos_root().join("local/manual");
    fs::create_dir_all(manual_repo.join(".git")).unwrap();

    env.command().args(["repo", "refresh"]).assert().success();

    let raw_registry = fs::read_to_string(env.registry_path()).unwrap();
    assert!(raw_registry.contains(&spec.repo_path));
    assert!(raw_registry.contains("\"repo_path\": \"manual\""));
}

#[test]
fn repo_import_preserves_git_data_and_origin_layout() {
    let env = TestEnv::new();
    env.setup();
    let source = env.home_dir.join("imports/toolbox");
    fs::create_dir_all(source.join(".git")).unwrap();
    fs::write(
        source.join(".git_origin"),
        "git@github.com:icepuma/tools/toolbox.git\n",
    )
    .unwrap();
    fs::write(source.join("README.md"), "hello\n").unwrap();

    env.command()
        .args(["repo", "import", source.as_str()])
        .assert()
        .success();

    let imported = env.repos_root().join("github.com/icepuma/tools/toolbox");
    assert!(imported.join(".git").exists());
    assert_eq!(
        fs::read_to_string(imported.join("README.md")).unwrap(),
        "hello\n"
    );
}

#[test]
fn repo_remove_deletes_repo_and_prunes_empty_parents() {
    let env = TestEnv::new();
    env.setup();
    let repos_root =
        Utf8PathBuf::from_path_buf(fs::canonicalize(env.repos_root()).unwrap()).unwrap();
    let repo = repos_root.join("github.com/icepuma/nanite");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();

    let mut registry = Registry::default();
    registry.upsert(ProjectRecord {
        name: "nanite".to_owned(),
        host: "github.com".to_owned(),
        repo_path: "icepuma/nanite".to_owned(),
        path: repo.clone(),
        origin: "https://github.com/icepuma/nanite.git".to_owned(),
        source_kind: SourceKind::Clone,
        last_seen: OffsetDateTime::now_utc(),
    });
    registry.save(&env.registry_path()).unwrap();

    env.command()
        .args(["repo", "remove", "--yes", "github.com/icepuma/nanite"])
        .assert()
        .success()
        .stdout(predicates::str::contains("removed"));

    assert!(!repo.exists());
    assert!(!repos_root.join("github.com/icepuma").exists());
    assert!(!repos_root.join("github.com").exists());
    let registry = Registry::load(&env.registry_path()).unwrap();
    assert!(registry.entries().is_empty());
}

#[test]
fn repo_remove_requires_yes_when_not_interactive() {
    let env = TestEnv::new();
    env.setup();
    let repos_root =
        Utf8PathBuf::from_path_buf(fs::canonicalize(env.repos_root()).unwrap()).unwrap();
    let repo = repos_root.join("github.com/icepuma/nanite");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("README.md"), "fixture\n").unwrap();

    env.command()
        .args(["repo", "remove", "github.com/icepuma/nanite"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "repo remove requires confirmation; rerun with --yes",
        ));

    assert!(repo.exists());
}

#[test]
fn jumpto_uses_query_against_the_registry() {
    let env = TestEnv::new();
    env.setup();
    let mut registry = Registry::default();
    registry.upsert(ProjectRecord {
        name: "nanite".to_owned(),
        host: "github.com".to_owned(),
        repo_path: "icepuma/nanite".to_owned(),
        path: env.repos_root().join("github.com/icepuma/nanite"),
        origin: "https://github.com/icepuma/nanite.git".to_owned(),
        source_kind: SourceKind::Clone,
        last_seen: OffsetDateTime::now_utc(),
    });
    registry.upsert(ProjectRecord {
        name: "nanite".to_owned(),
        host: "gitlab.com".to_owned(),
        repo_path: "example/nanite".to_owned(),
        path: env.repos_root().join("gitlab.com/example/nanite"),
        origin: "https://gitlab.com/example/nanite.git".to_owned(),
        source_kind: SourceKind::Clone,
        last_seen: OffsetDateTime::now_utc(),
    });
    registry.save(&env.registry_path()).unwrap();

    env.command()
        .args(["jumpto", "gitlab.com/example/nanite"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            env.repos_root().join("gitlab.com/example/nanite").as_str(),
        ));
}

#[test]
fn shell_init_fish_outputs_complete_setup() {
    let env = TestEnv::new();
    env.setup();

    env.command()
        .args(["shell", "init", "fish"])
        .assert()
        .success()
        .stdout(predicates::str::contains("function jumpto"))
        .stdout(predicates::str::contains("complete -c jumpto"))
        .stdout(predicates::str::contains("nanite __complete-repo-remove"));
}

#[test]
fn complete_repo_remove_lists_registry_targets() {
    let env = TestEnv::new();
    env.setup();
    let mut registry = Registry::default();
    registry.upsert(ProjectRecord {
        name: "nanite".to_owned(),
        host: "github.com".to_owned(),
        repo_path: "icepuma/nanite".to_owned(),
        path: env.repos_root().join("github.com/icepuma/nanite"),
        origin: "https://github.com/icepuma/nanite.git".to_owned(),
        source_kind: SourceKind::Clone,
        last_seen: OffsetDateTime::now_utc(),
    });
    registry.save(&env.registry_path()).unwrap();

    env.command()
        .arg("__complete-repo-remove")
        .assert()
        .success()
        .stdout(predicates::str::contains("github.com/icepuma/nanite"));
}

fn write_script(path: &Utf8PathBuf, contents: &str) {
    fs::write(path, contents).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}

fn create_bare_remote(env: &TestEnv) -> Utf8PathBuf {
    let source = env.home_dir.join("seed/source");
    let bare = env.home_dir.join("seed/origin.git");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("README.md"), "fixture\n").unwrap();

    run_git(["init", source.as_str()]);
    run_git(["-C", source.as_str(), "add", "README.md"]);
    run_git([
        "-C",
        source.as_str(),
        "-c",
        "user.name=Nanite Test",
        "-c",
        "user.email=nanite@example.com",
        "commit",
        "-m",
        "init",
    ]);
    run_git(["clone", "--bare", source.as_str(), bare.as_str()]);

    bare
}

fn run_git<const N: usize>(args: [&str; N]) {
    let status = ProcessCommand::new("git").args(args).status().unwrap();
    assert!(status.success());
}
