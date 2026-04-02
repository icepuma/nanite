use clap::{Args, CommandFactory, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "nanite",
    about = "Manage local repositories in an AI-first workspace",
    long_about = None,
    after_help = "Examples:\n  nanite setup ~/workspace\n  nanite init\n  nanite generate gitignore\n  nanite generate license\n  nanite repo clone github.com/icepuma/nanite\n  nanite repo refresh\n  nanite skill sync codex --apply\n  nanite jumpto nanite",
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
    #[command(about = "Render a template into the current repository", long_about = None)]
    Init {
        #[arg(long, help = "Overwrite an existing target file")]
        force: bool,
    },
    #[command(about = "Generate files from bundled assets", long_about = None)]
    Generate {
        #[command(subcommand)]
        command: GenerateCommands,
    },
    #[command(about = "Manage repositories in the workspace", long_about = None)]
    Repo {
        #[command(subcommand)]
        command: RepoCommands,
    },
    #[command(about = "Sync Nanite-managed skills", long_about = None)]
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
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
    #[command(about = "Search code in the workspace", long_about = None)]
    Search(SearchArgs),
    #[command(hide = true, name = "__complete-jumpto")]
    CompleteJumpto,
    #[command(hide = true, name = "__complete-repo-remove")]
    CompleteRepoRemove,
}

#[derive(Debug, Clone, Copy, Subcommand)]
#[command(
    about = "Generate files from bundled assets",
    long_about = None,
    after_help = "Examples:\n  nanite generate gitignore\n  nanite generate license"
)]
pub enum GenerateCommands {
    #[command(about = "Generate a .gitignore from bundled templates", long_about = None)]
    Gitignore {
        #[arg(long, help = "Overwrite an existing .gitignore file")]
        force: bool,
    },
    #[command(about = "Generate a LICENSE from bundled templates", long_about = None)]
    License {
        #[arg(long, help = "Overwrite an existing LICENSE file")]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
#[command(
    about = "Manage repositories in the workspace",
    long_about = None,
    after_help = "Examples:\n  nanite repo clone github.com/icepuma/nanite\n  nanite repo remove --yes github.com/icepuma/nanite\n  nanite repo refresh"
)]
pub enum RepoCommands {
    #[command(about = "Clone a repository into the workspace", long_about = None)]
    Clone {
        #[arg(value_name = "REMOTE", help = "Git remote or repository spec to clone")]
        remote: String,
        #[arg(long, help = "Overwrite an existing destination directory")]
        force: bool,
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
    about = "Sync Nanite-managed skills",
    long_about = None,
    after_help = "Examples:\n  nanite skill sync codex\n  nanite skill sync codex --apply"
)]
pub enum SkillCommands {
    #[command(about = "Sync bundled skills into an agent install location", long_about = None)]
    Sync {
        #[arg(value_name = "AGENT", help = "Agent to sync skills for")]
        provider: ProviderArg,
        #[arg(long, help = "Write changes instead of showing a dry run")]
        apply: bool,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ProviderArg {
    Codex,
    Claude,
}

impl ProviderArg {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
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

#[derive(Debug, Clone, Args)]
#[command(
    about = "Search code in the workspace",
    long_about = None,
    args_conflicts_with_subcommands = true,
    after_help = "Examples:\n  nanite search \"workspace_root\"\n  nanite search --repo github.com/icepuma/nanite \"command_repo\"\n  nanite search serve\n  nanite search index rebuild"
)]
pub struct SearchArgs {
    #[command(subcommand)]
    pub command: Option<SearchCommands>,
    #[arg(value_name = "QUERY", help = "Search query")]
    pub query: Option<String>,
    #[arg(
        long,
        value_name = "REPO",
        help = "Restrict results to a repository id"
    )]
    pub repo: Option<String>,
    #[arg(
        long,
        value_name = "PATH",
        help = "Restrict results to files matching a path query"
    )]
    pub path: Option<String>,
    #[arg(
        long,
        value_name = "FILE",
        help = "Restrict results to files matching a file name query"
    )]
    pub file: Option<String>,
    #[arg(long, value_name = "LANG", help = "Restrict results to a language id")]
    pub lang: Option<String>,
    #[arg(
        long,
        default_value_t = 50,
        value_name = "LIMIT",
        help = "Maximum results to print"
    )]
    pub limit: usize,
    #[arg(long, help = "Emit JSON instead of human-readable output")]
    pub json: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum SearchCommands {
    #[command(about = "Serve the local search UI", long_about = None)]
    Serve {
        #[arg(
            long,
            default_value = "127.0.0.1",
            value_name = "HOST",
            help = "Host interface to bind"
        )]
        host: String,
        #[arg(
            long,
            default_value_t = 0,
            value_name = "PORT",
            help = "Port to bind; 0 chooses an ephemeral port"
        )]
        port: u16,
    },
    #[command(about = "Manage the persistent search index", long_about = None)]
    Index {
        #[command(subcommand)]
        command: SearchIndexCommands,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum SearchIndexCommands {
    #[command(about = "Force a full rebuild of the workspace index", long_about = None)]
    Rebuild,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ShellArg {
    Fish,
}

#[must_use]
pub fn build_cli() -> clap::Command {
    Cli::command()
}
