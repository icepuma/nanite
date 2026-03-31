use anyhow::{Context, Result};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use tera::{Context as TeraContext, Function as TeraFunction, Tera};
use time::OffsetDateTime;

pub(super) fn render_tera_expression(
    expression: &str,
    values: &BTreeMap<String, String>,
) -> Result<String> {
    validate_tera_expression(expression)?;
    let mut tera = build_template_engine(values);
    let rendered = tera
        .render_str(&format!("{{{{ {expression} }}}}"), &tera_context(values))
        .with_context(|| format!("failed to render `{expression}`"))?;
    Ok(rendered)
}

pub(super) fn template_builtin_values(cwd: &camino::Utf8Path) -> BTreeMap<String, String> {
    BTreeMap::from([(
        "repo_name".to_owned(),
        cwd.file_name().unwrap_or("project").to_owned(),
    )])
}

pub(super) fn ai_sentinel(index: usize) -> String {
    format!("[[NANITE_FRAGMENT_{}]]", index + 1)
}

fn validate_tera_expression(expression: &str) -> Result<()> {
    let mut tera = build_template_engine(&BTreeMap::new());
    tera.add_raw_template("expression", &format!("{{{{ {expression} }}}}"))
        .with_context(|| format!("invalid template expression `{expression}`"))?;
    Ok(())
}

fn build_template_engine(values: &BTreeMap<String, String>) -> Tera {
    let mut tera = Tera::default();
    tera.autoescape_on(Vec::new());
    tera.register_function("current_year", CurrentYearFunction);
    tera.register_function(
        "repo_name",
        RepoNameFunction {
            repo_name: values
                .get("repo_name")
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| "project".to_owned()),
        },
    );
    tera
}

fn tera_context(values: &BTreeMap<String, String>) -> TeraContext {
    let mut context = TeraContext::new();
    for (name, value) in values {
        context.insert(name, value);
    }
    context
}

#[derive(Clone, Copy)]
struct CurrentYearFunction;

impl TeraFunction for CurrentYearFunction {
    fn call(
        &self,
        _args: &std::collections::HashMap<String, JsonValue>,
    ) -> tera::Result<JsonValue> {
        Ok(JsonValue::from(i64::from(OffsetDateTime::now_utc().year())))
    }
}

#[derive(Clone)]
struct RepoNameFunction {
    repo_name: String,
}

impl TeraFunction for RepoNameFunction {
    fn call(
        &self,
        _args: &std::collections::HashMap<String, JsonValue>,
    ) -> tera::Result<JsonValue> {
        Ok(JsonValue::from(self.repo_name.clone()))
    }
}
