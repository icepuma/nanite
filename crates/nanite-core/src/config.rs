use crate::{AppPaths, WorkspacePaths};
use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub workspace_root: Utf8PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ConfigFile {
    workspace_root: String,
}

impl Config {
    /// Loads the configured Nanite workspace settings.
    ///
    /// # Errors
    ///
    /// Returns an error when the config file cannot be read, cannot be parsed, or
    /// does not contain a supported agent configuration.
    pub fn load(paths: &AppPaths) -> Result<Self> {
        Self::load_optional(paths)?
            .ok_or_else(|| anyhow::anyhow!("run 'nanite setup <path>' first"))
    }

    /// Loads the config file if it exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the config file exists but cannot be read, parsed,
    /// or converted into a valid `Config`.
    pub fn load_optional(paths: &AppPaths) -> Result<Option<Self>> {
        let config_path = paths.config_file();
        if !config_path.exists() {
            return Ok(None);
        }

        let raw = fs::read_to_string(config_path.as_std_path())
            .with_context(|| format!("failed to read {config_path}"))?;
        let file: ConfigFile =
            toml::from_str(&raw).with_context(|| format!("failed to parse {config_path}"))?;

        Ok(Some(Self::from_file(&file, paths)))
    }

    #[must_use]
    pub fn default_for(paths: &AppPaths) -> Self {
        Self {
            workspace_root: paths.home_dir().join("development"),
        }
    }

    /// Persists the current configuration to disk.
    ///
    /// # Errors
    ///
    /// Returns an error when the config directory cannot be created, the config
    /// cannot be serialized, or the file cannot be written.
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        let config_path = paths.config_file();
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
        } else {
            bail!("failed to determine config directory for {config_path}");
        }

        let raw = toml::to_string_pretty(&self.to_file())?;
        fs::write(&config_path, raw).with_context(|| format!("failed to write {config_path}"))?;
        Ok(())
    }

    #[must_use]
    pub fn workspace_paths(&self) -> WorkspacePaths {
        WorkspacePaths::new(self.workspace_root.clone())
    }

    fn from_file(file: &ConfigFile, paths: &AppPaths) -> Self {
        Self {
            workspace_root: expand_path(&file.workspace_root, paths),
        }
    }

    fn to_file(&self) -> ConfigFile {
        ConfigFile {
            workspace_root: self.workspace_root.to_string(),
        }
    }
}

fn expand_path(value: &str, paths: &AppPaths) -> Utf8PathBuf {
    let home = paths.home_dir().as_str();
    let expanded = if value == "~" {
        home.to_owned()
    } else if let Some(stripped) = value.strip_prefix("~/") {
        format!("{home}/{stripped}")
    } else {
        value.to_owned()
    };

    let path = Utf8PathBuf::from(expanded);
    if path.is_absolute() {
        return path;
    }

    Utf8Path::new(paths.config_dir()).join(path)
}

#[cfg(test)]
mod tests {
    use super::Config;
    use crate::app_paths::AppPaths;
    use camino::Utf8PathBuf;
    use std::collections::HashMap;
    use std::ffi::OsString;

    fn test_paths() -> AppPaths {
        let env = HashMap::from([("HOME".to_owned(), "/tmp/home".to_owned())]);
        AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap()
    }

    #[test]
    fn default_config_points_at_the_home_development_directory() {
        let config = Config::default_for(&test_paths());

        assert_eq!(config.workspace_root.as_str(), "/tmp/home/development");
    }

    #[test]
    fn load_errors_when_nanite_is_unconfigured() {
        let error = Config::load(&test_paths()).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("run 'nanite setup <path>' first")
        );
    }

    #[test]
    fn loads_the_workspace_root_from_the_config_file() {
        let file: super::ConfigFile = toml::from_str(
            r#"
workspace_root = "/tmp/home/development"
"#,
        )
        .unwrap();

        let config = Config::from_file(&file, &test_paths());

        assert_eq!(config.workspace_root.as_str(), "/tmp/home/development");
    }

    #[test]
    fn ignores_settings_left_over_from_older_config_files() {
        let file: super::ConfigFile = toml::from_str(
            r#"
workspace_root = "/tmp/home/development"
agent = "codex"
"#,
        )
        .unwrap();

        let config = Config::from_file(&file, &test_paths());

        assert_eq!(config.workspace_root.as_str(), "/tmp/home/development");
    }

    #[test]
    fn expands_a_home_relative_workspace_root() {
        let file: super::ConfigFile = toml::from_str(
            r#"
workspace_root = "~/development"
"#,
        )
        .unwrap();

        let config = Config::from_file(&file, &test_paths());

        assert_eq!(config.workspace_root.as_str(), "/tmp/home/development");
    }

    #[test]
    fn saves_the_minimal_config() {
        let config = Config {
            workspace_root: Utf8PathBuf::from("/tmp/home/development"),
        };

        let rendered = toml::to_string_pretty(&config.to_file()).unwrap();

        assert!(rendered.contains("workspace_root = \"/tmp/home/development\""));
        assert!(!rendered.contains("agent"));
    }
}
