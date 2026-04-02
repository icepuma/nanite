#[path = "src/gitignore_catalog.rs"]
mod gitignore_catalog;
#[path = "src/license_catalog.rs"]
mod license_catalog;

use std::collections::BTreeSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

type BuildResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> BuildResult<()> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let gitignore_root = manifest_dir.join("../../content/gitignores");
    let license_root = manifest_dir.join("../../content/licenses/choosealicense");
    let search_ui_root = manifest_dir.join("../../content/search-ui");
    emit_rerun_markers(&gitignore_root)?;
    emit_rerun_markers(&license_root)?;
    emit_rerun_markers(&search_ui_root)?;

    let mut gitignore_entries = Vec::new();
    collect_gitignore_entries(&gitignore_root, &gitignore_root, &mut gitignore_entries)?;
    gitignore_entries.sort_by(|left, right| {
        left.label
            .cmp(&right.label)
            .then_with(|| left.group.cmp(&right.group))
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut seen_ids = BTreeSet::new();
    for entry in &gitignore_entries {
        if !seen_ids.insert(entry.id.clone()) {
            return Err(format!("duplicate gitignore id `{}`", entry.id).into());
        }
    }

    let generated_gitignores = render_gitignore_entries(&gitignore_entries)?;
    let mut license_entries = collect_license_entries(&license_root)?;
    license_entries.sort_by(|left, right| {
        right
            .featured
            .cmp(&left.featured)
            .then_with(|| {
                left.title
                    .to_ascii_lowercase()
                    .cmp(&right.title.to_ascii_lowercase())
            })
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut seen_license_ids = BTreeSet::new();
    for entry in &license_entries {
        if !seen_license_ids.insert(entry.id.clone()) {
            return Err(format!("duplicate license id `{}`", entry.id).into());
        }
    }

    let generated_licenses = render_license_entries(&license_entries)?;
    let generated_search_ui = render_search_ui(&search_ui_root)?;
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    fs::write(
        out_dir.join("generated_gitignores.rs"),
        generated_gitignores,
    )?;
    fs::write(out_dir.join("generated_licenses.rs"), generated_licenses)?;
    fs::write(out_dir.join("generated_search_ui.rs"), generated_search_ui)?;
    Ok(())
}

#[derive(Debug)]
struct CatalogEntry {
    id: String,
    label: String,
    group: String,
    display: String,
    source_path: String,
    body: String,
}

#[derive(Debug)]
struct LicenseCatalogEntry {
    id: String,
    spdx_id: String,
    title: String,
    nickname: Option<String>,
    description: String,
    how: String,
    permissions: Vec<license_catalog::LicenseRule>,
    conditions: Vec<license_catalog::LicenseRule>,
    limitations: Vec<license_catalog::LicenseRule>,
    featured: bool,
    hidden: bool,
    source_path: String,
    raw_body: String,
    template_body: String,
}

fn emit_rerun_markers(path: &Path) -> BuildResult<()> {
    println!("cargo:rerun-if-changed={}", path.display());
    if !path.exists() || path.is_file() {
        return Ok(());
    }

    let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        emit_rerun_markers(&entry.path())?;
    }

    Ok(())
}

fn collect_gitignore_entries(
    root: &Path,
    current: &Path,
    entries: &mut Vec<CatalogEntry>,
) -> BuildResult<()> {
    if !current.exists() {
        return Ok(());
    }

    let mut dir_entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    dir_entries.sort_by_key(std::fs::DirEntry::path);

    for entry in dir_entries {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_gitignore_entries(root, &path, entries)?;
            continue;
        }

        if path.extension().and_then(|value| value.to_str()) != Some("gitignore") {
            continue;
        }

        let relative = path.strip_prefix(root)?;
        let body = fs::read_to_string(&path)?;
        entries.push(catalog_entry_from_relative_path(relative, &body)?);
    }

    Ok(())
}

fn catalog_entry_from_relative_path(relative: &Path, body: &str) -> BuildResult<CatalogEntry> {
    let metadata = gitignore_catalog::metadata_from_relative_path(relative)?;

    Ok(CatalogEntry {
        id: metadata.id,
        label: metadata.label,
        group: metadata.group,
        display: metadata.display,
        source_path: metadata.source_path,
        body: body.to_owned(),
    })
}

fn collect_license_entries(root: &Path) -> BuildResult<Vec<LicenseCatalogEntry>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let rules_path = root.join("_data/rules.yml");
    let rules_source = fs::read_to_string(&rules_path)?;
    let rule_lookup = license_catalog::parse_rule_lookup(&rules_source)?;
    let licenses_root = root.join("_licenses");
    if !licenses_root.exists() {
        return Ok(Vec::new());
    }

    let mut entries = fs::read_dir(&licenses_root)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);

    let mut catalog = Vec::new();
    for entry in entries {
        if !entry.file_type()?.is_file() {
            continue;
        }

        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("txt") {
            continue;
        }

        let source = fs::read_to_string(&path)?;
        let relative = path.strip_prefix(root)?;
        let metadata = license_catalog::metadata_from_source(relative, &source, &rule_lookup)?;
        catalog.push(LicenseCatalogEntry {
            id: metadata.id,
            spdx_id: metadata.spdx_id,
            title: metadata.title,
            nickname: metadata.nickname,
            description: metadata.description,
            how: metadata.how,
            permissions: metadata.permissions,
            conditions: metadata.conditions,
            limitations: metadata.limitations,
            featured: metadata.featured,
            hidden: metadata.hidden,
            source_path: metadata.source_path,
            raw_body: metadata.raw_body,
            template_body: metadata.template_body,
        });
    }

    Ok(catalog)
}

