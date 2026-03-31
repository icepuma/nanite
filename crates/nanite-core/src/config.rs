use crate::{AppPaths, WorkspacePaths};
use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub workspace_root: Utf8PathBuf,
    pub agent: AgentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Codex,
    Claude,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ConfigFile {
    workspace_root: String,
    agent: String,
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

        Ok(Some(Self::from_file(&file, paths)?))
    }

    #[must_use]
    pub fn default_for(paths: &AppPaths) -> Self {
        Self {
            workspace_root: paths.home_dir().join("development"),
            agent: AgentKind::Codex,
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

    fn from_file(file: &ConfigFile, paths: &AppPaths) -> Result<Self> {
        Ok(Self {
            workspace_root: expand_path(&file.workspace_root, paths),
            agent: parse_agent(&file.agent)?,
        })
    }

    fn to_file(&self) -> ConfigFile {
        ConfigFile {
            workspace_root: self.workspace_root.to_string(),
            agent: self.agent.as_str().to_owned(),
        }
    }
}

impl AgentKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

fn parse_agent(value: &str) -> Result<AgentKind> {
    match value {
        "codex" => Ok(AgentKind::Codex),
        "claude" => Ok(AgentKind::Claude),
        other => bail!("unsupported agent `{other}`; supported agents: codex, claude"),
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
    use super::{AgentKind, Config};
    use crate::app_paths::AppPaths;
    use std::collections::HashMap;
    use std::ffi::OsString;

    #[test]
    fn default_config_uses_codex_agent() {
        let env = HashMap::from([("HOME".to_owned(), "/tmp/home".to_owned())]);
        let paths = AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap();

        let config = Config::default_for(&paths);

        assert_eq!(config.workspace_root.as_str(), "/tmp/home/development");
        assert_eq!(config.agent, AgentKind::Codex);
    }

    #[test]
    fn load_errors_when_nanite_is_unconfigured() {
        let env = HashMap::from([("HOME".to_owned(), "/tmp/home".to_owned())]);
        let paths = AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap();

        let error = Config::load(&paths).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("run 'nanite setup <path>' first")
        );
    }

    #[test]
    fn loads_new_agent_config() {
        let env = HashMap::from([("HOME".to_owned(), "/tmp/home".to_owned())]);
        let paths = AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap();
        let file: super::ConfigFile = toml::from_str(
            r#"
workspace_root = "/tmp/home/development"
agent = "codex"
"#,
        )
        .unwrap();

        let config = Config::from_file(&file, &paths).unwrap();

        assert_eq!(config.workspace_root.as_str(), "/tmp/home/development");
        assert_eq!(config.agent, AgentKind::Codex);
    }

    #[test]
    fn loads_claude_agent_config() {
        let env = HashMap::from([("HOME".to_owned(), "/tmp/home".to_owned())]);
        let paths = AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap();
        let file: super::ConfigFile = toml::from_str(
            r#"
workspace_root = "/tmp/home/development"
agent = "claude"
"#,
        )
        .unwrap();

        let config = Config::from_file(&file, &paths).unwrap();

        assert_eq!(config.workspace_root.as_str(), "/tmp/home/development");
        assert_eq!(config.agent, AgentKind::Claude);
    }

    #[test]
    fn rejects_missing_agent_in_config() {
        let error = toml::from_str::<super::ConfigFile>(
            r#"
workspace_root = "/tmp/home/development"
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("missing field `agent`"));
    }

    #[test]
    fn rejects_invalid_agent_values() {
        let env = HashMap::from([("HOME".to_owned(), "/tmp/home".to_owned())]);
        let paths = AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap();
        let file: super::ConfigFile = toml::from_str(
            r#"
workspace_root = "/tmp/home/development"
agent = "gpt"
"#,
        )
        .unwrap();

        let error = Config::from_file(&file, &paths).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("supported agents: codex, claude")
        );
    }

    #[test]
    fn saves_the_minimal_agent_config() {
        let env = HashMap::from([("HOME".to_owned(), "/tmp/home".to_owned())]);
        let _paths = AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap();
        let config = Config {
            workspace_root: Utf8PathBuf::from("/tmp/home/development"),
            agent: AgentKind::Codex,
        };

        let rendered = toml::to_string_pretty(&config.to_file()).unwrap();

        assert!(rendered.contains("workspace_root = \"/tmp/home/development\""));
        assert!(rendered.contains("agent = \"codex\""));
        assert!(!rendered.contains("[providers"));
    }

    use camino::Utf8PathBuf;
}
