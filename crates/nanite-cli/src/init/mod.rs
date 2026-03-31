mod agent;
mod debug;
pub mod progress;
mod prompting;

pub use progress::InitProgress;

use crate::context::ContextState;
use crate::util::current_directory;
use agent::{agent_command_name, build_claude_print_command, build_codex_exec_command};
use anyhow::{Context, Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use debug::InitDebugArtifacts;
use nanite_core::{
    AgentKind, AiFragment, AiFragmentRequest, ContextBundle, PreparedBundle, PreparedTemplate,
    ReadmeVerificationReport, TemplateRepository,
};
use nanite_git::configured_author_name;
use prompting::{InitPrompter, InquirePrompter, IoPrompter};
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, IsTerminal};
use std::process::Output;

pub fn command_init(context: &ContextState, force: bool) -> Result<()> {
    let repository = TemplateRepository::load(context.workspace_paths.templates_root())?;
    let current_dir = current_directory()?;
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    if interactive {
        let mut prompter = InquirePrompter;
        return command_init_with_prompter(
            context,
            &repository,
            &current_dir,
            force,
            &mut prompter,
        );
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut prompter = IoPrompter::new(stdin.lock(), stdout.lock());
    command_init_with_prompter(context, &repository, &current_dir, force, &mut prompter)
}

fn command_init_with_prompter(
    context: &ContextState,
    repository: &TemplateRepository,
    current_dir: &Utf8Path,
    force: bool,
    prompter: &mut impl InitPrompter,
) -> Result<()> {
    let selections = repository.selection_labels();
    if selections.is_empty() {
        bail!(
            "no template bundles found in {}",
            context.workspace_paths.templates_root()
        );
    }

    let selection_choice = prompter.choose("Select a template bundle", &selections)?;
    let selection = &selections[selection_choice];
    let bundle = repository.bundle_by_selection_label(selection)?;
    let mut seed_values = BTreeMap::from([(
        "repo_name".to_owned(),
        current_dir.file_name().unwrap_or("project").to_owned(),
    )]);
    if let Some(author) = configured_author_name(current_dir)? {
        seed_values.insert("author".to_owned(), author);
    }
    let prepared = bundle.prepare_with_seed_values(seed_values, prompter)?;
    let mut progress = InitProgress::new(&prepared);
    progress.mark_done(InitProgress::select_step_index(), None);
    progress.mark_done(InitProgress::collect_inputs_step_index(), None);
    let targets = match render_bundle(context, current_dir, &prepared, force, &mut progress) {
        Ok(targets) => {
            progress.finish_success();
            targets
        }
        Err(error) => {
            progress.finish_failure();
            return Err(error);
        }
    };
    for target in targets {
        println!("wrote {target}");
    }
    Ok(())
}

fn render_bundle(
    context: &ContextState,
    cwd: &Utf8Path,
    prepared: &PreparedBundle,
    force: bool,
    progress: &mut InitProgress,
) -> Result<Vec<Utf8PathBuf>> {
    let targets = collect_target_paths(prepared, cwd);
    ensure_targets_ready(&targets, force, progress)?;
    let debug = InitDebugArtifacts::new(cwd, &prepared.name)?;
    if prepared.requires_agent() {
        render_agent_bundle(context, cwd, prepared, progress, &debug)?;
    } else {
        render_static_bundle(cwd, prepared, progress)?;
    }
    Ok(targets)
}

struct GeneratedAiFragment {
    template_source_path: Utf8PathBuf,
    fragment: AiFragment,
    replacement: String,
}

struct RepairAiFragmentJob {
    template: PreparedTemplate,
    fragment: AiFragment,
    notes: Vec<String>,
}

struct VerificationOutcome {
    rendered_by_template: BTreeMap<Utf8PathBuf, String>,
    repair_jobs: Vec<RepairAiFragmentJob>,
}

fn collect_target_paths(prepared: &PreparedBundle, cwd: &Utf8Path) -> Vec<Utf8PathBuf> {
    prepared
        .templates()
        .iter()
        .map(|template| template.target_path(cwd))
        .collect()
}

fn ensure_targets_ready(
    targets: &[Utf8PathBuf],
    force: bool,
    progress: &mut InitProgress,
) -> Result<()> {
    for target in targets {
        if let Err(error) = ensure_target_state(target, force) {
            progress.fail(progress.write_step_index(), &error.to_string());
            return Err(error);
        }
    }
    Ok(())
}

fn render_agent_bundle(
    context: &ContextState,
    cwd: &Utf8Path,
    prepared: &PreparedBundle,
    progress: &mut InitProgress,
    debug: &InitDebugArtifacts,
) -> Result<()> {
    let context_bundles = inspect_bundle_contexts(prepared, cwd, progress, debug)?;
    let mut ai_values_by_template =
        generate_ai_values(context, cwd, prepared, &context_bundles, progress, debug)?;
    let mut verification = verify_templates(
        prepared,
        &context_bundles,
        &ai_values_by_template,
        progress,
        debug,
        "initial",
        true,
    )?;
    if verification.repair_jobs.is_empty() {
        progress.mark_done(progress.repair_step_index(), Some("not needed"));
        progress.mark_done(progress.reverify_step_index(), Some("not needed"));
    } else {
        progress.mark_done(progress.verify_step_index(), Some("needs repair"));
        repair_ai_fragments(
            context,
            cwd,
            &context_bundles,
            &mut ai_values_by_template,
            progress,
            debug,
            verification.repair_jobs,
        )?;
        verification = verify_templates(
            prepared,
            &context_bundles,
            &ai_values_by_template,
            progress,
            debug,
            "repair",
            false,
        )?;
        progress.mark_done(progress.reverify_step_index(), None);
    }
    write_rendered_templates(cwd, prepared, &verification.rendered_by_template, progress)
}

fn render_static_bundle(
    cwd: &Utf8Path,
    prepared: &PreparedBundle,
    progress: &mut InitProgress,
) -> Result<()> {
    let write_step = progress.write_step_index();
    progress.start(write_step, None);
    for template in prepared.templates() {
        let rendered = template.render_final(&BTreeMap::new())?;
        let target = template.target_path(cwd);
        if let Err(error) = write_rendered_output(&target, &rendered) {
            progress.fail(write_step, &error.to_string());
            return Err(error);
        }
    }
    progress.mark_done(write_step, None);
    Ok(())
}

fn inspect_bundle_contexts(
    prepared: &PreparedBundle,
    cwd: &Utf8Path,
    progress: &mut InitProgress,
    debug: &InitDebugArtifacts,
) -> Result<BTreeMap<Utf8PathBuf, ContextBundle>> {
    let inspect_step = progress.inspect_step_index();
    progress.start(inspect_step, None);
    let context_bundles = prepared
        .templates()
        .iter()
        .map(|template| {
            (
                template.source_path.clone(),
                template.build_context_bundle(cwd),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if let Err(error) = debug.write_bundle_context(&context_bundles) {
        progress.fail(inspect_step, &error.to_string());
        return Err(error);
    }
    progress.mark_done(inspect_step, None);
    Ok(context_bundles)
}

fn generate_ai_values(
    context: &ContextState,
    cwd: &Utf8Path,
    prepared: &PreparedBundle,
    context_bundles: &BTreeMap<Utf8PathBuf, ContextBundle>,
    progress: &mut InitProgress,
    debug: &InitDebugArtifacts,
) -> Result<BTreeMap<Utf8PathBuf, BTreeMap<usize, String>>> {
    let mut all_fragments = Vec::new();
    for template in prepared.templates() {
        let context_bundle = context_bundles
            .get(&template.source_path)
            .expect("template context bundle should exist");
        for fragment in template.ai_fragments() {
            let request = template.build_ai_fragment_request(
                cwd,
                context_bundle,
                &fragment,
                &BTreeMap::new(),
                &[],
            );
            all_fragments.push((template, fragment, request));
        }
    }

    let generate_step = progress.generate_step_index();
    let template_count = prepared.templates().len();
    let fragment_count = all_fragments.len();
    progress.start(
        generate_step,
        Some(&format!(
            "{fragment_count} fragment{} across {template_count} file{}",
            if fragment_count == 1 { "" } else { "s" },
            if template_count == 1 { "" } else { "s" }
        )),
    );

    let generated = all_fragments
        .into_par_iter()
        .enumerate()
        .map(|(offset, (template, fragment, request))| {
            let request = request?;
            generate_ai_fragment(
                context,
                debug,
                "generate",
                offset + 1,
                template,
                fragment,
                &request,
            )
        })
        .collect::<Vec<_>>();

    let mut ai_values_by_template = BTreeMap::<Utf8PathBuf, BTreeMap<usize, String>>::new();
    for result in generated {
        match result {
            Ok(generated_fragment) => {
                ai_values_by_template
                    .entry(generated_fragment.template_source_path)
                    .or_default()
                    .insert(
                        generated_fragment.fragment.placeholder.index,
                        generated_fragment.replacement,
                    );
            }
            Err(error) => {
                progress.fail(generate_step, &error.to_string());
                return Err(error);
            }
        }
    }
    progress.mark_done(generate_step, None);
    Ok(ai_values_by_template)
}

fn verify_templates(
    prepared: &PreparedBundle,
    context_bundles: &BTreeMap<Utf8PathBuf, ContextBundle>,
    ai_values_by_template: &BTreeMap<Utf8PathBuf, BTreeMap<usize, String>>,
    progress: &mut InitProgress,
    debug: &InitDebugArtifacts,
    report_prefix: &str,
    collect_repairs: bool,
) -> Result<VerificationOutcome> {
    let verify_step = if collect_repairs {
        progress.verify_step_index()
    } else {
        progress.reverify_step_index()
    };
    progress.start(verify_step, None);

    let mut rendered_by_template = BTreeMap::new();
    let mut repair_jobs = Vec::new();
    for template in prepared.templates() {
        let ai_values = ai_values_by_template
            .get(&template.source_path)
            .cloned()
            .unwrap_or_default();
        let rendered = template.render_final(&ai_values)?;
        if template.is_readme() {
            let context_bundle = context_bundles
                .get(&template.source_path)
                .expect("template context bundle should exist");
            let report = template.verify_readme(&rendered, context_bundle, &ai_values);
            if let Err(error) = debug.write_verifier_report(
                &format!("{report_prefix}-{}", template.output_name),
                &report,
                &rendered,
            ) {
                progress.fail(verify_step, &error.to_string());
                return Err(error);
            }
            if !report.is_valid() {
                if report.has_non_repairable_findings() || !collect_repairs {
                    let summary = format_readme_verifier_messages(&report);
                    progress.fail(verify_step, &summary);
                    return Err(anyhow!(summary));
                }
                repair_jobs.extend(build_repair_jobs(template, &report)?);
            }
        } else if rendered.contains("{{") || rendered.contains("}}") {
            let summary = format!(
                "{} still contains unresolved template placeholders",
                template.output_name
            );
            progress.fail(verify_step, &summary);
            return Err(anyhow!(summary));
        }
        rendered_by_template.insert(template.source_path.clone(), rendered);
    }

    if collect_repairs && repair_jobs.is_empty() {
        progress.mark_done(verify_step, None);
    }

    Ok(VerificationOutcome {
        rendered_by_template,
        repair_jobs,
    })
}

fn repair_ai_fragments(
    context: &ContextState,
    cwd: &Utf8Path,
    context_bundles: &BTreeMap<Utf8PathBuf, ContextBundle>,
    ai_values_by_template: &mut BTreeMap<Utf8PathBuf, BTreeMap<usize, String>>,
    progress: &mut InitProgress,
    debug: &InitDebugArtifacts,
    repair_jobs: Vec<RepairAiFragmentJob>,
) -> Result<()> {
    let repair_step = progress.repair_step_index();
    progress.start(
        repair_step,
        Some(&format!(
            "{} fragment{}",
            repair_jobs.len(),
            if repair_jobs.len() == 1 { "" } else { "s" }
        )),
    );

    let repaired = repair_jobs
        .into_par_iter()
        .enumerate()
        .map(|(offset, job)| {
            let context_bundle = context_bundles
                .get(&job.template.source_path)
                .expect("template context bundle should exist");
            let current_ai = ai_values_by_template
                .get(&job.template.source_path)
                .cloned()
                .unwrap_or_default();
            let request = job.template.build_ai_fragment_request(
                cwd,
                context_bundle,
                &job.fragment,
                &current_ai,
                &job.notes,
            )?;
            generate_ai_fragment(
                context,
                debug,
                "repair",
                offset + 1,
                &job.template,
                job.fragment,
                &request,
            )
        })
        .collect::<Vec<_>>();

    for result in repaired {
        match result {
            Ok(repaired_fragment) => {
                ai_values_by_template
                    .entry(repaired_fragment.template_source_path)
                    .or_default()
                    .insert(
                        repaired_fragment.fragment.placeholder.index,
                        repaired_fragment.replacement,
                    );
            }
            Err(error) => {
                progress.fail(repair_step, &error.to_string());
                return Err(error);
            }
        }
    }
    progress.mark_done(repair_step, None);
    Ok(())
}

fn write_rendered_templates(
    cwd: &Utf8Path,
    prepared: &PreparedBundle,
    rendered_by_template: &BTreeMap<Utf8PathBuf, String>,
    progress: &mut InitProgress,
) -> Result<()> {
    let write_step = progress.write_step_index();
    progress.start(write_step, None);
    for template in prepared.templates() {
        let target = template.target_path(cwd);
        let rendered = rendered_by_template
            .get(&template.source_path)
            .ok_or_else(|| anyhow!("missing rendered output for {}", template.output_name))?;
        if let Err(error) = write_rendered_output(&target, rendered) {
            progress.fail(write_step, &error.to_string());
            return Err(error);
        }
    }
    progress.mark_done(write_step, None);
    Ok(())
}

fn generate_ai_fragment(
    context: &ContextState,
    debug: &InitDebugArtifacts,
    stage: &str,
    ordinal: usize,
    template: &PreparedTemplate,
    fragment: AiFragment,
    request: &AiFragmentRequest,
) -> Result<GeneratedAiFragment> {
    let prompt = build_ai_fragment_prompt(request);
    let label = format!("{}-{}", template.output_name, fragment.label);
    debug.write_fragment_prompt(stage, ordinal, &label, &prompt)?;
    let replacement = run_ai_fragment(context, &prompt)?;
    debug.write_fragment_output(stage, ordinal, &label, &replacement)?;
    Ok(GeneratedAiFragment {
        template_source_path: template.source_path.clone(),
        fragment,
        replacement,
    })
}

fn build_repair_jobs(
    template: &PreparedTemplate,
    report: &ReadmeVerificationReport,
) -> Result<Vec<RepairAiFragmentJob>> {
    let fragments_by_index = template
        .ai_fragments()
        .into_iter()
        .map(|fragment| (fragment.placeholder.index, fragment))
        .collect::<BTreeMap<_, _>>();
    report
        .repairable_fragment_indexes()
        .into_iter()
        .map(|fragment_index| {
            let fragment = fragments_by_index
                .get(&fragment_index)
                .ok_or_else(|| {
                    anyhow!("missing README fragment metadata for repair index {fragment_index}")
                })?
                .clone();
            let notes = report
                .findings
                .iter()
                .filter(|finding| finding.fragment_index == Some(fragment_index))
                .map(|finding| finding.message.clone())
                .collect::<Vec<_>>();
            Ok(RepairAiFragmentJob {
                template: template.clone(),
                fragment,
                notes,
            })
        })
        .collect()
}

fn ensure_target_state(target: &Utf8Path, force: bool) -> Result<()> {
    ensure_file_can_be_written(target, force)?;
    if target.exists() && force {
        remove_existing_file(target)?;
    }
    Ok(())
}

fn ensure_file_can_be_written(path: &Utf8Path, force: bool) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(path.as_std_path())
        .with_context(|| format!("failed to inspect {path}"))?;
    if metadata.file_type().is_dir() {
        bail!("{path} is a directory");
    }
    if !force {
        bail!("{path} already exists; rerun with --force to overwrite");
    }

    Ok(())
}

fn remove_existing_file(path: &Utf8Path) -> Result<()> {
    fs::remove_file(path.as_std_path()).with_context(|| format!("failed to remove {path}"))
}

fn run_ai_fragment(context: &ContextState, prompt: &str) -> Result<String> {
    let mut invocation = match context.config.agent {
        AgentKind::Codex => build_codex_exec_command(context, prompt)?,
        AgentKind::Claude => build_claude_print_command(prompt)?,
    };
    let output = invocation.command.output().with_context(|| {
        format!(
            "failed to launch {}",
            agent_command_name(context.config.agent)
        )
    })?;
    if !output.status.success() {
        if let Some(last_line) = last_agent_output_line(&output) {
            bail!(
                "{} exited with status {}: {}",
                agent_command_name(context.config.agent),
                output.status,
                last_line,
            );
        }
        bail!(
            "{} exited with status {}",
            agent_command_name(context.config.agent),
            output.status
        );
    }

    let output = invocation.output_string(&output)?;
    validate_ai_fragment_output(&output)
}

fn write_rendered_output(target_path: &Utf8Path, contents: &str) -> Result<()> {
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    }
    fs::write(target_path, contents).with_context(|| format!("failed to write {target_path}"))
}

fn build_ai_fragment_prompt(request: &AiFragmentRequest) -> String {
    let mut prompt = format!(
        "<task>\nReplace one AI template fragment in `{}`.\nReturn only the markdown that should replace the active placeholder.\nDo not return explanations, code fences, frontmatter, or a full-document rewrite.\n</task>\n\n<fragment>\n- Index: {}\n- Label: {}\n- Active sentinel: `{}`\n- Prompt: {}\n</fragment>\n\n<contract>\n{}\n</contract>\n",
        request.target_file,
        request.fragment_index + 1,
        request.display_label,
        request.active_sentinel,
        request.fragment_prompt,
        render_fragment_contract(request),
    );
    if !request.repair_notes.is_empty() {
        write!(
            prompt,
            "\n<repair>\n{}\n</repair>\n",
            request
                .repair_notes
                .iter()
                .map(|note| format!("- {note}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
        .expect("writing to String should not fail");
    }
    write!(
        prompt,
        "\n<values>\n{}\n</values>\n\n<repo_brief>\n{}\n</repo_brief>\n\n<context_files>\n{}\n</context_files>\n\n<document>\n```md\n{}\n```\n</document>\n",
        render_values(&request.values),
        render_context_summary(&request.context),
        render_context_snippets(&request.context),
        request.document,
    )
    .expect("writing to String should not fail");
    if let Some(examples) = render_fragment_examples(request) {
        write!(prompt, "\n<examples>\n{examples}\n</examples>\n")
            .expect("writing to String should not fail");
    }
    prompt
}

fn render_fragment_contract(request: &AiFragmentRequest) -> String {
    match request.readme_role {
        Some(nanite_core::ReadmeFragmentRole::Badges) => [
            "- Keep the README house style fixed and factual.",
            "- Return exactly one markdown line or an empty string.",
            "- Only CI and license badges are allowed.",
            "- Use only verified CI/license facts from the repo brief and snippets.",
        ]
        .join("\n"),
        Some(nanite_core::ReadmeFragmentRole::Overview) => [
            "- Keep the README house style fixed and factual.",
            "- Write 2 or 3 sentences only.",
            "- Make the reader curious about the project, explain why it matters, and end with what they can do with it.",
            "- Avoid internal crates, package layout, or implementation breakdowns.",
            "- Use only commands, files, and claims supported by the repo brief and snippets.",
        ]
        .join("\n"),
        Some(nanite_core::ReadmeFragmentRole::QuickStart) => [
            "- Return only markdown bullet lines.",
            "- Write 2 or 3 bullets only.",
            "- Show the fastest verified install, bootstrap, or run path from the repo brief.",
            "- Do not invent prerequisites, scripts, or file paths.",
        ]
        .join("\n"),
        Some(nanite_core::ReadmeFragmentRole::Usage) => [
            "- Return only markdown bullet lines.",
            "- Write 2 or 3 bullets only.",
            "- Focus on what someone can do with the project once it is running.",
            "- Prefer the main user or developer workflow and the most relevant verified commands.",
            "- Do not describe internal crates, packages, or repo layout.",
            "- Do not invent commands, packages, or links.",
        ]
        .join("\n"),
        Some(nanite_core::ReadmeFragmentRole::Tests) => [
            "- Return only markdown bullet lines.",
            "- Write 1 to 3 bullets only.",
            "- Use only the verified test or check command from the repo brief.",
            "- If no verified test command exists, return exactly one bullet that says no verified test command was found.",
        ]
        .join("\n"),
        None => [
            "- Return only replacement markdown for the active fragment.",
            "- Do not return headings unless the template prompt explicitly asks for them.",
            "- Do not return code fences, frontmatter, or a full-document rewrite.",
            "- Use only facts supported by the repo brief and context snippets.",
        ]
        .join("\n"),
    }
}

fn render_fragment_examples(request: &AiFragmentRequest) -> Option<String> {
    let examples = match request.readme_role {
        Some(nanite_core::ReadmeFragmentRole::QuickStart) => vec![
            "<example>\n- Install dependencies with pnpm install.\n- Start the project with pnpm dev.\n</example>",
        ],
        Some(nanite_core::ReadmeFragmentRole::Usage) => vec![
            "<example>\n- Run nanite init to generate the next project file from the current repository.\n- Use nanite repo clone and jumpto to move quickly between repositories in the workspace.\n</example>",
        ],
        Some(nanite_core::ReadmeFragmentRole::Tests) => vec![
            "<example>\n- Run cargo test -q from the repository root.\n</example>",
            "<example>\n- No verified test command was found.\n</example>",
        ],
        _ => Vec::new(),
    };
    if examples.is_empty() {
        None
    } else {
        Some(examples.join("\n"))
    }
}

fn render_values(values: &BTreeMap<String, String>) -> String {
    if values.is_empty() {
        return "(none)".to_owned();
    }

    values
        .iter()
        .map(|(name, value)| {
            let rendered = if value.trim().is_empty() {
                "(blank)"
            } else {
                value.trim()
            };
            format!("- {name}: {rendered}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_context_summary(context: &ContextBundle) -> String {
    if context.summary_lines.is_empty() {
        return "(none)".to_owned();
    }

    context.summary_lines.join("\n")
}

pub fn render_context_snippets(context: &ContextBundle) -> String {
    if context.snippets.is_empty() {
        return "(none)".to_owned();
    }

    context
        .snippets
        .iter()
        .map(|snippet| {
            format!(
                "<snippet path=\"{}\">\n{}\n</snippet>",
                snippet.path, snippet.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn validate_ai_fragment_output(output: &str) -> Result<String> {
    let trimmed = output.trim();
    if trimmed.contains("```") {
        bail!("agent output must not contain code fences");
    }
    if trimmed.starts_with("---\n") || trimmed == "---" {
        bail!("agent output must not contain frontmatter");
    }
    Ok(trimmed.to_owned())
}

fn last_agent_output_line(output: &Output) -> Option<String> {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn format_readme_verifier_messages(report: &ReadmeVerificationReport) -> String {
    report.render_messages().join("; ")
}
