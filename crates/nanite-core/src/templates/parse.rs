use crate::templates::model::{AiPlaceholder, TemplateFragment, TextPlaceholder};
use anyhow::{Result, anyhow, bail};
use camino::Utf8Path;

pub(super) fn parse_template_fragments(
    body: &str,
    source_path: &Utf8Path,
) -> Result<Vec<TemplateFragment>> {
    let mut fragments = Vec::new();
    let mut cursor = 0;
    let mut ai_index = 0;

    while let Some(start_offset) = body[cursor..].find("{{") {
        let start = cursor + start_offset;
        if start > cursor {
            fragments.push(TemplateFragment::Literal(body[cursor..start].to_owned()));
        }

        let content_start = start + 2;
        let end_offset = body[content_start..]
            .find("}}")
            .ok_or_else(|| anyhow!("unterminated template placeholder in {}", source_path))?;
        let end = content_start + end_offset;
        let raw_inner = &body[content_start..end];
        if raw_inner.contains('\n') {
            bail!(
                "multiline template placeholders are not supported in {}",
                source_path
            );
        }

        let inner = raw_inner.trim();
        if inner.is_empty() {
            bail!("empty template placeholder in {}", source_path);
        }

        if let Some(prompt) = inner.strip_prefix("ai:") {
            let prompt = prompt.trim();
            if prompt.is_empty() {
                bail!("empty AI placeholder prompt in {}", source_path);
            }
            fragments.push(TemplateFragment::Ai(AiPlaceholder {
                index: ai_index,
                prompt: prompt.to_owned(),
            }));
            ai_index += 1;
        } else {
            let name = inner.trim();
            if is_valid_identifier(name) {
                fragments.push(TemplateFragment::Text(TextPlaceholder {
                    name: name.to_owned(),
                    prompt: humanize_identifier(name),
                }));
            } else if let Some(expression) = supported_tera_expression(name) {
                fragments.push(TemplateFragment::Expression(expression));
            } else {
                bail!("invalid placeholder `{name}` in {}", source_path);
            }
        }

        cursor = end + 2;
    }

    if cursor < body.len() {
        fragments.push(TemplateFragment::Literal(body[cursor..].to_owned()));
    }

    Ok(fragments)
}

fn is_valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }

    chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn supported_tera_expression(value: &str) -> Option<String> {
    let compact = value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    match compact.as_str() {
        "current_year()" => Some("current_year()".to_owned()),
        "repo_name()" => Some("repo_name()".to_owned()),
        _ => None,
    }
}

fn humanize_identifier(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            chars.next().map_or_else(String::new, |first| {
                let mut rendered = first.to_ascii_uppercase().to_string();
                rendered.push_str(chars.as_str());
                rendered
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}
