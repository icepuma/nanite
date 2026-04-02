use crate::cli::GenerateCommands;
use crate::ui::{self, BrowserItem, GITIGNORE_UPSTREAM_REPO, LICENSE_UPSTREAM_REPO};
use crate::util::current_directory;
use anyhow::{Context, Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use nanite_core::{Prompter, TextPlaceholder, template_variant_from_text};
use nanite_git::{configured_author_email, configured_author_name, git_origin, parse_remote};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use time::OffsetDateTime;

pub fn command_generate(command: GenerateCommands, git_binary: &str) -> Result<()> {
    match command {
        GenerateCommands::Gitignore { force } => command_generate_gitignore(force),
        GenerateCommands::License { force } => command_generate_license(force, git_binary),
    }
}

fn command_generate_gitignore(force: bool) -> Result<()> {
    let cwd = current_directory()?;
    let target_path = cwd.join(".gitignore");
    ensure_target_path(&target_path, force)?;

    let mut templates = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        choose_gitignore_templates_interactively(gitignore_catalog())?
    } else {
        let stdin = io::stdin();
        let stdout = io::stdout();
        choose_gitignore_templates_from_reader(gitignore_catalog(), stdin.lock(), stdout.lock())?
    };
    sort_gitignore_templates_by_catalog_order(gitignore_catalog(), &mut templates)?;

    let rendered = render_gitignore(&templates);
    fs::write(target_path.as_std_path(), rendered)
        .with_context(|| format!("failed to write {target_path}"))?;
    println!("wrote {target_path}");
    Ok(())
}

fn command_generate_license(force: bool, git_binary: &str) -> Result<()> {
    let cwd = current_directory()?;
    let target_path = cwd.join("LICENSE");
    ensure_target_path(&target_path, force)?;

    let license_catalog = exposed_license_catalog();
    if license_catalog.is_empty() {
        bail!(
            "no bundled license templates are available; run `just sync-vendored-files` and rebuild"
        );
    }

    let selected = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        choose_license_interactively(&license_catalog)?
    } else {
        let stdin = io::stdin();
        let stdout = io::stdout();
        choose_license_from_reader(&license_catalog, stdin.lock(), stdout.lock())?
    };

    let seed_values = license_seed_values(&cwd, git_binary)?;
    let template = template_variant_from_text(
        "LICENSE",
        Utf8PathBuf::from(format!("/bundled/{}", selected.source_path)),
        selected.template_body,
    )?;

    let target = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        let mut prompter = TuiLicensePrompter;
        template
            .prepare_with_seed_values(seed_values, &mut prompter)?
            .write(&cwd, force)?
    } else {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut prompter = IoLicensePrompter::new(stdin.lock(), stdout.lock());
        template
            .prepare_with_seed_values(seed_values, &mut prompter)?
            .write(&cwd, force)?
    };
    println!("wrote {target}");
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GitignoreTemplate {
    id: &'static str,
    label: &'static str,
    group: &'static str,
    display: &'static str,
    source_path: &'static str,
    body: &'static str,
}

