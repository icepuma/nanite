use crate::templates::model::{ContextBundle, ContextSnippet, RepoContextFacts};
use serde_json::Value as JsonValue;
use std::fs;

const MAX_CONTEXT_SNIPPET_BYTES: usize = 4 * 1024;
const MAX_WORKFLOW_SNIPPETS: usize = 8;
const MAX_INVENTORY_ENTRIES_PER_DIR: usize = 12;

pub(super) fn build_context_bundle(
    cwd: &camino::Utf8Path,
    values: &std::collections::BTreeMap<String, String>,
) -> ContextBundle {
    let repo_name = resolve_repo_name(values, cwd);
    let facts = build_repo_context_facts(cwd, repo_name);
    ContextBundle {
        summary_lines: build_context_summary_lines(cwd, &facts, values),
        snippets: collect_context_snippets(cwd),
        facts,
    }
}

fn resolve_repo_name(
    values: &std::collections::BTreeMap<String, String>,
    cwd: &camino::Utf8Path,
) -> String {
    values
        .get("repo_name")
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .or_else(|| {
            values
                .get("project_name")
                .filter(|value| !value.trim().is_empty())
                .cloned()
        })
        .unwrap_or_else(|| cwd.file_name().unwrap_or("project").to_owned())
}

fn render_optional_summary(value: Option<&str>) -> String {
    value.unwrap_or("(none)").to_owned()
}

fn render_list_summary(values: &[String], separator: &str) -> String {
    if values.is_empty() {
        "(none)".to_owned()
    } else {
        values.join(separator)
    }
}

fn build_context_summary_lines(
    cwd: &camino::Utf8Path,
    facts: &RepoContextFacts,
    values: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    let mut summary_lines = vec![
        format!("- Repo name: {}", facts.repo_name),
        format!("- Current directory: {cwd}"),
        format!("- Repo shape: {}", facts.repo_shape),
        format!(
            "- Verified CI workflows: {}",
            render_list_summary(&facts.ci_workflows, ", ")
        ),
        format!(
            "- Verified license source: {}",
            render_optional_summary(facts.license_source.as_deref())
        ),
        format!(
            "- Verified setup command: {}",
            render_optional_summary(facts.bootstrap_command.as_deref())
        ),
        format!(
            "- Verified run command: {}",
            render_optional_summary(facts.run_command.as_deref())
        ),
        format!(
            "- Verified test command: {}",
            render_optional_summary(facts.test_command.as_deref())
        ),
        format!(
            "- Docs present: {}",
            render_list_summary(&facts.docs_present, ", ")
        ),
        format!(
            "- Workspace inventory: {}",
            render_list_summary(&facts.workspace_inventory, "; ")
        ),
    ];
    for (name, value) in values {
        summary_lines.push(format!("- {name}: {}", render_summary_value(value)));
    }
    summary_lines
}

fn collect_context_snippets(cwd: &camino::Utf8Path) -> Vec<ContextSnippet> {
    let mut snippets = [
        "Cargo.toml",
        "package.json",
        "pnpm-workspace.yaml",
        "pyproject.toml",
        "go.mod",
        "go.work",
        "justfile",
        "Makefile",
        "Dockerfile",
        "compose.yaml",
        "docker-compose.yml",
        "README.md",
        "CONTRIBUTING.md",
        "LICENSE",
        "docs/README.md",
    ]
    .into_iter()
    .filter_map(|candidate| read_context_snippet(&cwd.join(candidate), candidate))
    .collect::<Vec<_>>();

    if let Some(workflow_snippets) = collect_workflow_snippets(cwd) {
        snippets.extend(workflow_snippets);
    }

    if let Some(inventory) = collect_workspace_inventory(cwd) {
        snippets.push(ContextSnippet {
            path: "workspace-inventory.txt".to_owned(),
            content: inventory,
        });
    }

    snippets
}

