# nanite

[![Verify](https://github.com/icepuma/nanite/actions/workflows/verify.yml/badge.svg?branch=main)](https://github.com/icepuma/nanite/actions/workflows/verify.yml)

Nanite is a local CLI for managing a repository workspace: it organizes repos under a predictable layout and helps you navigate between them.

## Quick Start

Install with Homebrew:

```sh
brew tap icepuma/nanite https://github.com/icepuma/nanite
brew install icepuma/nanite/nanite
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

## Usage

A typical flow looks like this:

```sh
nanite setup ~/workspace
nanite repo clone github.com/icepuma/nanite
cd "$(nanite jumpto nanite)"
nanite shell init fish | source
```

Main commands:

- `nanite setup <path>` creates the workspace and records its location.
- `nanite repo clone|import|remove|refresh` manages repositories under the workspace. Cloning an org or group URL (for example `github.com/icepuma`) bulk-clones every repository it contains.
- `nanite jumpto <query>` prints a repo path for shell wrappers and fast navigation.
- `nanite shell init fish` prints shell integration and completions.

Use `nanite --help` and `nanite <command> --help` for command-specific flags and examples.

## Development

From the repository root:

```sh
cargo build
cargo run -- --help
just verify
```

## License

Nanite is available under the [MIT License](LICENSE).
