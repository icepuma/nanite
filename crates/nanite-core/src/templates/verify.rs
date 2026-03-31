use crate::templates::model::{
    AiFragment, ContextBundle, PreparedTemplate, ReadmeFragmentRole, ReadmeVerificationFinding,
    ReadmeVerificationReport,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn verify_readme(
    template: &PreparedTemplate,
    rendered: &str,
    context: &ContextBundle,
    ai_values: &BTreeMap<usize, String>,
) -> ReadmeVerificationReport {
    if !template.is_readme() {
        return ReadmeVerificationReport::default();
    }

    let fragments = template.ai_fragments();
    let fragment_map = fragments
        .iter()
        .map(|fragment| (fragment.placeholder.index, fragment))
        .collect::<BTreeMap<_, _>>();
    let parsed = parse_readme_document(rendered);
    let mut findings = verify_readme_structure(rendered, &parsed);
    verify_badges(&mut findings, &parsed, context, &fragment_map);
    verify_fragment_values(&mut findings, &fragments, &fragment_map, context, ai_values);
    verify_static_readme_sections(&mut findings, template, ai_values, &parsed);
    ReadmeVerificationReport { findings }
}

impl ReadmeVerificationReport {
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.findings.is_empty()
    }

    #[must_use]
    pub fn repairable_fragment_indexes(&self) -> Vec<usize> {
        self.findings
            .iter()
            .filter(|finding| finding.repairable)
            .filter_map(|finding| finding.fragment_index)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[must_use]
    pub fn has_non_repairable_findings(&self) -> bool {
        self.findings.iter().any(|finding| !finding.repairable)
    }

    #[must_use]
    pub fn render_messages(&self) -> Vec<String> {
        self.findings
            .iter()
            .map(|finding| {
                finding.fragment_label.as_ref().map_or_else(
                    || finding.message.clone(),
                    |label| format!("{label}: {}", finding.message),
                )
            })
            .collect()
    }
}

fn verify_readme_structure(
    rendered: &str,
    parsed: &ParsedReadme,
) -> Vec<ReadmeVerificationFinding> {
    let mut findings = Vec::new();

    if rendered.contains("{{") || rendered.contains("}}") || rendered.contains("[[NANITE_") {
        findings.push(ReadmeVerificationFinding {
            fragment_index: None,
            fragment_label: None,
            repairable: false,
            message: "README still contains unresolved template placeholders".to_owned(),
        });
    }
    if !parsed.first_non_empty_is_h1 {
        findings.push(ReadmeVerificationFinding {
            fragment_index: None,
            fragment_label: None,
            repairable: false,
            message: "README must start with exactly one H1 title".to_owned(),
        });
    }
    if parsed.h1_count != 1 {
        findings.push(ReadmeVerificationFinding {
            fragment_index: None,
            fragment_label: None,
            repairable: false,
            message: "README must contain exactly one H1 heading".to_owned(),
        });
    }

    let expected_sections = ["Quick Start", "Usage", "Tests", "Contributing", "License"];
    let actual_sections = parsed
        .sections
        .iter()
        .map(|section| section.title.as_str())
        .collect::<Vec<_>>();
    if actual_sections != expected_sections {
        findings.push(ReadmeVerificationFinding {
            fragment_index: None,
            fragment_label: None,
            repairable: false,
            message: format!(
                "README top-level sections must be exactly: {}",
                expected_sections.join(", ")
            ),
        });
    }

    findings
}

fn verify_badges(
    findings: &mut Vec<ReadmeVerificationFinding>,
    parsed: &ParsedReadme,
    context: &ContextBundle,
    fragment_map: &BTreeMap<usize, &AiFragment>,
) {
    let badge_lines = parsed
        .preamble
        .iter()
        .filter(|line| looks_like_badge_line(line))
        .count();
    if badge_lines > 1 {
        findings.push(role_finding(
            ReadmeFragmentRole::Badges,
            fragment_map,
            true,
            "badge area must contain at most one badge line",
        ));
    }
    if badge_lines > 0
        && context.facts.ci_workflows.is_empty()
        && context.facts.license_source.is_none()
    {
        findings.push(role_finding(
            ReadmeFragmentRole::Badges,
            fragment_map,
            true,
            "badges require verified CI or license facts",
        ));
    }
}

fn verify_fragment_values(
    findings: &mut Vec<ReadmeVerificationFinding>,
    fragments: &[AiFragment],
    fragment_map: &BTreeMap<usize, &AiFragment>,
    context: &ContextBundle,
    ai_values: &BTreeMap<usize, String>,
) {
    for fragment in fragments {
        let Some(value) = ai_values.get(&fragment.placeholder.index) else {
            continue;
        };
        verify_fragment_value(findings, fragment, fragment_map, context, value);
    }
}

fn verify_fragment_value(
    findings: &mut Vec<ReadmeVerificationFinding>,
    fragment: &AiFragment,
    fragment_map: &BTreeMap<usize, &AiFragment>,
    context: &ContextBundle,
    value: &str,
) {
    match fragment.readme_role {
        Some(ReadmeFragmentRole::Badges) => verify_badge_fragment(findings, fragment_map, value),
        Some(ReadmeFragmentRole::Overview) => {
            verify_overview_fragment(findings, fragment_map, value);
        }
        Some(ReadmeFragmentRole::QuickStart) => verify_bullet_fragment(
            findings,
            fragment_map,
            ReadmeFragmentRole::QuickStart,
            value,
            2,
            3,
        ),
        Some(ReadmeFragmentRole::Usage) => verify_bullet_fragment(
            findings,
            fragment_map,
            ReadmeFragmentRole::Usage,
            value,
            2,
            3,
        ),
        Some(ReadmeFragmentRole::Tests) => {
            verify_tests_fragment(findings, fragment_map, context, value);
        }
        None => {}
    }

    if value.lines().any(|line| line.starts_with('#')) {
        let role = fragment.readme_role.unwrap_or(ReadmeFragmentRole::Overview);
        findings.push(role_finding(
            role,
            fragment_map,
            true,
            "AI fragments must not introduce headings",
        ));
    }
}

fn verify_badge_fragment(
    findings: &mut Vec<ReadmeVerificationFinding>,
    fragment_map: &BTreeMap<usize, &AiFragment>,
    value: &str,
) {
    if value.lines().filter(|line| !line.trim().is_empty()).count() > 1 {
        findings.push(role_finding(
            ReadmeFragmentRole::Badges,
            fragment_map,
            true,
            "badges must be a single markdown line or blank",
        ));
    }
}

fn verify_overview_fragment(
    findings: &mut Vec<ReadmeVerificationFinding>,
    fragment_map: &BTreeMap<usize, &AiFragment>,
    value: &str,
) {
    if value.lines().any(|line| line.trim_start().starts_with('-')) {
        findings.push(role_finding(
            ReadmeFragmentRole::Overview,
            fragment_map,
            true,
            "overview must be prose, not bullet points",
        ));
    }
    if !(2..=3).contains(&count_sentences(value)) {
        findings.push(role_finding(
            ReadmeFragmentRole::Overview,
            fragment_map,
            true,
            "overview must be 2 or 3 sentences",
        ));
    }
}

fn verify_tests_fragment(
    findings: &mut Vec<ReadmeVerificationFinding>,
    fragment_map: &BTreeMap<usize, &AiFragment>,
    context: &ContextBundle,
    value: &str,
) {
    verify_bullet_fragment(
        findings,
        fragment_map,
        ReadmeFragmentRole::Tests,
        value,
        1,
        3,
    );
    if context.facts.test_command.is_none()
        && !value
            .to_ascii_lowercase()
            .contains("no verified test command was found")
    {
        findings.push(role_finding(
            ReadmeFragmentRole::Tests,
            fragment_map,
            true,
            "tests must use the explicit fallback when no verified test command exists",
        ));
    }
}

fn verify_static_readme_sections(
    findings: &mut Vec<ReadmeVerificationFinding>,
    template: &PreparedTemplate,
    ai_values: &BTreeMap<usize, String>,
    parsed: &ParsedReadme,
) {
    let expected_sections = expected_static_readme_sections(template, ai_values);
    for title in ["Contributing", "License"] {
        let expected = expected_sections.get(title).cloned();
        let actual = parsed
            .sections
            .iter()
            .find(|section| section.title == title)
            .map(|section| normalize_section_body(&section.body));
        if expected != actual {
            findings.push(ReadmeVerificationFinding {
                fragment_index: None,
                fragment_label: Some(title.to_owned()),
                repairable: false,
                message: format!("{title} must stay identical to the template"),
            });
        }
    }
}

fn expected_static_readme_sections(
    template: &PreparedTemplate,
    ai_values: &BTreeMap<usize, String>,
) -> BTreeMap<String, String> {
    let skeleton = template
        .fragments
        .iter()
        .map(|fragment| match fragment {
            crate::templates::TemplateFragment::Literal(text) => text.clone(),
            crate::templates::TemplateFragment::Text(placeholder) => template
                .values
                .get(&placeholder.name)
                .cloned()
                .unwrap_or_default(),
            crate::templates::TemplateFragment::Expression(expression) => {
                crate::templates::render::render_tera_expression(expression, &template.values)
                    .unwrap_or_default()
            }
            crate::templates::TemplateFragment::Ai(placeholder) => ai_values
                .get(&placeholder.index)
                .cloned()
                .unwrap_or_default(),
        })
        .collect::<String>();
    let parsed = parse_readme_document(&skeleton);
    parsed
        .sections
        .into_iter()
        .filter(|section| matches!(section.title.as_str(), "Contributing" | "License"))
        .map(|section| (section.title, normalize_section_body(&section.body)))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedReadme {
    first_non_empty_is_h1: bool,
    h1_count: usize,
    preamble: Vec<String>,
    sections: Vec<ReadmeSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadmeSection {
    title: String,
    body: Vec<String>,
}

fn parse_readme_document(markdown: &str) -> ParsedReadme {
    let lines = markdown.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let first_non_empty = lines.iter().find(|line| !line.trim().is_empty());
    let first_non_empty_is_h1 =
        first_non_empty.is_some_and(|line| line.trim_start().starts_with("# "));
    let h1_count = lines
        .iter()
        .filter(|line| line.trim_start().starts_with("# "))
        .count();

    let mut seen_first_h1 = false;
    let mut preamble = Vec::new();
    let mut sections = Vec::new();
    let mut current_section: Option<ReadmeSection> = None;

    for line in lines {
        let trimmed = line.trim_start();
        if trimmed.starts_with("# ") && !seen_first_h1 {
            seen_first_h1 = true;
            continue;
        }
        if !seen_first_h1 {
            continue;
        }
        if let Some(title) = trimmed.strip_prefix("## ") {
            if let Some(section) = current_section.take() {
                sections.push(section);
            }
            current_section = Some(ReadmeSection {
                title: title.trim().to_owned(),
                body: Vec::new(),
            });
            continue;
        }
        if let Some(section) = current_section.as_mut() {
            section.body.push(line);
        } else {
            preamble.push(line);
        }
    }

    if let Some(section) = current_section {
        sections.push(section);
    }

    ParsedReadme {
        first_non_empty_is_h1,
        h1_count,
        preamble,
        sections,
    }
}

fn normalize_section_body(lines: &[String]) -> String {
    lines.join("\n").trim().to_owned()
}

fn looks_like_badge_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("[![") || trimmed.starts_with("![")
}

fn count_sentences(text: &str) -> usize {
    text.split_terminator(['.', '!', '?'])
        .filter(|segment| !segment.trim().is_empty())
        .count()
}

fn bullet_lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn verify_bullet_fragment(
    findings: &mut Vec<ReadmeVerificationFinding>,
    fragment_map: &BTreeMap<usize, &AiFragment>,
    role: ReadmeFragmentRole,
    value: &str,
    min: usize,
    max: usize,
) {
    let bullets = bullet_lines(value);
    if bullets.iter().any(|line| !line.starts_with("- ")) {
        findings.push(role_finding(
            role,
            fragment_map,
            true,
            &format!("{} must use markdown bullet lines only", role.label()),
        ));
        return;
    }
    if !(min..=max).contains(&bullets.len()) {
        findings.push(role_finding(
            role,
            fragment_map,
            true,
            &format!("{} must contain {min} to {max} bullet lines", role.label()),
        ));
    }
}

fn role_finding(
    role: ReadmeFragmentRole,
    fragment_map: &BTreeMap<usize, &AiFragment>,
    repairable: bool,
    message: &str,
) -> ReadmeVerificationFinding {
    let fragment = fragment_map
        .values()
        .find(|fragment| fragment.readme_role == Some(role));
    ReadmeVerificationFinding {
        fragment_index: fragment.map(|fragment| fragment.placeholder.index),
        fragment_label: fragment.map(|fragment| fragment.label.clone()),
        repairable,
        message: message.to_owned(),
    }
}