fn render_summary_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "(blank)".to_owned();
    }
    trimmed.to_owned()
}

fn detect_repo_shape(cwd: &camino::Utf8Path) -> &'static str {
    if cwd.join("pnpm-workspace.yaml").exists()
        || cwd.join("go.work").exists()
        || cwd.join("turbo.json").exists()
        || cwd.join("nx.json").exists()
        || directory_has_entries(cwd.join("apps"))
        || directory_has_entries(cwd.join("packages"))
        || directory_has_entries(cwd.join("crates"))
    {
        return "monorepo_root";
    }

    let cargo = read_text(&cwd.join("Cargo.toml"));
    if cargo
        .as_deref()
        .is_some_and(|contents| contents.contains("[workspace]"))
    {
        return "monorepo_root";
    }

    let package = read_text(&cwd.join("package.json"));
    if package
        .as_deref()
        .is_some_and(|contents| contents.contains("\"workspaces\""))
    {
        return "monorepo_root";
    }

    "single_project"
}

fn build_repo_context_facts(cwd: &camino::Utf8Path, repo_name: String) -> RepoContextFacts {
    let repo_shape = detect_repo_shape(cwd).to_owned();
    let ci_workflows = collect_workflow_names(cwd);
    let package_json = read_json(&cwd.join("package.json"));
    let package_manager = detect_package_manager(cwd, package_json.as_ref());
    let bootstrap_command = detect_bootstrap_command(cwd, package_json.as_ref(), package_manager);
    let run_command = detect_run_command(cwd, package_json.as_ref(), package_manager);
    let test_command = detect_test_command(cwd, package_json.as_ref(), package_manager);

    RepoContextFacts {
        repo_name,
        repo_shape,
        ci_workflows,
        license_source: detect_license_source(cwd),
        bootstrap_command,
        run_command,
        test_command,
        docs_present: collect_docs_present(cwd),
        workspace_inventory: collect_workspace_inventory_lines(cwd),
    }
}

fn directory_has_entries(path: camino::Utf8PathBuf) -> bool {
    let Ok(read_dir) = fs::read_dir(path) else {
        return false;
    };
    read_dir.into_iter().flatten().next().is_some()
}

fn read_context_snippet(path: &camino::Utf8Path, display_path: &str) -> Option<ContextSnippet> {
    if !path.is_file() {
        return None;
    }

    read_text(path).map(|content| ContextSnippet {
        path: display_path.to_owned(),
        content: truncate_for_context(&content),
    })
}

fn collect_workflow_snippets(cwd: &camino::Utf8Path) -> Option<Vec<ContextSnippet>> {
    let workflows_root = cwd.join(".github/workflows");
    let entries = fs::read_dir(workflows_root.as_std_path()).ok()?;
    let mut files = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = camino::Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            if !path.is_file() {
                return None;
            }
            Some(path)
        })
        .collect::<Vec<_>>();
    files.sort();

    let snippets = files
        .into_iter()
        .take(MAX_WORKFLOW_SNIPPETS)
        .filter_map(|path| {
            let file_name = path.file_name()?.to_owned();
            read_context_snippet(&path, &format!(".github/workflows/{file_name}"))
        })
        .collect::<Vec<_>>();

    if snippets.is_empty() {
        None
    } else {
        Some(snippets)
    }
}

