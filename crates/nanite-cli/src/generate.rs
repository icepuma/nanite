use crate::cli::GenerateCommands;
use crate::ui::{self, BrowserItem, GITIGNORE_UPSTREAM_REPO};
use crate::util::current_directory;
use anyhow::{Context, Result, anyhow, bail};
use camino::Utf8Path;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};

pub fn command_generate(command: GenerateCommands) -> Result<()> {
    match command {
        GenerateCommands::Gitignore { force } => command_generate_gitignore(force),
    }
}

fn command_generate_gitignore(force: bool) -> Result<()> {
    let cwd = current_directory()?;
    let target_path = cwd.join(".gitignore");
    ensure_target_path(&target_path, force)?;

    let mut templates = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        choose_templates_interactively(catalog())?
    } else {
        let stdin = io::stdin();
        let stdout = io::stdout();
        choose_templates_from_reader(catalog(), stdin.lock(), stdout.lock())?
    };
    sort_templates_by_catalog_order(catalog(), &mut templates)?;

    let rendered = render_gitignore(&templates);
    fs::write(target_path.as_std_path(), rendered)
        .with_context(|| format!("failed to write {target_path}"))?;
    println!("wrote {target_path}");
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

include!(concat!(env!("OUT_DIR"), "/generated_gitignores.rs"));

const fn catalog() -> &'static [GitignoreTemplate] {
    BUILTIN_GITIGNORES
}

fn ensure_target_path(path: &Utf8Path, force: bool) -> Result<()> {
    if !path.exists() || force {
        return Ok(());
    }

    bail!("{path} already exists; rerun with --force to overwrite it");
}

fn choose_templates_interactively(
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

fn choose_templates_from_reader<R, W>(
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
            "Bundled templates from {GITIGNORE_UPSTREAM_REPO} (vendored in content/gitignores):"
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

        match parse_non_tty_selection(templates, &line) {
            Ok(selected) => return Ok(selected),
            Err(error) => {
                writeln!(writer, "{error}")?;
            }
        }
    }
}

fn parse_non_tty_selection<'a>(
    templates: &'a [GitignoreTemplate],
    response: &str,
) -> Result<Vec<&'a GitignoreTemplate>> {
    let mut selected = Vec::new();

    for token in response
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let template = resolve_selection_token(templates, token)?;
        if !selected.contains(&template) {
            selected.push(template);
        }
    }

    if selected.is_empty() {
        bail!("select at least one template");
    }

    Ok(selected)
}

fn resolve_selection_token<'a>(
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

fn normalize_token(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn render_gitignore(templates: &[&GitignoreTemplate]) -> String {
    let mut sections = Vec::with_capacity(templates.len());

    for template in templates {
        let body = template.body.trim_end_matches('\n');
        let separator = separator_label(template, templates);
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

fn separator_label<'a>(
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

fn sort_templates_by_catalog_order(
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

#[cfg(test)]
mod tests {
    use super::{
        GitignoreTemplate, catalog, parse_non_tty_selection, render_gitignore,
        resolve_selection_token, sort_templates_by_catalog_order,
    };
    use crate::gitignore_catalog::metadata_from_relative_path;
    use std::path::Path;

    #[test]
    fn catalog_contains_known_entries_and_is_sorted() {
        let templates = catalog();

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
    fn nested_path_metadata_is_derived_consistently() {
        let relative = Path::new("community/Java/Maven.gitignore");
        let entry = metadata_from_relative_path(relative).unwrap();

        assert_eq!(entry.id, "community/java/maven");
        assert_eq!(entry.label, "maven");
        assert_eq!(entry.group, "community/java");
        assert_eq!(entry.display, "maven [community/java]");
        assert_eq!(entry.source_path, "community/Java/Maven.gitignore");
    }

    #[test]
    fn non_tty_selection_accepts_indexes_ids_labels_and_source_paths() {
        let templates = catalog();

        let selected =
            parse_non_tty_selection(templates, "1, root/kotlin, rust [root], Rust.gitignore")
                .unwrap();

        assert_eq!(selected.len(), 3);
        assert!(selected.iter().any(|template| template.id == "root/kotlin"));
        assert!(selected.iter().any(|template| template.id == "root/rust"));
    }

    #[test]
    fn selected_templates_are_sorted_by_catalog_order() {
        let templates = catalog();
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

        sort_templates_by_catalog_order(templates, &mut selected).unwrap();

        assert_eq!(selected[0].id, "root/java");
        assert_eq!(selected[1].id, "root/rust");
    }

    #[test]
    fn labels_always_include_group_context() {
        let template = catalog()
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

        let error = resolve_selection_token(&templates, "node")
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
}
