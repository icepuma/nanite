use anyhow::Result;
use nanite_core::{AppPaths, Config, WorkspacePaths};

#[derive(Clone)]
pub struct ContextState {
    pub app_paths: AppPaths,
    pub config: Config,
    pub workspace_paths: WorkspacePaths,
    pub git_binary: String,
    pub fzf_binary: String,
    pub zed_binary: String,
}

impl ContextState {
    pub fn load(
        app_paths: &AppPaths,
        git_binary: &str,
        fzf_binary: &str,
        zed_binary: &str,
    ) -> Result<Self> {
        let config = Config::load(app_paths)?;
        let workspace_paths = config.workspace_paths();
        Ok(Self {
            app_paths: app_paths.clone(),
            config,
            workspace_paths,
            git_binary: git_binary.to_owned(),
            fzf_binary: fzf_binary.to_owned(),
            zed_binary: zed_binary.to_owned(),
        })
    }
}
