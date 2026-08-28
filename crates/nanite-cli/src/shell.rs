use crate::build_cli;
use crate::cli::{ShellArg, ShellCommands};
use clap_complete::{Generator, Shell, generate};

pub fn command_shell(command: ShellCommands) {
    match command {
        ShellCommands::Init {
            shell: ShellArg::Fish,
        } => {
            print!("{}", render_fish_init());
        }
    }
}

pub fn render_fish_init() -> String {
    let completions = generate_completion_script(Shell::Fish);

    format!(
        "function jumpto --description 'cd into a Nanite repository'\n\
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
    String::from_utf8_lossy(&buffer).into_owned()
}