impl GitignoreTemplate {
    const fn new(
        id: &'static str,
        label: &'static str,
        group: &'static str,
        display: &'static str,
        source_path: &'static str,
        body: &'static str,
    ) -> Self {
        Self {
            id,
            label,
            group,
            display,
            source_path,
            body,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LicenseRule {
    tag: &'static str,
    label: &'static str,
    description: &'static str,
}

impl LicenseRule {
    const fn new(tag: &'static str, label: &'static str, description: &'static str) -> Self {
        Self {
            tag,
            label,
            description,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LicenseTemplate {
    id: &'static str,
    spdx_id: &'static str,
    title: &'static str,
    nickname: Option<&'static str>,
    description: &'static str,
    how: &'static str,
    permissions: &'static [LicenseRule],
    conditions: &'static [LicenseRule],
    limitations: &'static [LicenseRule],
    featured: bool,
    hidden: bool,
    source_path: &'static str,
    raw_body: &'static str,
    template_body: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/generated_gitignores.rs"));
include!(concat!(env!("OUT_DIR"), "/generated_licenses.rs"));

const fn gitignore_catalog() -> &'static [GitignoreTemplate] {
    BUILTIN_GITIGNORES
}

const fn license_catalog() -> &'static [LicenseTemplate] {
    BUILTIN_LICENSES
}

fn exposed_license_catalog() -> Vec<&'static LicenseTemplate> {
    license_catalog().iter().collect()
}

fn ensure_target_path(path: &Utf8Path, force: bool) -> Result<()> {
    if !path.exists() || force {
        return Ok(());
    }

    bail!("{path} already exists; rerun with --force to overwrite it");
}

fn choose_gitignore_templates_interactively(
    templates: &[GitignoreTemplate],
) -> Result<Vec<&GitignoreTemplate>> {
    let items = templates
        .iter()
        .map(|template| BrowserItem {
            value: template,
            label: template.display.to_owned(),
            caption: Some(template.source_path.to_owned()),
            search_terms: format!(
                "{} {} {} {} {}",
                template.id, template.label, template.group, template.display, template.source_path
            ),
            detail_lines: gitignore_detail_lines(template),
        })
        .collect();

    ui::multi_select(
        "Generate .gitignore",
        &format!(
            "{} bundled templates from {}. Search by language, tool, framework, or source path.",
            templates.len(),
            GITIGNORE_UPSTREAM_REPO
        ),
        items,
    )
}

fn choose_gitignore_templates_from_reader<R, W>(
    templates: &[GitignoreTemplate],
    mut reader: R,
    mut writer: W,
) -> Result<Vec<&GitignoreTemplate>>
where
    R: BufRead,
    W: Write,
{
    loop {
        writeln!(
            writer,
            "Bundled templates from {GITIGNORE_UPSTREAM_REPO} (downloaded into content/gitignores):"
        )?;
        writeln!(
            writer,
            "Select .gitignore templates (comma-separated indexes, ids, full labels, or source paths):"
        )?;
        for (index, template) in templates.iter().enumerate() {
            writeln!(
                writer,
                "  {}. {}  {}",
                index + 1,
                template.display,
                template.source_path
            )?;
        }
        write!(writer, "Selection: ")?;
        writer.flush()?;

        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            bail!("no gitignore selection provided");
        }

        match parse_non_tty_gitignore_selection(templates, &line) {
            Ok(selected) => return Ok(selected),
            Err(error) => {
                writeln!(writer, "{error}")?;
            }
        }
    }
}

fn choose_license_interactively<'a>(
    templates: &'a [&'a LicenseTemplate],
) -> Result<&'a LicenseTemplate> {
    let items = templates
        .iter()
        .copied()
        .map(|template| BrowserItem {
            value: template,
            label: license_display(template),
            caption: None,
            search_terms: license_search_terms(template),
            detail_lines: license_detail_lines(template),
        })
        .collect();

    ui::select_one(
        "Generate LICENSE",
        &format!(
            "{} bundled templates from {}. Search by SPDX id, title, nickname, or rule.",
            templates.len(),
            LICENSE_UPSTREAM_REPO
        ),
        items,
    )
}

fn choose_license_from_reader<'a, R, W>(
    templates: &'a [&'a LicenseTemplate],
    mut reader: R,
    mut writer: W,
) -> Result<&'a LicenseTemplate>
where
    R: BufRead,
    W: Write,
{
    loop {
        writeln!(
            writer,
            "Bundled licenses from {LICENSE_UPSTREAM_REPO} (downloaded into content/licenses/choosealicense):"
        )?;
        writeln!(
            writer,
            "Select a license (index, SPDX id, title, nickname, or source path):"
        )?;
        for (index, template) in templates.iter().enumerate() {
            writeln!(
                writer,
                "  {}. {}  {}",
                index + 1,
                license_display(template),
                template.source_path
            )?;
            writeln!(writer, "     {}", template.description)?;
        }
        write!(writer, "Selection: ")?;
        writer.flush()?;

        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            bail!("no license selection provided");
        }

        match resolve_license_selection_token(templates, line.trim()) {
            Ok(selected) => return Ok(selected),
            Err(error) => writeln!(writer, "{error}")?,
        }
    }
}

fn parse_non_tty_gitignore_selection<'a>(
    templates: &'a [GitignoreTemplate],
    response: &str,
) -> Result<Vec<&'a GitignoreTemplate>> {
    let mut selected = Vec::new();

    for token in response
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let template = resolve_gitignore_selection_token(templates, token)?;
        if !selected.contains(&template) {
            selected.push(template);
        }
    }

    if selected.is_empty() {
        bail!("select at least one template");
    }

    Ok(selected)
}

