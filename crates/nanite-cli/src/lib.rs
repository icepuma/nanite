#![allow(clippy::missing_errors_doc)]

use anyhow::{Context, Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Generator, Shell, generate};
use indicatif::{ProgressBar, ProgressStyle};
use inquire::ui::{Attributes, Color, RenderConfig, StyleSheet, Styled};
use inquire::{Confirm, Select, Text};
use nanite_agents::{
    FileDiff, SyncAction, SyncReason, SyncReport, SyncTarget, load_skills, sync_claude, sync_codex,
};
use nanite_core::{
    AgentKind, AiFragment, AiFragmentRequest, AppPaths, Config, ContextBundle, PreparedBundle,
    PreparedTemplate, Prompter, ReadmeVerificationReport, Registry, TemplateRepository,
    TextPlaceholder, WorkspacePaths,
};
use nanite_git::{
    clone_repo, configured_author_name, import_repo, remove_repo, resolve_repo_remove_target,
    scan_workspace,
};
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::process::{Command, Output, Stdio};
use std::time::Duration;
use tempfile::{NamedTempFile, TempDir};

#[derive(Debug, Parser)]
#[command(
    name = "nanite",
    about = "Manage local repositories in an AI-first workspace",
    long_about = None,
    after_help = "Examples:\n  nanite setup ~/workspace\n  nanite init\n  nanite repo clone github.com/icepuma/nanite\n  nanite repo refresh\n  nanite skill sync codex --apply\n  nanite jumpto nanite",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(about = "Create and configure a Nanite workspace", long_about = None)]
    Setup {
        #[arg(
            value_name = "PATH",
            help = "Empty directory to initialize as the Nanite workspace"
        )]
        path: String,
    },
    #[command(about = "Render a template into the current repository", long_about = None)]
    Init {
        #[arg(long, help = "Overwrite an existing target file")]
        force: bool,
    },
    #[command(about = "Manage repositories in the workspace", long_about = None)]
    Repo {
        #[command(subcommand)]
        command: RepoCommands,
    },
    #[command(about = "Sync Nanite-managed skills", long_about = None)]
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },
    #[command(name = "jumpto")]
    #[command(about = "Choose a workspace repository and print its path", long_about = None)]
    Jumpto {
        #[arg(
            value_name = "QUERY",
            help = "Initial search text for the repository picker"
        )]
        query: Option<String>,
    },
    #[command(about = "Print shell integration for Nanite", long_about = None)]
    Shell {
        #[command(subcommand)]
        command: ShellCommands,
    },
    #[command(hide = true, name = "__complete-jumpto")]
    CompleteJumpto,
    #[command(hide = true, name = "__complete-repo-remove")]
    CompleteRepoRemove,
}

#[derive(Debug, Subcommand)]
#[command(
    about = "Manage repositories in the workspace",
    long_about = None,
    after_help = "Examples:\n  nanite repo clone github.com/icepuma/nanite\n  nanite repo remove --yes github.com/icepuma/nanite\n  nanite repo refresh"
)]
enum RepoCommands {
    #[command(about = "Clone a repository into the workspace", long_about = None)]
    Clone {
        #[arg(value_name = "REMOTE", help = "Git remote or repository spec to clone")]
        remote: String,
        #[arg(long, help = "Overwrite an existing destination directory")]
        force: bool,
    },
    #[command(about = "Remove a repository from the workspace", long_about = None)]
    Remove {
        #[arg(
            value_name = "TARGET",
            help = "Workspace repo target, remote, or absolute path to remove"
        )]
        target: String,
        #[arg(long, short = 'y', help = "Skip the confirmation prompt")]
        yes: bool,
    },
    #[command(about = "Import an existing local repository into the workspace", long_about = None)]
    Import {
        #[arg(
            value_name = "SOURCE",
            help = "Existing repository directory to import"
        )]
        source: String,
    },
    #[command(about = "Refresh the registry from repositories under the workspace", long_about = None)]
    Refresh,
}

