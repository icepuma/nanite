# nanite

[![Verify](https://github.com/icepuma/nanite/actions/workflows/verify.yml/badge.svg?branch=main)](https://github.com/icepuma/nanite/actions/workflows/verify.yml)

Nanite is a local CLI for running an AI-first repository workspace: it creates a fixed workspace layout, keeps repositories organized inside it, generates common project files, syncs agent skills, and searches code across the workspace.

## Quick Start

Install with Homebrew:

```sh
brew tap icepuma/taps https://github.com/icepuma/taps
brew install icepuma/taps/nanite
```

Or install from a checkout:

```sh
cargo install --locked --path crates/nanite-cli
```

Create a workspace, clone a repo into it, and jump there:

```sh
nanite setup ~/workspace
nanite repo clone github.com/icepuma/nanite
nanite repo refresh
cd "$(nanite jumpto nanite)"
```

Search the workspace from the terminal or the local web UI:

```sh
nanite search workspace_root
nanite search --web
```

## Usage

A typical flow looks like this:

```sh
nanite setup ~/workspace
nanite repo clone github.com/icepuma/nanite
cd "$(nanite jumpto nanite)"
nanite generate gitignore
nanite generate license
nanite search 'repo:nanite workspace_root'
nanite search --web
nanite skill sync codex --apply
```

Main commands:

- `nanite repo clone|import|remove|refresh` manages repositories under the workspace.
- `nanite jumpto <query>` prints a repo path for shell wrappers and fast navigation.
- `nanite search <query>` searches indexed workspace code; `nanite search --web` serves the local search UI.
- `nanite init` renders a managed template into the current repository.
- `nanite generate gitignore|license` renders bundled file templates.
- `nanite skill sync codex|claude --apply` installs Nanite-managed skills for supported agents.
- `nanite shell init fish` prints shell integration and completions.

Use `nanite --help` and `nanite <command> --help` for command-specific flags and examples.

## Development

From the repository root:

```sh
cargo build
cargo run -- --help
just verify
```

Refresh the vendored catalogs with:

```sh
just sync-vendored-files
```

## License

Nanite is available under the [MIT License](LICENSE).