fn resolve_gitignore_selection_token<'a>(
    templates: &'a [GitignoreTemplate],
    token: &str,
) -> Result<&'a GitignoreTemplate> {
    if let Ok(index) = token.parse::<usize>() {
        return templates
            .get(index.saturating_sub(1))
            .ok_or_else(|| anyhow!("`{token}` is not a valid template index"));
    }

    let normalized = normalize_token(token);
    if let Some(template) = templates
        .iter()
        .find(|template| normalize_token(template.id) == normalized)
    {
        return Ok(template);
    }
    if let Some(template) = templates
        .iter()
        .find(|template| normalize_token(template.display) == normalized)
    {
        return Ok(template);
    }
    if let Some(template) = templates
        .iter()
        .find(|template| normalize_token(template.source_path) == normalized)
    {
        return Ok(template);
    }

    let matches = templates
        .iter()
        .filter(|template| normalize_token(template.label) == normalized)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [template] => Ok(*template),
        [] => bail!("`{token}` did not match any gitignore template"),
        _ => bail!("`{token}` matched multiple templates; use the id, full label, or source path"),
    }
}

fn resolve_license_selection_token<'a>(
    templates: &'a [&LicenseTemplate],
    token: &str,
) -> Result<&'a LicenseTemplate> {
    let token = token.trim();
    if token.is_empty() {
        bail!("select one license");
    }
    if token.contains(',') {
        bail!("select exactly one license");
    }

    if let Ok(index) = token.parse::<usize>() {
        return templates
            .get(index.saturating_sub(1))
            .copied()
            .ok_or_else(|| anyhow!("`{token}` is not a valid license index"));
    }

    let normalized = normalize_token(token);
    if let Some(template) = [
        templates
            .iter()
            .copied()
            .find(|template| normalize_token(template.id) == normalized),
        templates
            .iter()
            .copied()
            .find(|template| normalize_token(template.spdx_id) == normalized),
        templates
            .iter()
            .copied()
            .find(|template| normalize_token(template.title) == normalized),
        templates
            .iter()
            .copied()
            .find(|template| normalize_token(template.source_path) == normalized),
        templates.iter().copied().find(|template| {
            template
                .nickname
                .is_some_and(|nickname| normalize_token(nickname) == normalized)
        }),
    ]
    .into_iter()
    .flatten()
    .next()
    {
        return Ok(template);
    }

    bail!("`{token}` did not match any bundled license")
}

fn normalize_token(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn render_gitignore(templates: &[&GitignoreTemplate]) -> String {
    let mut sections = Vec::with_capacity(templates.len());

    for template in templates {
        let body = template.body.trim_end_matches('\n');
        let separator = gitignore_separator_label(template, templates);
        if let Some(separator) = separator {
            sections.push(format!("# --- {separator} ---\n{body}"));
        } else {
            sections.push(body.to_owned());
        }
    }

    let mut rendered = sections.join("\n\n");
    rendered.push('\n');
    rendered
}

fn gitignore_separator_label<'a>(
    template: &'a GitignoreTemplate,
    templates: &'a [&'a GitignoreTemplate],
) -> Option<&'a str> {
    if templates.len() == 1 {
        return None;
    }

    let duplicate_labels = templates
        .iter()
        .filter(|candidate| candidate.label == template.label)
        .count()
        > 1;
    if duplicate_labels {
        Some(template.display)
    } else {
        Some(template.label)
    }
}

fn sort_gitignore_templates_by_catalog_order(
    catalog: &[GitignoreTemplate],
    templates: &mut Vec<&GitignoreTemplate>,
) -> Result<()> {
    let mut ordered = templates
        .iter()
        .copied()
        .map(|template| {
            let index = catalog
                .iter()
                .position(|candidate| candidate == template)
                .ok_or_else(|| {
                    anyhow!("selected template `{}` was not in the catalog", template.id)
                })?;
            Ok((index, template))
        })
        .collect::<Result<Vec<_>>>()?;

    ordered.sort_by_key(|(index, _template)| *index);
    *templates = ordered
        .into_iter()
        .map(|(_index, template)| template)
        .collect();
    Ok(())
}

