use clap::{CommandFactory, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "nanite",
    about = "Manage local repositories in an AI-first workspace",
    long_about = None,
    after_help = "Examples:\n  nanite setup ~/workspace\n  nanite repo clone github.com/icepuma/nanite\n  nanite repo refresh\n  nanite jumpto nanite",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    #[command(about = "Create and configure a Nanite workspace", long_about = None)]
    Setup {
        #[arg(
            value_name = "PATH",
            help = "Empty directory to initialize as the Nanite workspace"
        )]
        path: String,
    },
    #[command(about = "Manage repositories in the workspace", long_about = None)]
    Repo {
        #[command(subcommand)]
        command: RepoCommands,
    },
    #[command(name = "jumpto")]
    #[command(about = "Choose a workspace repository and print its path", long_about = None)]
    Jumpto {
        #[arg(
            value_name = "QUERY",
            help = "Initial search text for the repository picker"
        )]
        query: Option<String>,
    },
    #[command(about = "Print shell integration for Nanite", long_about = None)]
    Shell {
        #[command(subcommand)]
        command: ShellCommands,
    },
    #[command(hide = true, name = "__complete-jumpto")]
    CompleteJumpto,
    #[command(hide = true, name = "__complete-repo-remove")]
    CompleteRepoRemove,
}

#[derive(Debug, Subcommand)]
#[command(
    about = "Manage repositories in the workspace",
    long_about = None,
    after_help = "Examples:\n  nanite repo clone github.com/icepuma/nanite\n  nanite repo remove --yes github.com/icepuma/nanite\n  nanite repo refresh"
)]
pub enum RepoCommands {
    #[command(
        about = "Clone a repository, GitHub org, or GitLab group into the workspace",
        long_about = "Detection is fully URL-based:\n  github.com/<owner>            -> all repos in the org/user (interactive confirm)\n  github.com/<owner>/<repo>     -> single repo\n  gitlab.com/<path>             -> probed via GitLab API; group -> bulk, project -> single\n  *.git suffix                  -> always single\n\nIf any destination already exists, nanite asks interactively before overwriting.\nA TTY is required whenever a prompt is needed."
    )]
    Clone {
        #[arg(
            value_name = "REMOTE",
            help = "Git remote, repository spec, GitHub org URL, or GitLab group URL"
        )]
        remote: String,
    },
    #[command(about = "Remove a repository from the workspace", long_about = None)]
    Remove {
        #[arg(
            value_name = "TARGET",
            help = "Workspace repo target, remote, or absolute path to remove"
        )]
        target: String,
        #[arg(long, short = 'y', help = "Skip the confirmation prompt")]
        yes: bool,
    },
    #[command(about = "Import an existing local repository into the workspace", long_about = None)]
    Import {
        #[arg(
            value_name = "SOURCE",
            help = "Existing repository directory to import"
        )]
        source: String,
    },
    #[command(about = "Refresh the registry from repositories under the workspace", long_about = None)]
    Refresh,
}

#[derive(Debug, Clone, Copy, Subcommand)]
#[command(
    about = "Print shell integration for Nanite",
    long_about = None,
    after_help = "Example:\n  nanite shell init fish | source"
)]
pub enum ShellCommands {
    #[command(about = "Print shell setup for wrappers and completions", long_about = None)]
    Init {
        #[arg(value_enum, value_name = "SHELL", help = "Shell to generate setup for")]
        shell: ShellArg,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ShellArg {
    Fish,
}

#[must_use]
pub fn build_cli() -> clap::Command {
    Cli::command()
}
