#![allow(dead_code)]
#![allow(clippy::redundant_pub_crate)]

use std::path::Path;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct GitignoreMetadata {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) group: String,
    pub(crate) display: String,
    pub(crate) source_path: String,
}

pub(crate) fn metadata_from_relative_path(relative: &Path) -> Result<GitignoreMetadata, String> {
    let stem = relative
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("invalid gitignore file name: {}", relative.display()))?;
    let group = relative
        .parent()
        .map(normalize_group)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "root".to_owned());
    let label = humanize_name(stem);
    let id = format!("{group}/{}", slugify_name(stem));
    let display = format!("{label} [{group}]");
    let source_path = relative
        .iter()
        .filter_map(|segment| segment.to_str())
        .collect::<Vec<_>>()
        .join("/");

    Ok(GitignoreMetadata {
        id,
        label,
        group,
        display,
        source_path,
    })
}

fn normalize_group(path: &Path) -> String {
    let segments = path
        .iter()
        .filter_map(|segment| segment.to_str())
        .map(slugify_name)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        "root".to_owned()
    } else {
        segments.join("/")
    }
}

fn humanize_name(input: &str) -> String {
    let mut output = String::new();
    let mut previous_is_lower_or_digit = false;
    let mut previous_was_separator = false;

    for character in input.chars() {
        match character {
            '_' | '-' => {
                if !output.is_empty() && !previous_was_separator {
                    output.push(' ');
                }
                previous_is_lower_or_digit = false;
                previous_was_separator = true;
            }
            _ if character.is_uppercase() => {
                if !output.is_empty() && previous_is_lower_or_digit && !previous_was_separator {
                    output.push(' ');
                }
                for lower in character.to_lowercase() {
                    output.push(lower);
                }
                previous_is_lower_or_digit = false;
                previous_was_separator = false;
            }
            _ => {
                output.push(character.to_ascii_lowercase());
                previous_is_lower_or_digit =
                    character.is_ascii_lowercase() || character.is_ascii_digit();
                previous_was_separator = false;
            }
        }
    }

    output.trim().to_owned()
}

fn slugify_name(input: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_dash = false;

    for character in humanize_name(input).chars() {
        match character {
            'a'..='z' | '0'..='9' => {
                slug.push(character);
                previous_was_dash = false;
            }
            '+' => {
                if !slug.is_empty() && !previous_was_dash {
                    slug.push('-');
                }
                slug.push_str("plus");
                previous_was_dash = false;
            }
            '#' => {
                if !slug.is_empty() && !previous_was_dash {
                    slug.push('-');
                }
                slug.push_str("sharp");
                previous_was_dash = false;
            }
            _ => {
                if !slug.is_empty() && !previous_was_dash {
                    slug.push('-');
                    previous_was_dash = true;
                }
            }
        }
    }

    slug.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::metadata_from_relative_path;
    use std::path::Path;

    #[test]
    fn derives_metadata_for_nested_paths() {
        let relative = Path::new("community/Java/Maven.gitignore");
        let metadata = metadata_from_relative_path(relative).unwrap();

        assert_eq!(metadata.id, "community/java/maven");
        assert_eq!(metadata.label, "maven");
        assert_eq!(metadata.group, "community/java");
        assert_eq!(metadata.display, "maven [community/java]");
        assert_eq!(metadata.source_path, "community/Java/Maven.gitignore");
    }

    #[test]
    fn slugifies_symbols_for_ids() {
        let relative = Path::new("root/C++.gitignore");
        let metadata = metadata_from_relative_path(relative).unwrap();

        assert_eq!(metadata.id, "root/c-plus-plus");
        assert_eq!(metadata.label, "c++");
    }
}