fn license_seed_values(cwd: &Utf8Path, git_binary: &str) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::from([
        (
            "year".to_owned(),
            OffsetDateTime::now_utc().year().to_string(),
        ),
        (
            "project".to_owned(),
            cwd.file_name().unwrap_or("project").to_owned(),
        ),
    ]);

    if let Some(author) = configured_author_name(cwd)? {
        values.insert("fullname".to_owned(), author);
    }
    if let Some(email) = configured_author_email(cwd)? {
        values.insert("email".to_owned(), email);
    }
    if let Some(origin) = git_origin(git_binary, cwd)? {
        if let Ok(spec) = parse_remote(&origin) {
            if let Some(login) = spec.repo_path.split('/').next().map(str::trim)
                && !login.is_empty()
            {
                values.insert("login".to_owned(), login.to_owned());
            }
            values.insert(
                "projecturl".to_owned(),
                normalized_project_url(&origin, &spec),
            );
        } else {
            values.insert("projecturl".to_owned(), origin);
        }
    }

    Ok(values)
}

fn normalized_project_url(origin: &str, spec: &nanite_git::RemoteSpec) -> String {
    if spec.host == "local" {
        return origin.to_owned();
    }

    format!("https://{}/{}", spec.host, spec.repo_path)
}

fn gitignore_detail_lines(template: &GitignoreTemplate) -> Vec<String> {
    let mut lines = vec![
        format!("Bundled from {GITIGNORE_UPSTREAM_REPO}"),
        format!("Source path: {}", template.source_path),
        format!("ID: {}", template.id),
        format!("Group: {}", template.group),
        String::new(),
        "Preview:".to_owned(),
    ];
    lines.extend(preview_lines(template.body, 10));
    lines
}

fn license_detail_lines(template: &LicenseTemplate) -> Vec<String> {
    let mut lines = vec![
        format!("Bundled from {LICENSE_UPSTREAM_REPO}"),
        format!("Source path: {}", template.source_path),
        format!("SPDX ID: {}", template.spdx_id),
        format!("Catalog ID: {}", template.id),
    ];
    if let Some(nickname) = template.nickname {
        lines.push(format!("Nickname: {nickname}"));
    }
    lines.extend([
        String::new(),
        template.description.to_owned(),
        String::new(),
        format!("How to apply: {}", template.how),
        String::new(),
    ]);
    append_rule_group(&mut lines, "Permissions", template.permissions);
    append_rule_group(&mut lines, "Conditions", template.conditions);
    append_rule_group(&mut lines, "Limitations", template.limitations);
    lines.push("License text:".to_owned());
    lines.extend(template.raw_body.lines().map(ToOwned::to_owned));
    lines
}

fn append_rule_group(lines: &mut Vec<String>, title: &str, rules: &[LicenseRule]) {
    lines.push(format!("{title}:"));
    if rules.is_empty() {
        lines.push("  (none)".to_owned());
    } else {
        for rule in rules {
            lines.push(format!("  - {}", rule.label));
            lines.push(format!("    {}", rule.description));
        }
    }
    lines.push(String::new());
}

fn license_display(template: &LicenseTemplate) -> String {
    format!("{} [{}]", template.title, template.spdx_id)
}

fn license_search_terms(template: &LicenseTemplate) -> String {
    let mut parts = vec![
        template.id.to_owned(),
        template.spdx_id.to_owned(),
        template.title.to_owned(),
        template.description.to_owned(),
        template.how.to_owned(),
        template.source_path.to_owned(),
    ];
    if let Some(nickname) = template.nickname {
        parts.push(nickname.to_owned());
    }
    parts.extend(template.permissions.iter().flat_map(|rule| {
        [
            rule.tag.to_owned(),
            rule.label.to_owned(),
            rule.description.to_owned(),
        ]
    }));
    parts.extend(template.conditions.iter().flat_map(|rule| {
        [
            rule.tag.to_owned(),
            rule.label.to_owned(),
            rule.description.to_owned(),
        ]
    }));
    parts.extend(template.limitations.iter().flat_map(|rule| {
        [
            rule.tag.to_owned(),
            rule.label.to_owned(),
            rule.description.to_owned(),
        ]
    }));
    parts.join(" ")
}

fn preview_lines(body: &str, max_lines: usize) -> Vec<String> {
    let mut lines = body
        .lines()
        .take(max_lines)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if body.lines().count() > max_lines {
        lines.push("...".to_owned());
    }
    if lines.is_empty() {
        lines.push("(empty template)".to_owned());
    }
    lines
}

struct TuiLicensePrompter;

impl Prompter for TuiLicensePrompter {
    fn prompt(&mut self, placeholder: &TextPlaceholder) -> Result<String> {
        ui::prompt_text(&license_prompt_label(placeholder))
    }
}

struct IoLicensePrompter<R, W> {
    reader: R,
    writer: W,
}

