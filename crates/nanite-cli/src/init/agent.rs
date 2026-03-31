use crate::context::ContextState;
use anyhow::{Context, Result};
use nanite_core::AgentKind;
use std::fs;
use std::process::{Command, Output, Stdio};
use tempfile::{NamedTempFile, TempDir};

pub(super) struct TextAgentCommand {
    pub(super) command: Command,
    _workdir: TempDir,
    output_file: Option<NamedTempFile>,
}

impl TextAgentCommand {
    pub(super) fn output_string(&self, output: &Output) -> Result<String> {
        if let Some(output_file) = &self.output_file {
            return fs::read_to_string(output_file.path())
                .with_context(|| format!("failed to read {}", output_file.path().display()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

pub(super) const fn agent_command_name(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Codex => "codex",
        AgentKind::Claude => "claude",
    }
}

pub(super) fn build_codex_exec_command(
    context: &ContextState,
    prompt: &str,
) -> Result<TextAgentCommand> {
    let workdir = tempfile::tempdir().context("failed to create Codex temp workspace")?;
    let output_file =
        NamedTempFile::new_in(workdir.path()).context("failed to create Codex output file")?;

    let mut command = Command::new("codex");
    command
        .current_dir(workdir.path())
        .env("CODEX_HOME", context.app_paths.codex_home_root().as_str())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("exec")
        .arg("--model")
        .arg("gpt-5.4-mini")
        .arg("--cd")
        .arg(workdir.path())
        .arg("--skip-git-repo-check")
        .arg("--sandbox")
        .arg("read-only")
        .arg("--ephemeral")
        .arg("-o")
        .arg(output_file.path())
        .arg(prompt);

    Ok(TextAgentCommand {
        command,
        _workdir: workdir,
        output_file: Some(output_file),
    })
}

pub(super) fn build_claude_print_command(prompt: &str) -> Result<TextAgentCommand> {
    let workdir = tempfile::tempdir().context("failed to create Claude temp workspace")?;
    let mut command = Command::new("claude");
    command
        .current_dir(workdir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("-p")
        .arg("--no-session-persistence")
        .arg("--output-format")
        .arg("text")
        .arg("--tools")
        .arg("")
        .arg(prompt);

    Ok(TextAgentCommand {
        command,
        _workdir: workdir,
        output_file: None,
    })
}
