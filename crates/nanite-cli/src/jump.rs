use crate::context::ContextState;
use crate::util::load_registry;
use anyhow::{Context, Result, anyhow};
use camino::Utf8PathBuf;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn command_jumpto(context: &ContextState, query: Option<&str>) -> Result<Option<Utf8PathBuf>> {
    let registry = load_registry(&context.app_paths)?;
    let candidates = render_jumpto_candidates(registry.entries());
    if candidates.is_empty() {
        return Ok(None);
    }

    let mut command = Command::new(&context.fzf_binary);
    command
        .args(jumpto_fzf_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some(query) = query {
        command.args(["-q", query]);
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn {}", context.fzf_binary))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open fzf stdin"))?;
        stdin.write_all(candidates.join("\n").as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Ok(None);
    }

    let selected = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if selected.is_empty() {
        return Ok(None);
    }

    let path = selected
        .split('\t')
        .nth(1)
        .ok_or_else(|| anyhow!("fzf returned an invalid selection"))?;
    Ok(Some(Utf8PathBuf::from(path)))
}

pub const fn jumpto_fzf_args() -> [&'static str; 14] {
    [
        "--select-1",
        "--exit-0",
        "--delimiter=\t",
        "--with-nth=1",
        "--layout=reverse",
        "--height=70%",
        "--border",
        "--prompt=jumpto > ",
        "--pointer=›",
        "--marker=•",
        "--header=Open a repository",
        "--info=inline-right",
        "--preview-window=hidden",
        "--color=border:8,header:12,prompt:10,pointer:14,marker:11,info:8,spinner:10,hl:14,hl+:14",
    ]
}

pub fn render_jumpto_candidates(records: Vec<&nanite_core::ProjectRecord>) -> Vec<String> {
    let name_width = records
        .iter()
        .map(|record| record.name.chars().count())
        .max()
        .unwrap_or_default();

    records
        .into_iter()
        .map(|record| {
            let repo = format!("{}/{}", record.host, record.repo_path);
            let display = format!("{:<width$}  {}", record.name, repo, width = name_width);
            format!("{display}\t{}", record.path)
        })
        .collect()
}

pub fn command_complete_jumpto(context: &ContextState) -> Result<()> {
    let registry = load_registry(&context.app_paths)?;
    for record in registry.entries() {
        println!("{}/{}\t{}", record.host, record.repo_path, record.path);
    }
    Ok(())
}

pub fn command_complete_repo_remove(context: &ContextState) -> Result<()> {
    let registry = load_registry(&context.app_paths)?;
    for record in registry.entries() {
        println!("{}/{}\t{}", record.host, record.repo_path, record.path);
    }
    Ok(())
}