impl<R, W> IoLicensePrompter<R, W> {
    const fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
}

impl<R, W> Prompter for IoLicensePrompter<R, W>
where
    R: BufRead,
    W: Write,
{
    fn prompt(&mut self, placeholder: &TextPlaceholder) -> Result<String> {
        let label = license_prompt_label(placeholder);
        write!(self.writer, "{label}: ")?;
        self.writer.flush()?;

        let mut line = String::new();
        let read = self.reader.read_line(&mut line)?;
        if read == 0 {
            bail!("no value provided for {label}");
        }

        Ok(line.trim().to_owned())
    }
}

fn license_prompt_label(placeholder: &TextPlaceholder) -> String {
    match placeholder.name.as_str() {
        "year" => "Copyright year".to_owned(),
        "fullname" => "Full name".to_owned(),
        "project" => "Project name".to_owned(),
        "projecturl" => "Project URL".to_owned(),
        "login" => "Owner login".to_owned(),
        "description" => "Project description".to_owned(),
        _ => placeholder.prompt.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GitignoreTemplate, LicenseRule, LicenseTemplate, exposed_license_catalog,
        gitignore_catalog, license_catalog, normalize_token, parse_non_tty_gitignore_selection,
        render_gitignore, resolve_gitignore_selection_token, resolve_license_selection_token,
        sort_gitignore_templates_by_catalog_order,
    };

    #[test]
    fn gitignore_catalog_contains_known_entries_and_is_sorted() {
        let templates = gitignore_catalog();

        assert!(templates.iter().any(|template| template.id == "root/rust"));
        assert!(templates.iter().any(|template| template.id == "root/java"));
        assert!(
            templates
                .iter()
                .any(|template| template.id == "root/kotlin")
        );

        let sorted = templates.windows(2).all(|pair| match pair {
            [left, right] => {
                left.label < right.label || (left.label == right.label && left.group <= right.group)
            }
            _ => true,
        });
        assert!(sorted, "embedded gitignore catalog should be sorted");
    }

    #[test]
    fn exposed_license_catalog_contains_expected_entries_and_ordering() {
        let templates = exposed_license_catalog();

        assert!(
            templates.iter().any(|template| template.id == "mit"),
            "run `just sync-vendored-files` before building tests"
        );
        assert!(templates.iter().any(|template| template.id == "apache-2.0"));
        assert!(
            templates
                .iter()
                .any(|template| template.id == "bsd-3-clause")
        );
        assert!(templates.iter().any(|template| template.id == "gpl-3.0"));
        assert!(templates.windows(2).all(|pair| match pair {
            [left, right] => {
                let left_key = (
                    !left.featured,
                    left.title.to_ascii_lowercase(),
                    left.id.to_owned(),
                );
                let right_key = (
                    !right.featured,
                    right.title.to_ascii_lowercase(),
                    right.id.to_owned(),
                );
                left_key <= right_key
            }
            _ => true,
        }));
    }

    #[test]
    fn hidden_licenses_are_exposed_in_the_picker_catalog() {
        let embedded = license_catalog();
        let exposed = exposed_license_catalog();

        assert!(embedded.iter().any(|template| template.hidden));
        assert!(exposed.iter().any(|template| template.hidden));
    }

    #[test]
    fn non_tty_gitignore_selection_accepts_indexes_ids_labels_and_source_paths() {
        let templates = gitignore_catalog();

        let selected = parse_non_tty_gitignore_selection(
            templates,
            "1, root/kotlin, rust [root], Rust.gitignore",
        )
        .unwrap();

        assert_eq!(selected.len(), 3);
        assert!(selected.iter().any(|template| template.id == "root/kotlin"));
        assert!(selected.iter().any(|template| template.id == "root/rust"));
    }

    #[test]
    fn license_selection_accepts_spdx_title_nickname_and_source_path() {
        let templates = [
            LicenseTemplate {
                id: "mit",
                spdx_id: "MIT",
                title: "MIT License",
                nickname: Some("MIT"),
                description: "Permissive",
                how: "Copy it.",
                permissions: &[],
                conditions: &[],
                limitations: &[],
                featured: true,
                hidden: false,
                source_path: "_licenses/mit.txt",
                raw_body: "MIT body",
                template_body: "MIT body",
            },
            LicenseTemplate {
                id: "apache-2.0",
                spdx_id: "Apache-2.0",
                title: "Apache License 2.0",
                nickname: Some("Apache 2"),
                description: "Permissive with patent grant",
                how: "Copy it.",
                permissions: &[],
                conditions: &[],
                limitations: &[],
                featured: true,
                hidden: false,
                source_path: "_licenses/apache-2.0.txt",
                raw_body: "Apache body",
                template_body: "Apache body",
            },
        ];
        let refs = templates.iter().collect::<Vec<_>>();

        assert_eq!(
            resolve_license_selection_token(&refs, "MIT").unwrap().id,
            "mit"
        );
        assert_eq!(
            resolve_license_selection_token(&refs, "Apache License 2.0")
                .unwrap()
                .id,
            "apache-2.0"
        );
        assert_eq!(
            resolve_license_selection_token(&refs, "Apache 2")
                .unwrap()
                .id,
            "apache-2.0"
        );
        assert_eq!(
            resolve_license_selection_token(&refs, "_licenses/mit.txt")
                .unwrap()
                .id,
            "mit"
        );
    }

    #[test]
    fn selected_gitignore_templates_are_sorted_by_catalog_order() {
        let templates = gitignore_catalog();
        let mut selected = vec![
            templates
                .iter()
                .find(|template| template.id == "root/rust")
                .unwrap(),
            templates
                .iter()
                .find(|template| template.id == "root/java")
                .unwrap(),
        ];

        sort_gitignore_templates_by_catalog_order(templates, &mut selected).unwrap();

        assert_eq!(selected[0].id, "root/java");
        assert_eq!(selected[1].id, "root/rust");
    }

    #[test]
    fn labels_always_include_group_context() {
        let template = gitignore_catalog()
            .iter()
            .find(|template| template.id == "root/rust")
            .unwrap();

        assert_eq!(template.display, "rust [root]");
    }

    #[test]
    fn ambiguous_label_requires_full_match() {
        let templates = [
            GitignoreTemplate::new(
                "root/node",
                "node",
                "root",
                "node [root]",
                "root/Node.gitignore",
                "node",
            ),
            GitignoreTemplate::new(
                "community/javascript/node",
                "node",
                "community/javascript",
                "node [community/javascript]",
                "community/JavaScript/Node.gitignore",
                "node2",
            ),
        ];

        let error = resolve_gitignore_selection_token(&templates, "node")
            .unwrap_err()
            .to_string();
        assert!(error.contains("matched multiple templates"));
    }

    #[test]
    fn render_normalizes_spacing_and_trailing_newline() {
        let templates = [
            GitignoreTemplate::new(
                "root/a",
                "a",
                "root",
                "a [root]",
                "root/A.gitignore",
                "one\n",
            ),
            GitignoreTemplate::new(
                "root/b",
                "b",
                "root",
                "b [root]",
                "root/B.gitignore",
                "two\n\n",
            ),
        ];

        let rendered = render_gitignore(&[&templates[0], &templates[1]]);

        assert_eq!(rendered, "# --- a ---\none\n\n# --- b ---\ntwo\n");
    }

    #[test]
    fn duplicate_labels_use_full_display_in_separator() {
        let templates = [
            GitignoreTemplate::new(
                "root/node",
                "node",
                "root",
                "node [root]",
                "root/Node.gitignore",
                "one\n",
            ),
            GitignoreTemplate::new(
                "community/javascript/node",
                "node",
                "community/javascript",
                "node [community/javascript]",
                "community/JavaScript/Node.gitignore",
                "two\n",
            ),
        ];

        let rendered = render_gitignore(&[&templates[0], &templates[1]]);

        assert!(rendered.contains("# --- node [root] ---"));
        assert!(rendered.contains("# --- node [community/javascript] ---"));
    }

    #[test]
    fn single_template_render_stays_unwrapped() {
        let templates = [GitignoreTemplate::new(
            "root/a",
            "a",
            "root",
            "a [root]",
            "root/A.gitignore",
            "one\n",
        )];

        let rendered = render_gitignore(&[&templates[0]]);

        assert_eq!(rendered, "one\n");
    }

    #[test]
    fn normalize_token_is_case_insensitive() {
        assert_eq!(normalize_token(" Apache-2.0 "), "apache-2.0");
    }

    #[test]
    fn license_rule_constructor_keeps_metadata() {
        let rule = LicenseRule::new("commercial-use", "Commercial use", "desc");

        assert_eq!(rule.tag, "commercial-use");
        assert_eq!(rule.label, "Commercial use");
        assert_eq!(rule.description, "desc");
    }
}
