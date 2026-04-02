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

const FAKE_PROVIDER_SCRIPT: &str = r#"#!/bin/sh
set -eu
output=""
prompt=""
all_args="$*"
while [ $# -gt 0 ]; do
    case "$1" in
        -o)
            output="$2"
            shift 2
            ;;
        *)
            prompt="$1"
            shift
            ;;
    esac
done
log="${NANITE_PROVIDER_LOG:-}"
if [ -n "$log" ]; then
    printf '%s %s\n' "$(basename "$0")" "$all_args" >> "$log"
    printf 'CODEX_HOME=%s\n' "${CODEX_HOME:-}" >> "$log"
    printf 'CLAUDE_CODE_PLUGIN_SEED_DIR=%s\n' "${CLAUDE_CODE_PLUGIN_SEED_DIR:-}" >> "$log"
    printf 'PROMPT_START\n%s\nPROMPT_END\n' "$prompt" >> "$log"
fi
mode="${NANITE_FAKE_PROVIDER_MODE:-write}"
repo_name=$(printf '%s\n' "$prompt" | sed -n 's/^- repo_name: //p' | head -n 1)
if [ -z "$repo_name" ]; then
    repo_name=$(printf '%s\n' "$prompt" | sed -n 's/^- Repo name: //p' | head -n 1)
fi
if [ -z "$repo_name" ]; then repo_name="generated"; fi
label=$(printf '%s\n' "$prompt" | sed -n 's/^- Label: //p' | head -n 1)
repairing=0
if printf '%s\n' "$prompt" | grep -q '<repair>'; then
    repairing=1
fi
emit_payload() {
    if [ -n "$output" ]; then
        printf '%b' "$1" > "$output"
    else
        printf '%b' "$1"
    fi
}
default_payload() {
    case "$label" in
        "Badges")
            emit_payload ""
            ;;
        "Overview")
            emit_payload "$repo_name keeps repository templates and workspace tooling aligned around a single local workflow.\n\nIt helps generate consistent project files, keep docs predictable, and reduce setup churn across repositories."
            ;;
        "Quick Start")
            emit_payload "- Install or refresh dependencies with the verified setup command for this repository.\n- Start from the repository root and use the verified run path when one exists.\n- Use the generated docs as the baseline for the next local step."
            ;;
        "Usage")
            emit_payload "- Follow the repository's main workflow from the verified commands and directories in the repo brief.\n- Use nanite commands and existing project tooling to keep repeated work consistent.\n- Prefer the documented local path instead of inventing extra setup."
            ;;
        "Tests")
            emit_payload "- No verified test command was found."
            ;;
        *)
            emit_payload "Generated content for $repo_name."
            ;;
    esac
}
case "$mode" in
    write)
        default_payload
        ;;
    repair-overview)
        if [ "$label" = "Overview" ] && [ "$repairing" -eq 0 ]; then
            emit_payload "$repo_name is a repository toolkit. It keeps files moving. It helps with setup. It also standardizes workflows."
        else
            default_payload
        fi
        ;;
    repair-overview-still-bad)
        if [ "$label" = "Overview" ]; then
            emit_payload "$repo_name is a repository toolkit. It keeps files moving. It helps with setup. It also standardizes workflows."
        else
            default_payload
        fi
        ;;
    fenced)
        emit_payload '```md