#[derive(Debug, Clone, Copy, Subcommand)]
#[command(
    about = "Sync Nanite-managed skills",
    long_about = None,
    after_help = "Examples:\n  nanite skill sync codex\n  nanite skill sync codex --apply"
)]
enum SkillCommands {
    #[command(about = "Sync bundled skills into an agent install location", long_about = None)]
    Sync {
        #[arg(value_name = "AGENT", help = "Agent to sync skills for")]
        provider: ProviderArg,
        #[arg(long, help = "Write changes instead of showing a dry run")]
        apply: bool,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ProviderArg {
    Codex,
    Claude,
}

#[derive(Debug, Clone, Copy, Subcommand)]
#[command(
    about = "Print shell integration for Nanite",
    long_about = None,
    after_help = "Example:\n  nanite shell init fish | source"
)]
enum ShellCommands {
    #[command(about = "Print shell setup for wrappers and completions", long_about = None)]
    Init {
        #[arg(value_enum, value_name = "SHELL", help = "Shell to generate setup for")]
        shell: ShellArg,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ShellArg {
    Fish,
}

#[must_use]
pub fn build_cli() -> clap::Command {
    Cli::command()
}

pub fn run() -> Result<i32> {
    run_with(Cli::parse())
}

fn run_with(cli: Cli) -> Result<i32> {
    let app_paths = AppPaths::discover()?;
    let git_binary = std::env::var("NANITE_GIT").unwrap_or_else(|_| "git".to_owned());
    let fzf_binary = std::env::var("NANITE_FZF").unwrap_or_else(|_| "fzf".to_owned());
    let load_context = || -> Result<ContextState> {
        let config = Config::load(&app_paths)?;
        let workspace_paths = config.workspace_paths();
        Ok(ContextState {
            app_paths: app_paths.clone(),
            config,
            workspace_paths,
            git_binary: git_binary.clone(),
            fzf_binary: fzf_binary.clone(),
        })
    };

    match cli.command {
        Commands::Setup { path } => {
            command_setup(&app_paths, &bundled_content_root()?, &path)?;
            Ok(0)
        }
        Commands::Init { force } => {
            let context = load_context()?;
            command_init(&context, force)?;
            Ok(0)
        }
        Commands::Repo { command } => {
            let context = load_context()?;
            command_repo(&context, command)?;
            Ok(0)
        }
        Commands::Skill { command } => {
            let context = load_context()?;
            command_skill(&context, command)?;
            Ok(0)
        }
        Commands::Jumpto { query } => {
            let context = load_context()?;
            Ok(
                command_jumpto(&context, query.as_deref())?.map_or(1, |path| {
                    println!("{path}");
                    0
                }),
            )
        }
        Commands::Shell { command } => {
            let context = load_context()?;
            command_shell(&context, command);
            Ok(0)
        }
        Commands::CompleteJumpto => {
            let context = load_context()?;
            command_complete_jumpto(&context)?;
            Ok(0)
        }
        Commands::CompleteRepoRemove => {
            let context = load_context()?;
            command_complete_repo_remove(&context)?;
            Ok(0)
        }
    }
}

struct ContextState {
    app_paths: AppPaths,
    config: Config,
    workspace_paths: WorkspacePaths,
    git_binary: String,
    fzf_binary: String,
}

fn command_setup(app_paths: &AppPaths, bundled_content_root: &Utf8Path, path: &str) -> Result<()> {
    if let Some(config) = Config::load_optional(app_paths)? {
        anyhow::bail!(
            "nanite is already configured for {}; remove {} to reconfigure",
            config.workspace_root,
            app_paths.config_file()
        );
    }

    let workspace_root = resolve_cli_path(path)?;
    ensure_setup_target_is_empty(&workspace_root)?;
    fs::create_dir_all(workspace_root.as_std_path())
        .with_context(|| format!("failed to create {workspace_root}"))?;
    let workspace_root = canonicalize_utf8(&workspace_root)?;
    let workspace_paths = WorkspacePaths::new(workspace_root.clone());

    fs::create_dir_all(workspace_paths.templates_root())
        .with_context(|| format!("failed to create {}", workspace_paths.templates_root()))?;
    fs::create_dir_all(workspace_paths.skills_root())
        .with_context(|| format!("failed to create {}", workspace_paths.skills_root()))?;
    fs::create_dir_all(workspace_paths.repos_root())
        .with_context(|| format!("failed to create {}", workspace_paths.repos_root()))?;

    copy_dir_contents(
        &bundled_content_root.join("templates"),
        workspace_paths.templates_root(),
    )?;
    copy_dir_contents(
        &bundled_content_root.join("skills"),
        workspace_paths.skills_root(),
    )?;

    let defaults = Config::default_for(app_paths)?;
    Config {
        workspace_root: workspace_root.clone(),
        agent: defaults.agent,
    }
    .save(app_paths)?;

    println!("configured {workspace_root}");
    Ok(())
}

fn command_init(context: &ContextState, force: bool) -> Result<()> {
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

fn render_values(values: &std::collections::BTreeMap<String, String>) -> String {
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

fn render_context_summary(context: &ContextBundle) -> String {
    if context.summary_lines.is_empty() {
        return "(none)".to_owned();
    }

    context.summary_lines.join("\n")
}

fn render_context_snippets(context: &ContextBundle) -> String {
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

struct TextAgentCommand {
    command: Command,
    _workdir: TempDir,
    output_file: Option<NamedTempFile>,
}

impl TextAgentCommand {
    fn output_string(&self, output: &Output) -> Result<String> {
        if let Some(output_file) = &self.output_file {
            return fs::read_to_string(output_file.path())
                .with_context(|| format!("failed to read {}", output_file.path().display()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

enum InitProgressMode {
    Tty(ProgressBar),
    Plain,
}

#[derive(Clone, Copy)]
enum InitStepState {
    Pending,
    Active,
    Done,
    Failed,
}

struct InitStep {
    label: String,
    status: InitStepState,
    detail: Option<String>,
}

struct InitProgress {
    mode: InitProgressMode,
    steps: Vec<InitStep>,
    inspect_index: Option<usize>,
    generate_index: Option<usize>,
    verify_index: Option<usize>,
    repair_index: Option<usize>,
    reverify_index: Option<usize>,
    write_index: usize,
}

impl InitProgress {
    fn new(prepared: &PreparedBundle) -> Self {
        let mut steps = vec![
            InitStep {
                label: "Select bundle".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            },
            InitStep {
                label: "Collect text inputs".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            },
        ];
        let mut inspect_index = None;
        let mut generate_index = None;
        let mut verify_index = None;
        let mut repair_index = None;
        let mut reverify_index = None;

        if prepared.requires_agent() {
            inspect_index = Some(steps.len());
            steps.push(InitStep {
                label: "Inspect repository context".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            });
            generate_index = Some(steps.len());
            steps.push(InitStep {
                label: "Generate AI fragments".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            });
            verify_index = Some(steps.len());
            steps.push(InitStep {
                label: "Verify outputs".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            });
            repair_index = Some(steps.len());
            steps.push(InitStep {
                label: "Repair failing sections".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            });
            reverify_index = Some(steps.len());
            steps.push(InitStep {
                label: "Re-verify outputs".to_owned(),
                status: InitStepState::Pending,
                detail: None,
            });
        }

        let write_index = steps.len();
        steps.push(InitStep {
            label: "Write files".to_owned(),
            status: InitStepState::Pending,
            detail: None,
        });

        let mode = if io::stderr().is_terminal() {
            let bar = ProgressBar::new(steps.len() as u64);
            let style = ProgressStyle::with_template(
                "{spinner:.cyan} {prefix:.bold.dim} {wide_bar:.cyan/blue} {msg}",
            )
            .expect("progress template should be valid")
            .progress_chars("█▉▊▋▌▍▎▏ ")
            .tick_strings(&["⠋", "⠙", "⠸", "⠴", "⠦", "⠇"]);
            bar.set_style(style);
            bar.enable_steady_tick(Duration::from_millis(100));
            InitProgressMode::Tty(bar)
        } else {
            InitProgressMode::Plain
        };

        let progress = Self {
            mode,
            steps,
            inspect_index,
            generate_index,
            verify_index,
            repair_index,
            reverify_index,
            write_index,
        };
        progress.render();
        progress
    }

    const fn select_step_index() -> usize {
        0
    }

    const fn collect_inputs_step_index() -> usize {
        1
    }

    fn inspect_step_index(&self) -> usize {
        self.inspect_index.unwrap_or(self.write_index)
    }

    fn generate_step_index(&self) -> usize {
        self.generate_index.unwrap_or(self.write_index)
    }

    fn verify_step_index(&self) -> usize {
        self.verify_index.unwrap_or(self.write_index)
    }

    fn repair_step_index(&self) -> usize {
        self.repair_index.unwrap_or(self.write_index)
    }

    fn reverify_step_index(&self) -> usize {
        self.reverify_index.unwrap_or(self.write_index)
    }

    const fn write_step_index(&self) -> usize {
        self.write_index
    }

    fn mark_done(&mut self, index: usize, detail: Option<&str>) {
        self.steps[index].status = InitStepState::Done;
        self.steps[index].detail = detail.map(ToOwned::to_owned);
        self.render();
    }

    fn start(&mut self, index: usize, detail: Option<&str>) {
        self.steps[index].status = InitStepState::Active;
        self.steps[index].detail = detail.map(ToOwned::to_owned);
        self.render();
    }

    fn fail(&mut self, index: usize, detail: &str) {
        self.steps[index].status = InitStepState::Failed;
        self.steps[index].detail = Some(detail.to_owned());
        self.render();
    }

    fn finish_success(self) {
        match self.mode {
            InitProgressMode::Tty(progress) => progress.finish_and_clear(),
            InitProgressMode::Plain => {}
        }
    }

    fn finish_failure(self) {
        let rendered = self.rendered();
        match self.mode {
            InitProgressMode::Tty(progress) => progress.abandon_with_message(rendered),
            InitProgressMode::Plain => {}
        }
    }

    fn render(&self) {
        match &self.mode {
            InitProgressMode::Tty(progress) => {
                progress.set_position(self.tty_position() as u64);
                progress.set_prefix(self.tty_prefix());
                progress.set_message(self.tty_message());
            }
            InitProgressMode::Plain => {
                eprintln!("{}", self.last_milestone());
            }
        }
    }

    fn rendered(&self) -> String {
        if let Some(step) = self
            .steps
            .iter()
            .find(|step| matches!(step.status, InitStepState::Failed))
        {
            return Self::render_step_message("failed", step);
        }
        if let Some(step) = self
            .steps
            .iter()
            .find(|step| matches!(step.status, InitStepState::Active))
        {
            return Self::render_step_message("working", step);
        }
        if let Some(step) = self
            .steps
            .iter()
            .rfind(|step| matches!(step.status, InitStepState::Done))
        {
            return Self::render_step_message("done", step);
        }

        "waiting".to_owned()
    }

    fn last_milestone(&self) -> String {
        let step = self
            .steps
            .iter()
            .rev()
            .find(|step| !matches!(step.status, InitStepState::Pending))
            .unwrap_or(&self.steps[0]);
        let verb = match step.status {
            InitStepState::Pending => "pending",
            InitStepState::Active => "active",
            InitStepState::Done => "done",
            InitStepState::Failed => "failed",
        };
        step.detail.as_ref().map_or_else(
            || format!("{verb} {}", step.label),
            |detail| format!("{verb} {}: {detail}", step.label),
        )
    }

    fn completed_steps(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| matches!(step.status, InitStepState::Done))
            .count()
    }

    fn tty_position(&self) -> usize {
        let completed = self.completed_steps();
        if self
            .steps
            .iter()
            .any(|step| matches!(step.status, InitStepState::Active))
        {
            (completed + 1).min(self.steps.len())
        } else {
            completed
        }
    }

    fn tty_prefix(&self) -> String {
        let index = self
            .current_step_index()
            .map_or(1, |index| index.saturating_add(1));
        format!("step {index}/{}", self.steps.len())
    }

    fn tty_message(&self) -> String {
        let Some(index) = self.current_step_index() else {
            return "Waiting for work".to_owned();
        };
        let step = &self.steps[index];
        let status = match step.status {
            InitStepState::Pending => "Queued",
            InitStepState::Active => "Working",
            InitStepState::Done => "Done",
            InitStepState::Failed => "Failed",
        };
        let detail = step
            .detail
            .as_deref()
            .map(Self::truncate_progress_detail)
            .filter(|detail| !detail.is_empty());
        detail.map_or_else(
            || format!("{status} · {}", step.label),
            |detail| format!("{status} · {} · {detail}", step.label),
        )
    }

    fn current_step_index(&self) -> Option<usize> {
        self.steps
            .iter()
            .position(|step| matches!(step.status, InitStepState::Failed))
            .or_else(|| {
                self.steps
                    .iter()
                    .position(|step| matches!(step.status, InitStepState::Active))
            })
            .or_else(|| {
                self.steps
                    .iter()
                    .rposition(|step| matches!(step.status, InitStepState::Done))
            })
    }

    fn truncate_progress_detail(detail: &str) -> String {
        const MAX_CHARS: usize = 72;
        let compact = detail.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut truncated = compact.chars().take(MAX_CHARS).collect::<String>();
        if compact.chars().count() > MAX_CHARS {
            truncated.push('…');
        }
        truncated
    }

    fn render_step_message(prefix: &str, step: &InitStep) -> String {
        step.detail.as_ref().map_or_else(
            || format!("{prefix} {}", step.label),
            |detail| format!("{prefix} {}: {detail}", step.label),
        )
    }
}

struct InitDebugArtifacts {
    root: Option<Utf8PathBuf>,
}

impl InitDebugArtifacts {
    fn new(cwd: &Utf8Path, output_name: &str) -> Result<Self> {
        let enabled = std::env::var_os("NANITE_INIT_DEBUG").is_some();
        if !enabled {
            return Ok(Self { root: None });
        }

        let slug = output_name
            .chars()
            .map(|char| match char {
                'a'..='z' | 'A'..='Z' | '0'..='9' => char,
                _ => '-',
            })
            .collect::<String>();
        let root = cwd.join(".nanite/init/debug").join(slug);
        fs::create_dir_all(root.as_std_path())
            .with_context(|| format!("failed to create {root}"))?;
        Ok(Self { root: Some(root) })
    }

    fn write_bundle_context(&self, contexts: &BTreeMap<Utf8PathBuf, ContextBundle>) -> Result<()> {
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

    fn write_fragment_prompt(
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

    fn write_fragment_output(
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

    fn write_verifier_report(
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
        .map(|char| match char {
            'a'..='z' | 'A'..='Z' | '0'..='9' => char.to_ascii_lowercase(),
            _ => '-',
        })
        .collect::<String>()
}

fn build_codex_exec_command(context: &ContextState, prompt: &str) -> Result<TextAgentCommand> {
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

fn build_claude_print_command(prompt: &str) -> Result<TextAgentCommand> {
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

fn command_repo(context: &ContextState, command: RepoCommands) -> Result<()> {
    let mut registry = load_registry(&context.app_paths)?;

    match command {
        RepoCommands::Clone { remote, force } => {
            let progress = clone_progress_bar(&remote);
            let result = clone_repo(
                context.workspace_paths.repos_root(),
                &remote,
                force,
                progress.clone(),
            );
            if let Some(progress) = &progress {
                progress.finish_and_clear();
            }
            let record = result?;
            println!("cloned {}", record.path);
            registry.upsert(record);
        }
        RepoCommands::Import { source } => {
            let source = resolve_cli_path(&source)?;
            let record = import_repo(
                context.workspace_paths.repos_root(),
                &source,
                &context.git_binary,
            )?;
            println!("imported {}", record.path);
            registry.upsert(record);
        }
        RepoCommands::Remove { target, yes } => {
            let destination =
                resolve_repo_remove_target(context.workspace_paths.repos_root(), &target)?;
            confirm_repo_removal(&destination, yes)?;
            let removed_path = remove_repo(context.workspace_paths.repos_root(), &target)?;
            println!("removed {removed_path}");
            registry.remove_path(&removed_path);
        }
        RepoCommands::Refresh => {
            let records =
                scan_workspace(&context.git_binary, context.workspace_paths.repos_root())?;
            let count = records.len();
            for record in records {
                registry.upsert(record);
            }
            println!("refreshed {count} repositories");
        }
    }

    registry.save(&context.app_paths.registry_file())
}

fn clone_progress_bar(remote: &str) -> Option<ProgressBar> {
    if !io::stdout().is_terminal() {
        return None;
    }

    let bar = ProgressBar::new(0);
    let style = ProgressStyle::with_template(
        "{spinner:.cyan} Cloning {msg} [{wide_bar:.cyan/blue}] {pos}/{len}",
    )
    .unwrap_or_else(|_| ProgressStyle::default_bar())
    .progress_chars("=> ");
    bar.set_style(style);
    bar.set_message(remote.to_owned());
    Some(bar)
}

fn confirm_repo_removal(path: &Utf8Path, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }

    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        let confirmed = Confirm::new(&format!("Remove repository at {path}?"))
            .with_render_config(inquire_render_config())
            .with_default(false)
            .prompt()
            .with_context(|| format!("failed to confirm removal of {path}"))?;
        if confirmed {
            return Ok(());
        }
        bail!("aborted removal of {path}");
    }

    bail!("repo remove requires confirmation; rerun with --yes");
}

fn command_skill(context: &ContextState, command: SkillCommands) -> Result<()> {
    match command {
        SkillCommands::Sync { provider, apply } => {
            let skills = load_skills(context.workspace_paths.skills_root())?;
            let report = match provider {
                ProviderArg::Codex => sync_codex(
                    &skills,
                    &context.app_paths.codex_render_root(),
                    &context.app_paths.codex_skills_root(),
                    apply,
                )?,
                ProviderArg::Claude => {
                    let seed_root = context.app_paths.claude_plugin_seed_root();
                    sync_claude(&skills, std::slice::from_ref(&seed_root), apply)?
                }
            };
            print_sync_report(provider, apply, &report);
        }
    }

    Ok(())
}

fn command_jumpto(context: &ContextState, query: Option<&str>) -> Result<Option<Utf8PathBuf>> {
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

const fn jumpto_fzf_args() -> [&'static str; 14] {
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

fn render_jumpto_candidates(records: Vec<&nanite_core::ProjectRecord>) -> Vec<String> {
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

fn command_shell(context: &ContextState, command: ShellCommands) {
    match command {
        ShellCommands::Init {
            shell: ShellArg::Fish,
        } => {
            print!("{}", render_fish_init(context));
        }
    }
}

fn command_complete_jumpto(context: &ContextState) -> Result<()> {
    let registry = load_registry(&context.app_paths)?;
    for record in registry.entries() {
        println!("{}/{}\t{}", record.host, record.repo_path, record.path);
    }
    Ok(())
}

fn command_complete_repo_remove(context: &ContextState) -> Result<()> {
    let registry = load_registry(&context.app_paths)?;
    for record in registry.entries() {
        println!("{}/{}\t{}", record.host, record.repo_path, record.path);
    }
    Ok(())
}

fn render_fish_init(context: &ContextState) -> String {
    let escaped_codex_home = escape_fish_string(context.app_paths.codex_home_root().as_str());
    let escaped_seed_dirs =
        escape_fish_string(context.app_paths.claude_plugin_seed_root().as_str());
    let completions = generate_completion_script(Shell::Fish);

    format!(
        "set -gx CODEX_HOME \"{escaped_codex_home}\"\n\
set -gx CLAUDE_CODE_PLUGIN_SEED_DIR \"{escaped_seed_dirs}\"\n\
function jumpto --description 'cd into a Nanite repository'\n\
    set -l destination (nanite jumpto $argv)\n\
    or return $status\n\
    if test -n \"$destination\"\n\
        cd \"$destination\"\n\
    end\n\
end\n\
{completions}\n\
complete -c jumpto -f -a '(nanite __complete-jumpto)'\n\
complete -c nanite -n '__fish_seen_subcommand_from repo; and __fish_seen_subcommand_from remove' -f -a '(nanite __complete-repo-remove)'\n"
    )
}

fn generate_completion_script<G>(shell: G) -> String
where
    G: Generator,
{
    let mut command = build_cli();
    let mut buffer = Vec::new();
    generate(shell, &mut command, "nanite", &mut buffer);
    String::from_utf8(buffer).expect("clap completion output is valid UTF-8")
}

fn print_sync_report(provider: ProviderArg, apply: bool, report: &SyncReport) {
    let theme = CliTheme::detect();
    let title = if apply {
        format!("sync {} skills", provider.as_str())
    } else {
        format!("sync {} skills (dry run)", provider.as_str())
    };
    let create_count = report
        .items
        .iter()
        .filter(|item| item.action == SyncAction::Create)
        .count();
    let update_count = report
        .items
        .iter()
        .filter(|item| item.action == SyncAction::Override)
        .count();
    let ok_count = report
        .items
        .iter()
        .filter(|item| item.action == SyncAction::Unchanged)
        .count();

    println!("{}", theme.bold(&title));
    println!(
        "{} {}  {} {}  {} {}",
        theme.green(&create_count.to_string()),
        theme.dim("create"),
        theme.yellow(&update_count.to_string()),
        theme.dim("update"),
        theme.blue(&ok_count.to_string()),
        theme.dim("ok"),
    );

    for item in &report.items {
        println!();
        print_sync_item(item, &theme);
    }
}

fn print_sync_item(item: &nanite_agents::SyncItem, theme: &CliTheme) {
    println!(
        "{} {}",
        format_action_badge(item.action, theme),
        theme.bold(&item.slug)
    );

    for target in &item.targets {
        print_sync_target(target, theme);
    }
}

fn print_sync_target(target: &SyncTarget, theme: &CliTheme) {
    println!("  {} {}", theme.dim("path"), target.path);
    if target.reasons.is_empty() {
        println!("  {} {}", theme.dim("state"), theme.blue("up to date"));
        return;
    }

    for reason in &target.reasons {
        print_sync_reason(reason, theme);
    }
}

fn print_sync_reason(reason: &SyncReason, theme: &CliTheme) {
    match reason {
        SyncReason::Missing { diff } => {
            println!("  {} {}", theme.dim("state"), theme.green("missing"));
            print_file_diff(diff, theme);
        }
        SyncReason::ContentChanged { diff } => {
            println!(
                "  {} {}",
                theme.dim("state"),
                theme.yellow("content changed")
            );
            print_file_diff(diff, theme);
        }
        SyncReason::WrongSymlink { expected, actual } => {
            println!(
                "  {} {}",
                theme.dim("state"),
                theme.yellow("symlink target changed")
            );
            println!("  {} {}", theme.dim("actual"), actual);
            println!("  {} {}", theme.dim("expect"), expected);
        }
        SyncReason::NotSymlink => {
            println!(
                "  {} {}",
                theme.dim("state"),
                theme.red("exists, but is not a symlink")
            );
        }
        SyncReason::NotDirectory => {
            println!(
                "  {} {}",
                theme.dim("state"),
                theme.red("exists, but is not a directory")
            );
        }
    }
}

fn print_file_diff(diff: &FileDiff, theme: &CliTheme) {
    if diff.is_empty() {
        return;
    }

    println!("  {}", theme.dim("diff"));
    for path in &diff.added {
        println!("    {} {}", theme.green("+"), path);
    }
    for path in &diff.changed {
        println!("    {} {}", theme.yellow("~"), path);
    }
    for path in &diff.removed {
        println!("    {} {}", theme.red("-"), path);
    }
}

fn format_action_badge(action: SyncAction, theme: &CliTheme) -> String {
    match action {
        SyncAction::Create => theme.green("[create]"),
        SyncAction::Override => theme.yellow("[update]"),
        SyncAction::Unchanged => theme.blue("[ok]"),
    }
}

impl ProviderArg {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

const fn agent_command_name(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Codex => "codex",
        AgentKind::Claude => "claude",
    }
}

struct CliTheme {
    color: bool,
}

impl CliTheme {
    fn detect() -> Self {
        Self {
            color: io::stdout().is_terminal()
                && std::env::var_os("NO_COLOR").is_none()
                && std::env::var("TERM").map_or(true, |term| term != "dumb"),
        }
    }

    fn bold(&self, value: &str) -> String {
        self.paint("1", value)
    }

    fn dim(&self, value: &str) -> String {
        self.paint("2", value)
    }

    fn blue(&self, value: &str) -> String {
        self.paint("34", value)
    }

    fn green(&self, value: &str) -> String {
        self.paint("32", value)
    }

    fn red(&self, value: &str) -> String {
        self.paint("31", value)
    }

    fn yellow(&self, value: &str) -> String {
        self.paint("33", value)
    }

    fn paint(&self, code: &str, value: &str) -> String {
        if self.color {
            return format!("\u{1b}[{code}m{value}\u{1b}[0m");
        }

        value.to_owned()
    }
}

fn current_directory() -> Result<Utf8PathBuf> {
    utf8_from_path_buf(std::env::current_dir().context("failed to resolve the current directory")?)
}

fn resolve_cli_path(value: &str) -> Result<Utf8PathBuf> {
    let path = Utf8PathBuf::from(value);
    if path.is_absolute() {
        return Ok(path);
    }

    Ok(current_directory()?.join(path))
}

fn load_registry(app_paths: &AppPaths) -> Result<Registry> {
    Registry::load(&app_paths.registry_file())
}

fn bundled_content_root() -> Result<Utf8PathBuf> {
    let manifest_dir = Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir.join("../..");
    let root = fs::canonicalize(root).context("failed to resolve workspace root")?;
    Ok(utf8_from_path_buf(root)?.join("content"))
}

fn ensure_setup_target_is_empty(path: &Utf8Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_dir() {
        anyhow::bail!("{path} is not a directory");
    }

    let mut entries =
        fs::read_dir(path).with_context(|| format!("failed to read workspace root {path}"))?;
    if entries.next().transpose()?.is_some() {
        anyhow::bail!("{path} is not empty");
    }

    Ok(())
}

fn copy_dir_contents(source_root: &Utf8Path, target_root: &Utf8Path) -> Result<()> {
    let entries =
        fs::read_dir(source_root).with_context(|| format!("failed to read {source_root}"))?;
    for entry in entries {
        let entry = entry?;
        let source_path = utf8_from_path_buf(entry.path())?;
        let target_path = target_root.join(
            source_path
                .file_name()
                .ok_or_else(|| anyhow!("failed to determine file name for {source_path}"))?,
        );

        if entry.file_type()?.is_dir() {
            fs::create_dir_all(target_path.as_std_path())
                .with_context(|| format!("failed to create {target_path}"))?;
            copy_dir_contents(&source_path, &target_path)?;
        } else {
            fs::copy(source_path.as_std_path(), target_path.as_std_path())
                .with_context(|| format!("failed to copy {source_path} to {target_path}"))?;
        }
    }

    Ok(())
}

fn canonicalize_utf8(path: &Utf8Path) -> Result<Utf8PathBuf> {
    utf8_from_path_buf(
        fs::canonicalize(path.as_std_path())
            .with_context(|| format!("failed to resolve {path}"))?,
    )
}

fn utf8_from_path_buf(path: std::path::PathBuf) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path).map_err(|path| anyhow!("non-UTF-8 path: {}", path.display()))
}

fn escape_fish_string(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

trait InitPrompter: Prompter {
    fn choose(&mut self, prompt: &str, options: &[String]) -> Result<usize>;
}

struct InquirePrompter;

impl InitPrompter for InquirePrompter {
    fn choose(&mut self, prompt: &str, options: &[String]) -> Result<usize> {
        if options.is_empty() {
            bail!("{prompt} has no options");
        }

        let selected = Select::new(prompt, options.to_vec())
            .with_render_config(inquire_render_config())
            .prompt()
            .with_context(|| format!("failed to choose {prompt}"))?;
        options
            .iter()
            .position(|option| option == &selected)
            .ok_or_else(|| anyhow!("selected option `{selected}` was not in the prompt list"))
    }
}

impl Prompter for InquirePrompter {
    fn prompt(&mut self, placeholder: &TextPlaceholder) -> Result<String> {
        Text::new(&placeholder.prompt)
            .with_render_config(inquire_render_config())
            .prompt()
            .with_context(|| format!("failed to capture {}", placeholder.prompt))
    }
}

fn inquire_render_config() -> RenderConfig<'static> {
    RenderConfig {
        prompt_prefix: Styled::new("•")
            .with_fg(Color::LightGreen)
            .with_attr(Attributes::BOLD),
        answered_prompt_prefix: Styled::new("✓")
            .with_fg(Color::LightGreen)
            .with_attr(Attributes::BOLD),
        highlighted_option_prefix: Styled::new("›")
            .with_fg(Color::LightCyan)
            .with_attr(Attributes::BOLD),
        prompt: StyleSheet::new().with_attr(Attributes::BOLD),
        selected_option: Some(
            StyleSheet::new()
                .with_fg(Color::LightCyan)
                .with_attr(Attributes::BOLD),
        ),
        ..RenderConfig::default()
    }
}

struct IoPrompter<R, W> {
    reader: R,
    writer: W,
}

impl<R, W> IoPrompter<R, W> {
    const fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
}

impl<R, W> IoPrompter<R, W>
where
    R: Read,
    W: Write,
{
    fn read_line(&mut self) -> Result<String> {
        let mut buffer = String::new();
        let mut byte = [0_u8; 1];
        loop {
            let read = self.reader.read(&mut byte)?;
            if read == 0 || byte[0] == b'\n' {
                break;
            }
            buffer.push(char::from(byte[0]));
        }

        Ok(buffer)
    }
}

impl<R, W> InitPrompter for IoPrompter<R, W>
where
    R: Read,
    W: Write,
{
    fn choose(&mut self, prompt: &str, options: &[String]) -> Result<usize> {
        if options.is_empty() {
            bail!("{prompt} has no options");
        }

        loop {
            writeln!(self.writer, "{prompt}:")?;
            for (index, option) in options.iter().enumerate() {
                writeln!(self.writer, "  {}. {}", index + 1, option)?;
            }
            write!(self.writer, "Choice [1]: ")?;
            self.writer.flush()?;

            let response = self.read_line()?;
            let trimmed = response.trim();
            if trimmed.is_empty() {
                return Ok(0);
            }
            if let Ok(choice) = trimmed.parse::<usize>()
                && (1..=options.len()).contains(&choice)
            {
                return Ok(choice - 1);
            }
            if let Some(index) = options.iter().position(|option| option == trimmed) {
                return Ok(index);
            }

            writeln!(
                self.writer,
                "Enter a number between 1 and {}.",
                options.len()
            )?;
        }
    }
}

impl<R, W> Prompter for IoPrompter<R, W>
where
    R: Read,
    W: Write,
{
    fn prompt(&mut self, placeholder: &TextPlaceholder) -> Result<String> {
        write!(self.writer, "{}: ", placeholder.prompt)?;
        self.writer.flush()?;
        Ok(self.read_line()?.trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Cli, ContextState, InitProgress, jumpto_fzf_args, render_fish_init,
        render_jumpto_candidates,
    };
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
            super::Commands::Repo {
                command: super::RepoCommands::Clone { remote, force },
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
        assert!(args.contains(&"--color=border:8,header:12,prompt:10,pointer:14,marker:11,info:8,spinner:10,hl:14,hl+:14"));
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
}
