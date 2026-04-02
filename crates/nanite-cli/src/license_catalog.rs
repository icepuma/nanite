use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LicenseRule {
    pub tag: String,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LicenseMetadata {
    pub id: String,
    pub spdx_id: String,
    pub title: String,
    pub nickname: Option<String>,
    pub description: String,
    pub how: String,
    pub permissions: Vec<LicenseRule>,
    pub conditions: Vec<LicenseRule>,
    pub limitations: Vec<LicenseRule>,
    pub featured: bool,
    pub hidden: bool,
    pub source_path: String,
    pub raw_body: String,
    pub template_body: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ChooseALicenseFrontmatter {
    title: String,
    #[serde(rename = "spdx-id")]
    spdx_id: String,
    description: String,
    how: String,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    conditions: Vec<String>,
    #[serde(default)]
    limitations: Vec<String>,
    #[serde(default)]
    featured: bool,
    #[serde(default = "default_hidden")]
    hidden: bool,
    #[serde(default)]
    nickname: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuleGroups {
    #[serde(default)]
    permissions: Vec<RuleDefinition>,
    #[serde(default)]
    conditions: Vec<RuleDefinition>,
    #[serde(default)]
    limitations: Vec<RuleDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuleDefinition {
    tag: String,
    label: String,
    description: String,
}

const PLACEHOLDER_MAP: &[(&str, &str)] = &[
    ("[year]", "{{ year }}"),
    ("[fullname]", "{{ fullname }}"),
    ("[email]", "{{ email }}"),
    ("[project]", "{{ project }}"),
    ("[description]", "{{ description }}"),
    ("[projecturl]", "{{ projecturl }}"),
    ("[login]", "{{ login }}"),
];

pub fn parse_rule_lookup(source: &str) -> Result<BTreeMap<String, LicenseRule>, String> {
    let groups: RuleGroups = serde_yaml::from_str(source).map_err(|error| error.to_string())?;
    let mut lookup = BTreeMap::new();

    for definition in groups
        .permissions
        .into_iter()
        .chain(groups.conditions)
        .chain(groups.limitations)
    {
        lookup.insert(
            definition.tag.clone(),
            LicenseRule {
                tag: definition.tag,
                label: definition.label,
                description: definition.description,
            },
        );
    }

    Ok(lookup)
}

pub fn metadata_from_source(
    relative: &Path,
    source: &str,
    rule_lookup: &BTreeMap<String, LicenseRule>,
) -> Result<LicenseMetadata, String> {
    let stripped = source
        .strip_prefix("---\n")
        .ok_or_else(|| format!("missing frontmatter start in {}", relative.display()))?;
    let (frontmatter, body) = stripped
        .split_once("\n---\n")
        .ok_or_else(|| format!("missing frontmatter end in {}", relative.display()))?;

    let metadata: ChooseALicenseFrontmatter =
        serde_yaml::from_str(frontmatter).map_err(|error| error.to_string())?;
    let source_path = relative
        .iter()
        .filter_map(|segment| segment.to_str())
        .collect::<Vec<_>>()
        .join("/");

    Ok(LicenseMetadata {
        id: metadata.spdx_id.to_ascii_lowercase(),
        spdx_id: metadata.spdx_id,
        title: metadata.title,
        nickname: metadata.nickname,
        description: metadata.description,
        how: metadata.how,
        permissions: resolve_rules(&metadata.permissions, rule_lookup)?,
        conditions: resolve_rules(&metadata.conditions, rule_lookup)?,
        limitations: resolve_rules(&metadata.limitations, rule_lookup)?,
        featured: metadata.featured,
        hidden: metadata.hidden,
        source_path,
        raw_body: body.to_owned(),
        template_body: normalize_choosealicense_placeholders(body),
    })
}

pub fn normalize_choosealicense_placeholders(body: &str) -> String {
    PLACEHOLDER_MAP
        .iter()
        .fold(body.to_owned(), |rendered, (source, target)| {
            rendered.replace(source, target)
        })
}

fn resolve_rules(
    tags: &[String],
    rule_lookup: &BTreeMap<String, LicenseRule>,
) -> Result<Vec<LicenseRule>, String> {
    tags.iter()
        .map(|tag| {
            rule_lookup
                .get(tag)
                .cloned()
                .ok_or_else(|| format!("missing rule definition for `{tag}`"))
        })
        .collect()
}

const fn default_hidden() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{metadata_from_source, normalize_choosealicense_placeholders, parse_rule_lookup};
    use std::path::Path;

    #[test]
    fn normalizes_choosealicense_placeholders() {
        let normalized = normalize_choosealicense_placeholders(
            "Copyright (c) [year] [fullname]\nEmail: [email]\n",
        );

        assert_eq!(
            normalized,
            "Copyright (c) {{ year }} {{ fullname }}\nEmail: {{ email }}\n"
        );
    }

    #[test]
    fn resolves_rule_labels_from_rules_yaml() {
        let lookup = parse_rule_lookup(
            "permissions:\n  - tag: commercial-use\n    label: Commercial use\n    description: May be used commercially.\n",
        )
        .unwrap();
        let source = "---\ntitle: MIT License\nspdx-id: MIT\ndescription: Sample\nhow: Copy it.\npermissions:\n  - commercial-use\nhidden: false\n---\nMIT\n";
        let entry = metadata_from_source(Path::new("_licenses/mit.txt"), source, &lookup).unwrap();

        assert_eq!(entry.permissions.len(), 1);
        assert_eq!(entry.permissions[0].label, "Commercial use");
        assert_eq!(entry.id, "mit");
        assert_eq!(entry.source_path, "_licenses/mit.txt");
    }
}