fn collect_workflow_names(cwd: &camino::Utf8Path) -> Vec<String> {
    let workflows_root = cwd.join(".github/workflows");
    let Ok(entries) = fs::read_dir(workflows_root.as_std_path()) else {
        return Vec::new();
    };
    let mut names = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = camino::Utf8PathBuf::from_path_buf(entry.path()).ok()?;
            if !path.is_file() {
                return None;
            }
            path.file_name().map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn collect_workspace_inventory(cwd: &camino::Utf8Path) -> Option<String> {
    let mut sections = Vec::new();
    for (dir_name, label) in [
        ("apps", "apps"),
        ("packages", "packages"),
        ("crates", "crates"),
    ] {
        let root = cwd.join(dir_name);
        let Ok(entries) = fs::read_dir(root.as_std_path()) else {
            continue;
        };

        let mut names = entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if !entry.file_type().ok()?.is_dir() {
                    return None;
                }
                entry.file_name().into_string().ok()
            })
            .collect::<Vec<_>>();
        names.sort();
        names.truncate(MAX_INVENTORY_ENTRIES_PER_DIR);
        if names.is_empty() {
            continue;
        }

        let mut content = format!("[{label}]\n");
        for name in names {
            content.push_str("- ");
            content.push_str(&name);
            content.push('\n');
        }
        sections.push(content.trim_end().to_owned());
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

fn collect_workspace_inventory_lines(cwd: &camino::Utf8Path) -> Vec<String> {
    collect_workspace_inventory(cwd)
        .map(|content| {
            content
                .split("\n\n")
                .map(|section| section.replace('\n', "; "))
                .collect()
        })
        .unwrap_or_default()
}

fn read_text(path: &camino::Utf8Path) -> Option<String> {
    fs::read_to_string(path.as_std_path()).ok()
}

fn read_json(path: &camino::Utf8Path) -> Option<JsonValue> {
    read_text(path).and_then(|text| serde_json::from_str(&text).ok())
}

fn collect_docs_present(cwd: &camino::Utf8Path) -> Vec<String> {
    ["README.md", "CONTRIBUTING.md", "LICENSE", "docs/README.md"]
        .into_iter()
        .filter(|candidate| cwd.join(candidate).exists())
        .map(ToOwned::to_owned)
        .collect()
}

fn detect_license_source(cwd: &camino::Utf8Path) -> Option<String> {
    for candidate in ["LICENSE", "LICENSE.md", "COPYING"] {
        if cwd.join(candidate).is_file() {
            return Some(candidate.to_owned());
        }
    }

    let cargo = read_text(&cwd.join("Cargo.toml"));
    if let Some(line) = cargo
        .as_deref()
        .and_then(|text| find_toml_string_value(text, "license"))
    {
        return Some(format!("Cargo.toml ({line})"));
    }

    let package = read_json(&cwd.join("package.json"));
    if let Some(license) = package
        .as_ref()
        .and_then(|json| json.get("license"))
        .and_then(JsonValue::as_str)
    {
        return Some(format!("package.json ({license})"));
    }

    None
}

fn detect_package_manager(
    cwd: &camino::Utf8Path,
    package_json: Option<&JsonValue>,
) -> Option<&'static str> {
    if cwd.join("pnpm-workspace.yaml").exists() || cwd.join("pnpm-lock.yaml").exists() {
        return Some("pnpm");
    }
    if cwd.join("bun.lock").exists() || cwd.join("bun.lockb").exists() {
        return Some("bun");
    }
    if cwd.join("yarn.lock").exists() {
        return Some("yarn");
    }
    if let Some(manager) = package_json
        .and_then(|json| json.get("packageManager"))
        .and_then(JsonValue::as_str)
    {
        if manager.starts_with("pnpm@") {
            return Some("pnpm");
        }
        if manager.starts_with("yarn@") {
            return Some("yarn");
        }
        if manager.starts_with("bun@") {
            return Some("bun");
        }
        if manager.starts_with("npm@") {
            return Some("npm");
        }
    }
    if package_json.is_some() {
        return Some("npm");
    }

    None
}