bad
```'
        ;;
    fail)
        exit 1
        ;;
    *)
        echo "unsupported fake provider mode: $mode" >&2
        exit 1
        ;;
esac
"#;

struct TestEnv {
    _tempdir: TempDir,
    config_dir: Utf8PathBuf,
    codex_script: Utf8PathBuf,
    data_dir: Utf8PathBuf,
    fzf_script: Utf8PathBuf,
    git_script: Utf8PathBuf,
    home_dir: Utf8PathBuf,
    provider_log: Utf8PathBuf,
    state_dir: Utf8PathBuf,
    workspace_root: Utf8PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let tempdir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tempdir.path().to_path_buf()).unwrap();
        let home_dir = root.join("home");
        let config_dir = root.join("config");
        let data_dir = root.join("data");
        let state_dir = root.join("state");
        let workspace_root = home_dir.join("development");
        let provider_log = root.join("provider.log");
        fs::create_dir_all(&home_dir).unwrap();
        fs::create_dir_all(&config_dir).unwrap();
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&state_dir).unwrap();

        let git_script = root.join("fake-git.sh");
        let fzf_script = root.join("fake-fzf.sh");
        let codex_script = root.join("codex");
        let claude_script = root.join("claude");
        write_script(&git_script, FAKE_GIT_SCRIPT);
        write_script(&fzf_script, FAKE_FZF_SCRIPT);
        write_script(&claude_script, FAKE_PROVIDER_SCRIPT);
        write_script(&codex_script, FAKE_PROVIDER_SCRIPT);

        Self {
            _tempdir: tempdir,
            config_dir,
            codex_script,
            data_dir,
            fzf_script,
            git_script,
            home_dir,
            provider_log,
            state_dir,
            workspace_root,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("nanite").unwrap();
        command
            .env("HOME", &self.home_dir)
            .env("CODEX_HOME", self.home_dir.join(".codex"))
            .env("NANITE_CONFIG_DIR", &self.config_dir)
            .env("NANITE_DATA_DIR", &self.data_dir)
            .env("NANITE_STATE_DIR", &self.state_dir)
            .env("NANITE_GIT", &self.git_script)
            .env("NANITE_FZF", &self.fzf_script)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.codex_script.parent().unwrap(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("NANITE_PROVIDER_LOG", &self.provider_log);
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

    fn set_agent(&self, agent: &str) {
        let config_path = self.config_dir.join("config.toml");
        let config = fs::read_to_string(&config_path).unwrap();
        let updated = config.replace("agent = \"codex\"", &format!("agent = \"{agent}\""));
        fs::write(config_path, updated).unwrap();
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
fn setup_creates_workspace_and_seeds_content() {
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

    assert!(expected_root.join("templates/default/README.md").exists());
    assert!(
        expected_root
            .join("skills/conventional-commits/SKILL.md")
            .exists()
    );
    assert!(
        expected_root
            .join("skills/conventional-commits/agents/openai.yaml")
            .exists()
    );
    assert!(expected_root.join("repos").exists());
    assert!(
        fs::read_to_string(env.config_dir.join("config.toml"))
            .unwrap()
            .contains(expected_root.as_str())
    );
    assert!(
        fs::read_to_string(env.config_dir.join("config.toml"))
            .unwrap()
            .contains("agent = \"codex\"")
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

    env.assert_unconfigured_failure(&["init"]);
    env.assert_unconfigured_failure(&["skill", "sync", "codex"]);
    env.assert_unconfigured_failure(&["repo", "refresh"]);
    env.assert_unconfigured_failure(&["repo", "clone", "https://example.com/a/b.git"]);
    env.assert_unconfigured_failure(&["repo", "remove", "github.com/example/tool"]);
    env.assert_unconfigured_failure(&["repo", "import", "imports/toolbox"]);
    env.assert_unconfigured_failure(&["jumpto"]);
    env.assert_unconfigured_failure(&["shell", "init", "fish"]);
}

#[test]
fn generate_gitignore_succeeds_before_setup() {
    let env = TestEnv::new();
    let project_dir = env.home_dir.join("scratch");
    fs::create_dir_all(&project_dir).unwrap();

    env.command()
        .current_dir(&project_dir)
        .args(["generate", "gitignore"])
        .write_stdin("root/rust\n")
        .assert()
        .success()
        .stdout(predicates::str::contains("wrote"))
        .stdout(predicates::str::contains(".gitignore"));

    let rendered = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
    assert!(rendered.contains("\ntarget\n"));
}

#[test]
fn generate_gitignore_combines_selected_templates_in_catalog_order() {
    let env = TestEnv::new();
    let project_dir = env.home_dir.join("combined");
    fs::create_dir_all(&project_dir).unwrap();

    env.command()
        .current_dir(&project_dir)
        .args(["generate", "gitignore"])
        .write_stdin("root/rust,root/java\n")
        .assert()
        .success();

    let rendered = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
    assert!(rendered.contains("# --- java ---"));
    assert!(rendered.contains("# --- rust ---"));
    let java_index = rendered.find("*.class").unwrap();
    let rust_index = rendered.find("\ntarget\n").unwrap();

    assert!(java_index < rust_index);
}

#[test]
fn generate_gitignore_refuses_to_overwrite_without_force() {
    let env = TestEnv::new();
    let project_dir = env.home_dir.join("existing");
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(project_dir.join(".gitignore"), "keep-me\n").unwrap();

    env.command()
        .current_dir(&project_dir)
        .args(["generate", "gitignore"])
        .write_stdin("root/rust\n")
        .assert()
        .failure()
        .stderr(predicates::str::contains("already exists"));

    assert_eq!(
        fs::read_to_string(project_dir.join(".gitignore")).unwrap(),
        "keep-me\n"
    );
}

#[test]
fn generate_gitignore_overwrites_with_force() {
    let env = TestEnv::new();
    let project_dir = env.home_dir.join("overwrite");
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(project_dir.join(".gitignore"), "keep-me\n").unwrap();

    env.command()
        .current_dir(&project_dir)
        .args(["generate", "gitignore", "--force"])
        .write_stdin("root/rust\n")
        .assert()
        .success();

    let rendered = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
    assert!(rendered.contains("\ntarget\n"));
    assert!(!rendered.contains("keep-me"));
}

#[test]
fn init_interactively_renders_a_deterministic_template() {
    let env = TestEnv::new();
    env.setup();
    let project_dir = env.repos_root().join("github.com/icepuma/sample");
    fs::create_dir_all(&project_dir).unwrap();

    env.command()
        .current_dir(&project_dir)
        .arg("init")
        .write_stdin("1\nsample\n")
        .assert()
        .success()
        .stdout(predicates::str::contains("wrote "))
        .stdout(predicates::str::contains("AGENTS.md"))
        .stdout(predicates::str::contains("README.md"))
        .stdout(predicates::str::contains("LICENSE"));

    let agents = fs::read_to_string(project_dir.join("AGENTS.md")).unwrap();

    assert!(agents.contains("# Agent Guide"));
    assert!(agents.contains("This repository is `sample`."));
    assert!(project_dir.join("README.md").exists());
    assert!(project_dir.join("LICENSE").exists());
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
fn skills_sync_is_dry_run_by_default_and_applies_when_requested() {
    let env = TestEnv::new();
    env.setup();
    let codex_conventional_commits = env.home_dir.join(".codex/skills/conventional-commits");

    env.command()
        .args(["skill", "sync", "codex"])
        .assert()
        .success()
        .stdout(predicates::str::contains("sync codex skills (dry run)"))
        .stdout(predicates::str::contains("[create] conventional-commits"))
        .stdout(predicates::str::contains("state missing"))
        .stdout(predicates::str::contains("+ SKILL.md"));
    assert!(!codex_conventional_commits.exists());

    env.command()
        .args(["skill", "sync", "codex", "--apply"])
        .assert()
        .success()
        .stdout(predicates::str::contains("sync codex skills"))
        .stdout(predicates::str::contains("[create] conventional-commits"));
    assert!(codex_conventional_commits.join("SKILL.md").exists());
    assert!(
        codex_conventional_commits
            .join("agents/openai.yaml")
            .exists()
    );
    assert!(
        fs::read_to_string(codex_conventional_commits.join("SKILL.md"))
            .unwrap()
            .starts_with("---\n")
    );

    env.command()
        .args(["skill", "sync", "claude", "--apply"])
        .assert()
        .success()
        .stdout(predicates::str::contains("sync claude skills"))
        .stdout(predicates::str::contains("[create] conventional-commits"));
    assert!(
        env.data_dir
            .join("claude/plugins/nanite-skills/skills/conventional-commits/SKILL.md")
            .exists()
    );
}

#[test]
fn skills_sync_reports_content_drift() {
    let env = TestEnv::new();
    env.setup();

    env.command()
        .args(["skill", "sync", "codex", "--apply"])
        .assert()
        .success();

    fs::write(
        env.data_dir
            .join("rendered/codex/conventional-commits/SKILL.md"),
        "stale\n",
    )
    .unwrap();

    env.command()
        .args(["skill", "sync", "codex"])
        .assert()
        .success()
        .stdout(predicates::str::contains("[update] conventional-commits"))
        .stdout(predicates::str::contains("state content changed"))
        .stdout(predicates::str::contains("~ SKILL.md"));
}

#[test]
fn init_with_ai_fragments_requests_plain_text_output_and_writes_file_locally() {
    let env = TestEnv::new();
    env.setup();
    let project_dir = env.repos_root().join("github.com/icepuma/interactive");
    fs::create_dir_all(&project_dir).unwrap();

    env.command()
        .current_dir(&project_dir)
        .arg("init")
        .write_stdin("default\ninteractive\n")
        .assert()
        .success()
        .stdout(predicates::str::contains("wrote "))
        .stderr(predicates::str::contains("done Generate AI fragments"))
        .stderr(predicates::str::contains("done Verify outputs"));

    let readme = fs::read_to_string(project_dir.join("README.md")).unwrap();

    assert!(readme.contains("# interactive"));
    assert!(readme.contains(
        "interactive keeps repository templates and workspace tooling aligned around a single local workflow."
    ));
    assert!(readme.contains("## Quick Start"));
    assert!(readme.contains("## Usage"));
    assert!(readme.contains("## Tests"));
    assert!(readme.contains("## Contributing"));
    assert!(readme.contains("## License"));

    let provider_log = fs::read_to_string(&env.provider_log).unwrap();
    assert!(provider_log.contains("codex exec"));
    assert!(provider_log.contains("--model gpt-5.4-mini"));
    assert!(!provider_log.contains("--output-schema"));
    assert!(provider_log.contains("CODEX_HOME="));
    assert_eq!(provider_log.matches("codex exec").count(), 6);
    assert!(provider_log.contains("- Label: Overview"));
}

#[test]
fn init_with_ai_fragments_does_not_require_synced_skills() {
    let env = TestEnv::new();
    env.setup();
    let project_dir = env.repos_root().join("github.com/icepuma/no-skill-needed");
    fs::create_dir_all(&project_dir).unwrap();

    env.command()
        .current_dir(&project_dir)
        .arg("init")
        .write_stdin("default\nno-skill-needed\n")
        .assert()
        .success();

    assert!(project_dir.join("README.md").exists());
}

#[test]
fn init_with_ai_fragments_fail_when_provider_returns_fenced_output() {
    let env = TestEnv::new();
    env.setup();
    let project_dir = env.repos_root().join("github.com/icepuma/fenced-output");
    fs::create_dir_all(&project_dir).unwrap();

    env.command()
        .current_dir(&project_dir)
        .env("NANITE_FAKE_PROVIDER_MODE", "fenced")
        .arg("init")
        .write_stdin("default\nfenced-output\n")
        .assert()
        .failure()
        .stderr(predicates::str::contains("must not contain code fences"));
}

#[test]
fn init_with_ai_fragments_fail_when_provider_fails() {
    let env = TestEnv::new();
    env.setup();
    let project_dir = env.repos_root().join("github.com/icepuma/provider-failure");
    fs::create_dir_all(&project_dir).unwrap();

    env.command()
        .current_dir(&project_dir)
        .env("NANITE_FAKE_PROVIDER_MODE", "fail")
        .arg("init")
        .write_stdin("default\nprovider-failure\n")
        .assert()
        .failure()
        .stderr(predicates::str::contains("codex exited with status"));
}

#[test]
fn init_repairs_only_the_invalid_readme_fragment() {
    let env = TestEnv::new();
    env.setup();
    let project_dir = env.repos_root().join("github.com/icepuma/repair-overview");
    fs::create_dir_all(&project_dir).unwrap();

    env.command()
        .current_dir(&project_dir)
        .env("NANITE_FAKE_PROVIDER_MODE", "repair-overview")
        .arg("init")
        .write_stdin("default\nrepair-overview\n")
        .assert()
        .success()
        .stderr(predicates::str::contains("done Repair failing sections"))
        .stderr(predicates::str::contains("done Re-verify outputs"));

    let provider_log = fs::read_to_string(&env.provider_log).unwrap();
    assert_eq!(provider_log.matches("codex exec").count(), 7);
    assert!(provider_log.contains("<repair>"));

    let readme = fs::read_to_string(project_dir.join("README.md")).unwrap();
    assert!(readme.contains("## Contributing"));
    assert!(readme.contains("## License"));
    assert!(!readme.contains("It keeps files moving. It helps with setup."));
}

#[test]
fn init_fails_when_repaired_readme_is_still_inconsistent() {
    let env = TestEnv::new();
    env.setup();
    let project_dir = env
        .repos_root()
        .join("github.com/icepuma/repair-overview-still-bad");
    fs::create_dir_all(&project_dir).unwrap();

    env.command()
        .current_dir(&project_dir)
        .env("NANITE_FAKE_PROVIDER_MODE", "repair-overview-still-bad")
        .arg("init")
        .write_stdin("default\nrepair-overview-still-bad\n")
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "Overview: overview must be 2 or 3 sentences",
        ));
}

#[test]
fn parallel_ai_fragments_use_unresolved_draft_context() {
    let env = TestEnv::new();
    env.setup();
    let project_dir = env
        .repos_root()
        .join("github.com/icepuma/ordered-fragments");
    fs::create_dir_all(&project_dir).unwrap();

    env.command()
        .current_dir(&project_dir)
        .arg("init")
        .write_stdin("default\nordered-fragments\n")
        .assert()
        .success();

    let provider_log = fs::read_to_string(&env.provider_log).unwrap();
    assert!(provider_log.contains("[[NANITE_FRAGMENT_2]]"));
    assert!(provider_log.contains("{{ai:Write the overview"));
}

#[test]
fn init_with_ai_fragments_use_claude_print_mode_when_configured() {
    let env = TestEnv::new();
    env.setup();
    env.set_agent("claude");
    let project_dir = env.repos_root().join("github.com/icepuma/claude-init");
    fs::create_dir_all(&project_dir).unwrap();

    env.command()
        .current_dir(&project_dir)
        .arg("init")
        .write_stdin("default\nclaude-init\n")
        .assert()
        .success();

    let provider_log = fs::read_to_string(&env.provider_log).unwrap();
    assert!(provider_log.contains("claude -p"));
    assert!(provider_log.contains("--tools "));
    assert!(!provider_log.contains("--plugin-dir"));
    assert_eq!(provider_log.matches("claude -p").count(), 6);
    assert!(
        fs::read_to_string(project_dir.join("README.md"))
            .unwrap()
            .contains("## Usage")
    );
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
        .stdout(predicates::str::contains("CODEX_HOME"))
        .stdout(predicates::str::contains("function jumpto"))
        .stdout(predicates::str::contains("CLAUDE_CODE_PLUGIN_SEED_DIR"))
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
