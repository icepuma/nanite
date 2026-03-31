use crate::build_cli;
use crate::cli::{ShellArg, ShellCommands};
use crate::context::ContextState;
use crate::util::escape_fish_string;
use clap_complete::{Generator, Shell, generate};

pub fn command_shell(context: &ContextState, command: ShellCommands) {
    match command {
        ShellCommands::Init {
            shell: ShellArg::Fish,
        } => {
            print!("{}", render_fish_init(context));
        }
    }
}

pub fn render_fish_init(context: &ContextState) -> String {
    let escaped_codex_home = escape_fish_string(context.app_paths.codex_home_root().as_str());
    let escaped_seed_dirs =
        escape_fish_string(context.app_paths.claude_plugin_seed_root().as_str());
    let completions = generate_completion_script(Shell::Fish);

    format!(
        "set -gx CODEX_HOME \"{escaped_codex_home}\"\n\
set -gx CLAUDE_CODE_PLUGIN_SEED_DIR \"{escaped_seed_dirs}\"\n\
function jumpto --description 'cd into a Nanite repository'\n\
    set -l destination (nanite jumpto $argv)\n\
    or return $status\n\
    if test -n \"$destination\"\n\
        cd \"$destination\"\n\
    end\n\
end\n\
{completions}\n\
complete -c jumpto -f -a '(nanite __complete-jumpto)'\n\
complete -c nanite -n '__fish_seen_subcommand_from repo; and __fish_seen_subcommand_from remove' -f -a '(nanite __complete-repo-remove)'\n"
    )
}

fn generate_completion_script<G>(shell: G) -> String
where
    G: Generator,
{
    let mut command = build_cli();
    let mut buffer = Vec::new();
    generate(shell, &mut command, "nanite", &mut buffer);
    String::from_utf8(buffer).expect("clap completion output is valid UTF-8")
}