fn render_gitignore_entries(entries: &[CatalogEntry]) -> BuildResult<String> {
    let mut rendered = String::from("const BUILTIN_GITIGNORES: &[GitignoreTemplate] = &[\n");
    for entry in entries {
        rendered.push_str("    GitignoreTemplate::new(");
        write!(rendered, "{:?}, ", entry.id)?;
        write!(rendered, "{:?}, ", entry.label)?;
        write!(rendered, "{:?}, ", entry.group)?;
        write!(rendered, "{:?}, ", entry.display)?;
        write!(rendered, "{:?}, ", entry.source_path)?;
        writeln!(rendered, "{:?}),", entry.body)?;
    }
    rendered.push_str("];\n");
    Ok(rendered)
}

fn render_license_entries(entries: &[LicenseCatalogEntry]) -> BuildResult<String> {
    let mut rendered = String::from("const BUILTIN_LICENSES: &[LicenseTemplate] = &[\n");
    for entry in entries {
        rendered.push_str("    LicenseTemplate {\n");
        writeln!(rendered, "        id: {:?},", entry.id)?;
        writeln!(rendered, "        spdx_id: {:?},", entry.spdx_id)?;
        writeln!(rendered, "        title: {:?},", entry.title)?;
        match &entry.nickname {
            Some(nickname) => writeln!(rendered, "        nickname: Some({nickname:?}),")?,
            None => rendered.push_str("        nickname: None,\n"),
        }
        writeln!(rendered, "        description: {:?},", entry.description)?;
        writeln!(rendered, "        how: {:?},", entry.how)?;
        rendered.push_str("        permissions: ");
        render_license_rules(&mut rendered, &entry.permissions)?;
        rendered.push_str(",\n        conditions: ");
        render_license_rules(&mut rendered, &entry.conditions)?;
        rendered.push_str(",\n        limitations: ");
        render_license_rules(&mut rendered, &entry.limitations)?;
        writeln!(rendered, ",")?;
        writeln!(rendered, "        featured: {},", entry.featured)?;
        writeln!(rendered, "        hidden: {},", entry.hidden)?;
        writeln!(rendered, "        source_path: {:?},", entry.source_path)?;
        writeln!(rendered, "        raw_body: {:?},", entry.raw_body)?;
        writeln!(
            rendered,
            "        template_body: {:?},",
            entry.template_body
        )?;
        rendered.push_str("    },\n");
    }
    rendered.push_str("];\n");
    Ok(rendered)
}

fn render_license_rules(
    rendered: &mut String,
    rules: &[license_catalog::LicenseRule],
) -> BuildResult<()> {
    rendered.push_str("&[");
    for rule in rules {
        rendered.push_str("LicenseRule::new(");
        write!(rendered, "{:?}, ", rule.tag)?;
        write!(rendered, "{:?}, ", rule.label)?;
        write!(rendered, "{:?}), ", rule.description)?;
    }
    rendered.push(']');
    Ok(())
}

fn render_search_ui(root: &Path) -> BuildResult<String> {
    let html = fs::read_to_string(root.join("index.html"))?;
    let style = fs::read_to_string(root.join("style.css"))?;
    let script = fs::read_to_string(root.join("app.js"))?;

    let html = html
        .replace("<!-- __STYLE__ -->", &format!("<style>\n{style}\n</style>"))
        .replace(
            "<!-- __SCRIPT__ -->",
            &format!("<script>\n{script}\n</script>"),
        );

    Ok(format!("pub const SEARCH_UI_HTML: &str = {html:?};\n"))
}
