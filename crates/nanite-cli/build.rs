#[path = "src/gitignore_catalog.rs"]
mod gitignore_catalog;

use std::collections::BTreeSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

type BuildResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> BuildResult<()> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let gitignore_root = manifest_dir.join("../../content/gitignores");
    emit_rerun_markers(&gitignore_root)?;

    let mut entries = Vec::new();
    collect_entries(&gitignore_root, &gitignore_root, &mut entries)?;
    entries.sort_by(|left, right| {
        left.label
            .cmp(&right.label)
            .then_with(|| left.group.cmp(&right.group))
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut seen_ids = BTreeSet::new();
    for entry in &entries {
        if !seen_ids.insert(entry.id.clone()) {
            return Err(format!("duplicate gitignore id `{}`", entry.id).into());
        }
    }

    let generated = render_entries(&entries)?;
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    fs::write(out_dir.join("generated_gitignores.rs"), generated)?;
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

fn collect_entries(
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
            collect_entries(root, &path, entries)?;
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

fn render_entries(entries: &[CatalogEntry]) -> BuildResult<String> {
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