fn detect_bootstrap_command(
    cwd: &camino::Utf8Path,
    package_json: Option<&JsonValue>,
    package_manager: Option<&'static str>,
) -> Option<String> {
    if let Some(manager) = package_manager {
        return Some(
            match manager {
                "pnpm" => "pnpm install",
                "bun" => "bun install",
                "yarn" => "yarn install",
                _ => "npm install",
            }
            .to_owned(),
        );
    }
    if cwd.join("Cargo.toml").is_file() {
        return Some("cargo build".to_owned());
    }
    if cwd.join("pyproject.toml").is_file() {
        return Some("python -m pip install -e .".to_owned());
    }
    if cwd.join("go.mod").is_file() || cwd.join("go.work").is_file() {
        return Some("go build ./...".to_owned());
    }
    if package_json.is_some() {
        return Some("npm install".to_owned());
    }

    None
}

fn detect_run_command(
    cwd: &camino::Utf8Path,
    package_json: Option<&JsonValue>,
    package_manager: Option<&'static str>,
) -> Option<String> {
    if let Some(target) = detect_named_target(&cwd.join("justfile"), &["dev", "run", "start"]) {
        return Some(format!("just {target}"));
    }
    if let Some(target) =
        detect_named_target(&cwd.join("Makefile"), &["dev", "run", "start", "serve"])
    {
        return Some(format!("make {target}"));
    }
    if let Some(scripts) = package_json.and_then(package_scripts) {
        if scripts.contains_key("dev") {
            return Some(package_manager_run_command(package_manager, "dev"));
        }
        if scripts.contains_key("start") {
            return Some(package_manager_run_command(package_manager, "start"));
        }
    }
    if cwd.join("Cargo.toml").is_file() {
        return Some("cargo run".to_owned());
    }

    None
}

fn detect_test_command(
    cwd: &camino::Utf8Path,
    package_json: Option<&JsonValue>,
    package_manager: Option<&'static str>,
) -> Option<String> {
    if let Some(target) = detect_named_target(&cwd.join("justfile"), &["test", "check"]) {
        return Some(format!("just {target}"));
    }
    if let Some(target) = detect_named_target(&cwd.join("Makefile"), &["test", "check"]) {
        return Some(format!("make {target}"));
    }
    if let Some(scripts) = package_json.and_then(package_scripts) {
        if scripts.contains_key("test") {
            return Some(package_manager_run_command(package_manager, "test"));
        }
        if scripts.contains_key("check") {
            return Some(package_manager_run_command(package_manager, "check"));
        }
    }
    if cwd.join("Cargo.toml").is_file() {
        return Some("cargo test -q".to_owned());
    }
    if cwd.join("pyproject.toml").is_file() {
        return Some("pytest".to_owned());
    }
    if cwd.join("go.mod").is_file() || cwd.join("go.work").is_file() {
        return Some("go test ./...".to_owned());
    }

    None
}

fn package_scripts(package_json: &JsonValue) -> Option<&serde_json::Map<String, JsonValue>> {
    package_json.get("scripts")?.as_object()
}

fn package_manager_run_command(package_manager: Option<&str>, script: &str) -> String {
    match package_manager.unwrap_or("npm") {
        "pnpm" => format!("pnpm {script}"),
        "yarn" => format!("yarn {script}"),
        "bun" => format!("bun run {script}"),
        _ => format!("npm run {script}"),
    }
}

fn detect_named_target(path: &camino::Utf8Path, names: &[&str]) -> Option<String> {
    let text = read_text(path)?;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some((name, _)) = trimmed.split_once(':') {
            let candidate = name.trim();
            if names.contains(&candidate) {
                return Some(candidate.to_owned());
            }
        }
    }
    None
}

fn find_toml_string_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let prefix = format!("{key} = ");
        if !trimmed.starts_with(&prefix) {
            return None;
        }
        let value = trimmed[prefix.len()..].trim();
        Some(value.trim_matches('"'))
    })
}

fn truncate_for_context(contents: &str) -> String {
    if contents.len() <= MAX_CONTEXT_SNIPPET_BYTES {
        return contents.to_owned();
    }

    let mut end = MAX_CONTEXT_SNIPPET_BYTES;
    while !contents.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &contents[..end])
}
