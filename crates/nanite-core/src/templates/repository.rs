use crate::frontmatter::parse_frontmatter;
use crate::prompt::Prompter;
use crate::templates::context::build_context_bundle;
use crate::templates::model::{
    AiFragment, AiFragmentRequest, AiPlaceholder, ContextBundle, PreparedBundle, PreparedTemplate,
    ReadmeFragmentRole, ReadmeVerificationReport, TemplateBundle, TemplateFragment,
    TemplateMetadata, TemplateRepository, TemplateVariant, TextPlaceholder,
};
use crate::templates::parse::parse_template_fragments;
use crate::templates::render::{ai_sentinel, render_tera_expression, template_builtin_values};
use crate::templates::verify::verify_readme;
use anyhow::{Context, Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

impl TemplateRepository {
    /// Loads all template bundles from the configured templates directory.
    ///
    /// # Errors
    ///
    /// Returns an error when template directories cannot be read, contain
    /// non-UTF-8 paths, or include invalid template frontmatter or placeholders.
    pub fn load(templates_root: &Utf8Path) -> Result<Self> {
        let entries = fs::read_dir(templates_root)
            .with_context(|| format!("failed to read {templates_root}"))?;
        let mut bundles = Vec::new();

        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let bundle_path = Utf8PathBuf::from_path_buf(entry.path())
                .map_err(|path| anyhow!("non-UTF-8 template path: {}", path.display()))?;
            let bundle_name = bundle_path
                .file_name()
                .ok_or_else(|| anyhow!("failed to determine bundle name for {bundle_path}"))?
                .to_owned();
            let bundle_entries = fs::read_dir(bundle_path.as_std_path())
                .with_context(|| format!("failed to read {bundle_path}"))?;
            let mut templates = Vec::new();

            for template_entry in bundle_entries {
                let template_entry = template_entry?;
                if !template_entry.file_type()?.is_file() {
                    continue;
                }

                let source_path = Utf8PathBuf::from_path_buf(template_entry.path())
                    .map_err(|path| anyhow!("non-UTF-8 template path: {}", path.display()))?;
                let raw = fs::read_to_string(source_path.as_std_path())
                    .with_context(|| format!("failed to read {source_path}"))?;
                let document = parse_frontmatter::<TemplateMetadata>(&raw)
                    .with_context(|| format!("failed to parse {source_path}"))?;
                let fragments = parse_template_fragments(&document.body, &source_path)?;

                templates.push(TemplateVariant {
                    output_name: document.metadata.filename,
                    source_path,
                    fragments,
                });
            }

            templates.sort_by(|left, right| {
                left.output_name
                    .cmp(&right.output_name)
                    .then_with(|| left.source_path.cmp(&right.source_path))
            });

            if templates.is_empty() {
                continue;
            }

            bundles.push(TemplateBundle {
                name: bundle_name,
                source_path: bundle_path,
                templates,
            });
        }

        bundles.sort_by(|left, right| left.name.cmp(&right.name));

        Ok(Self { bundles })
    }

    #[must_use]
    pub fn output_names(&self) -> Vec<String> {
        self.bundles
            .iter()
            .flat_map(|bundle| {
                bundle
                    .templates
                    .iter()
                    .map(|template| template.output_name.clone())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn selection_labels(&self) -> Vec<String> {
        self.bundles
            .iter()
            .map(TemplateBundle::selection_label)
            .collect()
    }

    /// Looks up a template bundle by its interactive selection label.
    ///
    /// # Errors
    ///
    /// Returns an error when the label does not match any loaded bundle.
    pub fn bundle_by_selection_label(&self, label: &str) -> Result<&TemplateBundle> {
        self.bundles
            .iter()
            .find(|bundle| bundle.selection_label() == label)
            .ok_or_else(|| anyhow!("no template bundle found for selection `{label}`"))
    }
}

impl TemplateBundle {
    /// Prepares all templates in the bundle by resolving text placeholders.
    ///
    /// # Errors
    ///
    /// Returns an error when the prompter fails to provide a placeholder value.
    pub fn prepare(&self, prompter: &mut impl Prompter) -> Result<PreparedBundle> {
        self.prepare_with_values(BTreeMap::new(), prompter)
    }

    /// Prepares the bundle with built-in values derived from `cwd`.
    ///
    /// # Errors
    ///
    /// Returns an error when the prompter fails to provide a placeholder value.
    pub fn prepare_for_path(
        &self,
        cwd: &Utf8Path,
        prompter: &mut impl Prompter,
    ) -> Result<PreparedBundle> {
        self.prepare_with_seed_values(template_builtin_values(cwd), prompter)
    }

    /// Prepares the bundle using caller-provided seed values before prompting.
    ///
    /// # Errors
    ///
    /// Returns an error when the prompter fails to provide a placeholder value.
    pub fn prepare_with_seed_values(
        &self,
        seed_values: BTreeMap<String, String>,
        prompter: &mut impl Prompter,
    ) -> Result<PreparedBundle> {
        self.prepare_with_values(seed_values, prompter)
    }

    fn prepare_with_values(
        &self,
        mut values: BTreeMap<String, String>,
        prompter: &mut impl Prompter,
    ) -> Result<PreparedBundle> {
        let mut prompted_names = BTreeSet::new();

        for placeholder in self.text_placeholders() {
            if values.contains_key(&placeholder.name) {
                prompted_names.insert(placeholder.name.clone());
                continue;
            }
            if prompted_names.insert(placeholder.name.clone()) {
                let answer = prompter.prompt(&placeholder)?;
                values.insert(placeholder.name.clone(), answer);
            }
        }

        let templates = self
            .templates
            .iter()
            .map(|template| PreparedTemplate {
                output_name: template.output_name.clone(),
                source_path: template.source_path.clone(),
                fragments: template.fragments.clone(),
                values: values.clone(),
            })
            .collect();

        Ok(PreparedBundle {
            name: self.name.clone(),
            source_path: self.source_path.clone(),
            templates,
            values,
        })
    }

    #[must_use]
    pub fn selection_label(&self) -> String {
        let files = self
            .templates
            .iter()
            .map(TemplateVariant::selection_file_label)
            .collect::<Vec<_>>()
            .join(", ");
        format!("{} -> {}", self.name, files)
    }

    fn text_placeholders(&self) -> Vec<TextPlaceholder> {
        self.templates
            .iter()
            .flat_map(TemplateVariant::text_placeholders)
            .collect()
    }
}

fn extract_single_prepared_template(bundle: PreparedBundle) -> Result<PreparedTemplate> {
    bundle
        .templates
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("single bundle should contain one prepared template"))
}

impl TemplateVariant {
    #[must_use]
    fn selection_file_label(&self) -> String {
        let source_name = self
            .source_path
            .file_name()
            .unwrap_or(&self.output_name)
            .to_owned();
        if source_name == self.output_name {
            source_name
        } else {
            format!("{source_name} -> {}", self.output_name)
        }
    }

    /// Prepares a single template by resolving text placeholders.
    ///
    /// # Errors
    ///
    /// Returns an error when the prompter fails to provide a placeholder value.
    pub fn prepare(&self, prompter: &mut impl Prompter) -> Result<PreparedTemplate> {
        TemplateBundle {
            name: "single".to_owned(),
            source_path: "/tmp".into(),
            templates: vec![self.clone()],
        }
        .prepare(prompter)
        .and_then(extract_single_prepared_template)
    }

    /// Prepares a single template with built-in values derived from `cwd`.
    ///
    /// # Errors
    ///
    /// Returns an error when the prompter fails to provide a placeholder value.
    pub fn prepare_for_path(
        &self,
        cwd: &Utf8Path,
        prompter: &mut impl Prompter,
    ) -> Result<PreparedTemplate> {
        TemplateBundle {
            name: "single".to_owned(),
            source_path: "/tmp".into(),
            templates: vec![self.clone()],
        }
        .prepare_for_path(cwd, prompter)
        .and_then(extract_single_prepared_template)
    }

    /// Prepares a single template using caller-provided seed values.
    ///
    /// # Errors
    ///
    /// Returns an error when the prompter fails to provide a placeholder value.
    pub fn prepare_with_seed_values(
        &self,
        seed_values: BTreeMap<String, String>,
        prompter: &mut impl Prompter,
    ) -> Result<PreparedTemplate> {
        TemplateBundle {
            name: "single".to_owned(),
            source_path: "/tmp".into(),
            templates: vec![self.clone()],
        }
        .prepare_with_seed_values(seed_values, prompter)
        .and_then(extract_single_prepared_template)
    }

    fn text_placeholders(&self) -> Vec<TextPlaceholder> {
        self.fragments
            .iter()
            .filter_map(|fragment| match fragment {
                TemplateFragment::Text(placeholder) => Some(placeholder.clone()),
                TemplateFragment::Literal(_)
                | TemplateFragment::Expression(_)
                | TemplateFragment::Ai(_) => None,
            })
            .collect()
    }
}

impl PreparedBundle {
    #[must_use]
    pub fn templates(&self) -> &[PreparedTemplate] {
        &self.templates
    }

    #[must_use]
    pub fn requires_agent(&self) -> bool {
        self.templates.iter().any(PreparedTemplate::requires_agent)
    }

    #[must_use]
    pub const fn text_values(&self) -> &BTreeMap<String, String> {
        &self.values
    }
}

impl PreparedTemplate {
    #[must_use]
    pub fn target_path(&self, cwd: &Utf8Path) -> Utf8PathBuf {
        cwd.join(&self.output_name)
    }

    #[must_use]
    pub fn requires_agent(&self) -> bool {
        self.fragments
            .iter()
            .any(|fragment| matches!(fragment, TemplateFragment::Ai(_)))
    }

    #[must_use]
    pub fn ai_placeholders(&self) -> Vec<AiPlaceholder> {
        self.ai_fragments()
            .into_iter()
            .map(|fragment| fragment.placeholder)
            .collect()
    }

    #[must_use]
    pub fn ai_fragments(&self) -> Vec<AiFragment> {
        let readme_roles = if self.is_readme() {
            Some([
                ReadmeFragmentRole::Badges,
                ReadmeFragmentRole::Overview,
                ReadmeFragmentRole::QuickStart,
                ReadmeFragmentRole::Usage,
                ReadmeFragmentRole::Tests,
            ])
        } else {
            None
        };
        let mut fragments = Vec::new();
        let mut last_heading = None;
        let mut generic_index = 1;
        let mut readme_index = 0;

        for fragment in &self.fragments {
            match fragment {
                TemplateFragment::Literal(text) => {
                    if let Some(heading) = last_markdown_heading(text) {
                        last_heading = Some(heading);
                    }
                }
                TemplateFragment::Text(_) | TemplateFragment::Expression(_) => {}
                TemplateFragment::Ai(placeholder) => {
                    let readme_role = readme_roles
                        .as_ref()
                        .and_then(|roles| roles.get(readme_index))
                        .copied();
                    let label = readme_role.map_or_else(
                        || {
                            last_heading.clone().unwrap_or_else(|| {
                                let label = format!("Generate block {generic_index}");
                                generic_index += 1;
                                label
                            })
                        },
                        |role| {
                            readme_index += 1;
                            role.label().to_owned()
                        },
                    );
                    fragments.push(AiFragment {
                        placeholder: placeholder.clone(),
                        label,
                        readme_role,
                    });
                }
            }
        }

        fragments
    }

    /// Renders the final file with all AI fragments resolved.
    ///
    /// # Errors
    ///
    /// Returns an error when a required AI fragment value is missing or when a
    /// template expression fails to render.
    pub fn render_final(&self, ai_values: &BTreeMap<usize, String>) -> Result<String> {
        let mut rendered = String::new();

        for fragment in &self.fragments {
            match fragment {
                TemplateFragment::Literal(text) => rendered.push_str(text),
                TemplateFragment::Text(placeholder) => {
                    rendered.push_str(&render_tera_expression(&placeholder.name, &self.values)?);
                }
                TemplateFragment::Expression(expression) => {
                    rendered.push_str(&render_tera_expression(expression, &self.values)?);
                }
                TemplateFragment::Ai(placeholder) => {
                    let value = ai_values.get(&placeholder.index).ok_or_else(|| {
                        anyhow!(
                            "missing AI replacement for fragment {} in {}",
                            placeholder.index,
                            self.source_path
                        )
                    })?;
                    rendered.push_str(value);
                }
            }
        }

        Ok(rendered)
    }

    /// Writes the rendered file to `cwd`.
    ///
    /// # Errors
    ///
    /// Returns an error when AI fragments are still unresolved, the target file
    /// already exists without `force`, rendering fails, or the file cannot be
    /// written.
    pub fn write(&self, cwd: &Utf8Path, force: bool) -> Result<Utf8PathBuf> {
        if self.requires_agent() {
            bail!(
                "{} requires AI fragment resolution before it can be written",
                self.output_name
            );
        }

        let target = self.target_path(cwd);
        if target.exists() && !force {
            bail!("{target} already exists; rerun with --force to overwrite");
        }

        let rendered = self.render_final(&BTreeMap::new())?;
        fs::write(&target, rendered).with_context(|| format!("failed to write {target}"))?;
        Ok(target)
    }

    #[must_use]
    pub fn build_context_bundle(&self, cwd: &Utf8Path) -> ContextBundle {
        build_context_bundle(cwd, &self.values)
    }

    /// Builds the prompt payload for a specific AI fragment.
    ///
    /// # Errors
    ///
    /// Returns an error when the active document cannot be rendered with the
    /// currently resolved template values.
    pub fn build_ai_fragment_request(
        &self,
        cwd: &Utf8Path,
        context: &ContextBundle,
        fragment: &AiFragment,
        resolved_ai: &BTreeMap<usize, String>,
        repair_notes: &[String],
    ) -> Result<AiFragmentRequest> {
        Ok(AiFragmentRequest {
            target_path: self.target_path(cwd),
            target_file: self.output_name.clone(),
            template_source_path: self.source_path.clone(),
            values: self.values.clone(),
            fragment_index: fragment.placeholder.index,
            display_label: fragment.label.clone(),
            fragment_prompt: fragment.placeholder.prompt.clone(),
            active_sentinel: ai_sentinel(fragment.placeholder.index),
            document: self.render_document(fragment.placeholder.index, resolved_ai)?,
            context: context.clone(),
            readme_role: fragment.readme_role,
            repair_notes: repair_notes.to_vec(),
        })
    }

    #[must_use]
    pub fn is_readme(&self) -> bool {
        self.output_name == "README.md"
    }

    #[must_use]
    pub fn verify_readme(
        &self,
        rendered: &str,
        context: &ContextBundle,
        ai_values: &BTreeMap<usize, String>,
    ) -> ReadmeVerificationReport {
        verify_readme(self, rendered, context, ai_values)
    }

    fn render_document(
        &self,
        active_fragment: usize,
        resolved_ai: &BTreeMap<usize, String>,
    ) -> Result<String> {
        let mut rendered = String::new();

        for fragment in &self.fragments {
            match fragment {
                TemplateFragment::Literal(text) => rendered.push_str(text),
                TemplateFragment::Text(placeholder) => {
                    rendered.push_str(&render_tera_expression(&placeholder.name, &self.values)?);
                }
                TemplateFragment::Expression(expression) => {
                    rendered.push_str(&render_tera_expression(expression, &self.values)?);
                }
                TemplateFragment::Ai(placeholder) => {
                    if placeholder.index == active_fragment {
                        rendered.push_str(&ai_sentinel(placeholder.index));
                    } else if let Some(value) = resolved_ai.get(&placeholder.index) {
                        rendered.push_str(value);
                    } else {
                        rendered.push_str("{{ai:");
                        rendered.push_str(&placeholder.prompt);
                        rendered.push_str("}}");
                    }
                }
            }
        }

        Ok(rendered)
    }
}

fn last_markdown_heading(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with('#') {
                return None;
            }
            let title = trimmed.trim_start_matches('#').trim();
            if title.is_empty() {
                return None;
            }
            Some(title.to_owned())
        })
        .next_back()
}
