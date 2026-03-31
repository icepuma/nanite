use super::{render_context_snippets, render_context_summary};
use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use nanite_core::{ContextBundle, ReadmeVerificationReport};
use std::collections::BTreeMap;
use std::fs;

pub(super) struct InitDebugArtifacts {
    root: Option<Utf8PathBuf>,
}

impl InitDebugArtifacts {
    pub(super) fn new(cwd: &Utf8Path, output_name: &str) -> Result<Self> {
        let enabled = std::env::var_os("NANITE_INIT_DEBUG").is_some();
        if !enabled {
            return Ok(Self { root: None });
        }

        let slug = output_name
            .chars()
            .map(|character| match character {
                'a'..='z' | 'A'..='Z' | '0'..='9' => character,
                _ => '-',
            })
            .collect::<String>();
        let root = cwd.join(".nanite/init/debug").join(slug);
        fs::create_dir_all(root.as_std_path())
            .with_context(|| format!("failed to create {root}"))?;
        Ok(Self { root: Some(root) })
    }

    pub(super) fn write_bundle_context(
        &self,
        contexts: &BTreeMap<Utf8PathBuf, ContextBundle>,
    ) -> Result<()> {
        let rendered_contexts = contexts
            .iter()
            .map(|(path, context)| {
                format!(
                    "# {}\n\n{}\n\n{}",
                    path,
                    render_context_summary(context),
                    render_context_snippets(context)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        self.write("context.txt", &rendered_contexts)
    }

    pub(super) fn write_fragment_prompt(
        &self,
        stage: &str,
        ordinal: usize,
        label: &str,
        prompt: &str,
    ) -> Result<()> {
        self.write(
            &format!("{stage}-{ordinal:02}-{}-prompt.txt", slugify(label)),
            prompt,
        )
    }

    pub(super) fn write_fragment_output(
        &self,
        stage: &str,
        ordinal: usize,
        label: &str,
        output: &str,
    ) -> Result<()> {
        self.write(
            &format!("{stage}-{ordinal:02}-{}-output.md", slugify(label)),
            output,
        )
    }

    pub(super) fn write_verifier_report(
        &self,
        stage: &str,
        report: &ReadmeVerificationReport,
        rendered: &str,
    ) -> Result<()> {
        let messages = if report.findings.is_empty() {
            "valid".to_owned()
        } else {
            report.render_messages().join("\n")
        };
        self.write(
            &format!("verify-{stage}.txt"),
            &format!("{messages}\n\n-----\n\n{rendered}"),
        )
    }

    fn write(&self, name: &str, contents: &str) -> Result<()> {
        let Some(root) = &self.root else {
            return Ok(());
        };
        fs::write(root.join(name), contents)
            .with_context(|| format!("failed to write {}", root.join(name)))
    }
}

fn slugify(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' => character.to_ascii_lowercase(),
            _ => '-',
        })
        .collect::<String>()
}
