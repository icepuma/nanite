use anyhow::{Context, Result, anyhow};
use camino::Utf8PathBuf;
use std::ffi::OsString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    config: Utf8PathBuf,
    codex_home: Utf8PathBuf,
    data: Utf8PathBuf,
    home: Utf8PathBuf,
    state: Utf8PathBuf,
}

impl AppPaths {
    /// Discovers Nanite's filesystem locations from the current process environment.
    ///
    /// # Errors
    ///
    /// Returns an error when required environment variables are missing or contain
    /// non-UTF-8 data.
    pub fn discover() -> Result<Self> {
        Self::from_env(|key| std::env::var_os(key))
    }

    /// Resolves Nanite's filesystem locations from an environment lookup function.
    ///
    /// # Errors
    ///
    /// Returns an error when `HOME` is unavailable or when any relevant path
    /// variable contains non-UTF-8 data.
    pub fn from_env<F>(mut lookup: F) -> Result<Self>
    where
        F: FnMut(&str) -> Option<OsString>,
    {
        let home = utf8_os_string(
            lookup("HOME").ok_or_else(|| {
                anyhow!("HOME is not set and no Nanite directory overrides were provided")
            })?,
            "HOME",
        )?;

        let config = resolve_dir(
            &mut lookup,
            "NANITE_CONFIG_DIR",
            "XDG_CONFIG_HOME",
            &home,
            ".config/nanite",
        )?;
        let codex_home = lookup("CODEX_HOME")
            .map(|value| utf8_path_buf(value, "CODEX_HOME"))
            .transpose()?
            .unwrap_or_else(|| Utf8PathBuf::from(&home).join(".codex"));
        let data = resolve_dir(
            &mut lookup,
            "NANITE_DATA_DIR",
            "XDG_DATA_HOME",
            &home,
            ".local/share/nanite",
        )?;
        let state = resolve_dir(
            &mut lookup,
            "NANITE_STATE_DIR",
            "XDG_STATE_HOME",
            &home,
            ".local/state/nanite",
        )?;

        Ok(Self {
            config,
            codex_home,
            data,
            home: Utf8PathBuf::from(home),
            state,
        })
    }

    #[must_use]
    pub const fn config_dir(&self) -> &Utf8PathBuf {
        &self.config
    }

    #[must_use]
    pub const fn data_dir(&self) -> &Utf8PathBuf {
        &self.data
    }

    #[must_use]
    pub const fn codex_home_root(&self) -> &Utf8PathBuf {
        &self.codex_home
    }

    #[must_use]
    pub fn codex_skills_root(&self) -> Utf8PathBuf {
        self.codex_home.join("skills")
    }

    #[must_use]
    pub const fn home_dir(&self) -> &Utf8PathBuf {
        &self.home
    }

    #[must_use]
    pub const fn state_dir(&self) -> &Utf8PathBuf {
        &self.state
    }

    #[must_use]
    pub fn config_file(&self) -> Utf8PathBuf {
        self.config.join("config.toml")
    }

    #[must_use]
    pub fn registry_file(&self) -> Utf8PathBuf {
        self.state.join("registry.json")
    }

    #[must_use]
    pub fn search_index_root(&self) -> Utf8PathBuf {
        self.state.join("search-index")
    }

    #[must_use]
    pub fn codex_render_root(&self) -> Utf8PathBuf {
        self.data.join("rendered/codex")
    }

    #[must_use]
    pub fn claude_plugin_seed_root(&self) -> Utf8PathBuf {
        self.data.join("claude/plugins")
    }
}

fn resolve_dir<F>(
    lookup: &mut F,
    override_key: &str,
    xdg_key: &str,
    home: &str,
    fallback_suffix: &str,
) -> Result<Utf8PathBuf>
where
    F: FnMut(&str) -> Option<OsString>,
{
    if let Some(value) = lookup(override_key) {
        return utf8_path_buf(value, override_key);
    }

    if let Some(value) = lookup(xdg_key) {
        let dir = utf8_os_string(value, xdg_key)?;
        return Ok(Utf8PathBuf::from(dir).join("nanite"));
    }

    Ok(Utf8PathBuf::from(home).join(fallback_suffix))
}

fn utf8_path_buf(value: OsString, key: &str) -> Result<Utf8PathBuf> {
    Ok(Utf8PathBuf::from(utf8_os_string(value, key)?))
}

fn utf8_os_string(value: OsString, key: &str) -> Result<String> {
    value
        .into_string()
        .map_err(|_| anyhow!("{key} contains non-UTF-8 data"))
        .with_context(|| format!("failed to resolve {key}"))
}

#[cfg(test)]
mod tests {
    use super::AppPaths;
    use std::collections::HashMap;
    use std::ffi::OsString;

    #[test]
    fn prefers_explicit_nanite_overrides() {
        let env = HashMap::from([
            ("CODEX_HOME".to_owned(), "/tmp/codex".to_owned()),
            ("HOME".to_owned(), "/tmp/home".to_owned()),
            ("NANITE_CONFIG_DIR".to_owned(), "/tmp/config".to_owned()),
            ("NANITE_DATA_DIR".to_owned(), "/tmp/data".to_owned()),
            ("NANITE_STATE_DIR".to_owned(), "/tmp/state".to_owned()),
        ]);

        let paths = AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap();

        assert_eq!(paths.config_dir().as_str(), "/tmp/config");
        assert_eq!(paths.codex_home_root().as_str(), "/tmp/codex");
        assert_eq!(paths.codex_skills_root().as_str(), "/tmp/codex/skills");
        assert_eq!(paths.data_dir().as_str(), "/tmp/data");
        assert_eq!(paths.state_dir().as_str(), "/tmp/state");
    }

    #[test]
    fn falls_back_to_xdg_paths() {
        let env = HashMap::from([
            ("HOME".to_owned(), "/tmp/home".to_owned()),
            ("XDG_CONFIG_HOME".to_owned(), "/tmp/.config".to_owned()),
            ("XDG_DATA_HOME".to_owned(), "/tmp/.data".to_owned()),
            ("XDG_STATE_HOME".to_owned(), "/tmp/.state".to_owned()),
        ]);

        let paths = AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap();

        assert_eq!(paths.config_dir().as_str(), "/tmp/.config/nanite");
        assert_eq!(paths.codex_home_root().as_str(), "/tmp/home/.codex");
        assert_eq!(paths.data_dir().as_str(), "/tmp/.data/nanite");
        assert_eq!(paths.state_dir().as_str(), "/tmp/.state/nanite");
    }

    #[test]
    fn falls_back_to_home_when_xdg_is_missing() {
        let env = HashMap::from([("HOME".to_owned(), "/tmp/home".to_owned())]);

        let paths = AppPaths::from_env(|key| env.get(key).map(OsString::from)).unwrap();

        assert_eq!(paths.config_dir().as_str(), "/tmp/home/.config/nanite");
        assert_eq!(paths.codex_home_root().as_str(), "/tmp/home/.codex");
        assert_eq!(
            paths.codex_skills_root().as_str(),
            "/tmp/home/.codex/skills"
        );
        assert_eq!(paths.data_dir().as_str(), "/tmp/home/.local/share/nanite");
        assert_eq!(
            paths.claude_plugin_seed_root().as_str(),
            "/tmp/home/.local/share/nanite/claude/plugins"
        );
        assert_eq!(
            paths.search_index_root().as_str(),
            "/tmp/home/.local/state/nanite/search-index"
        );
        assert_eq!(paths.state_dir().as_str(), "/tmp/home/.local/state/nanite");
    }
}
