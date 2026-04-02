# nanite

[![Verify](https://github.com/icepuma/nanite/actions/workflows/verify.yml/badge.svg?branch=main)](https://github.com/icepuma/nanite/actions/workflows/verify.yml) [![Homebrew tap](https://img.shields.io/badge/Homebrew-tap-FBB040?logo=homebrew&logoColor=white)](https://github.com/icepuma/taps) [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Nanite is a local CLI for managing an AI-first repository workspace. It creates a fixed workspace layout for repositories, templates, and skills so cloning repos, rendering standard files, syncing agent setup, and moving between projects all happen through one repeatable workflow.

## Install

Install from the remote Homebrew tap:

```sh
brew tap icepuma/taps https://github.com/icepuma/taps
brew install icepuma/taps/nanite
```

Install from a local checkout with Cargo:

```sh
cargo install --locked --path crates/nanite-cli
```

## Quick Start

Create a workspace in an empty directory:

```sh
nanite setup ~/workspace
```

Clone a repository into that workspace and jump to it:

```sh
nanite repo clone github.com/icepuma/nanite
nanite repo refresh
nanite jumpto nanite
```

## Core Features

- Workspace bootstrap: `nanite setup` creates the expected `repos/`, `templates/`, and `skills/` layout.
- Template rendering: `nanite init` renders a template into the current repository and supports `--force` when replacing an existing target file.
- Gitignore generation: `nanite generate gitignore` builds a `.gitignore` from a searchable catalog vendored from `github/gitignore`, shows each template's upstream source path, and supports `--force` when replacing an existing file.
- Repository management: `nanite repo clone`, `nanite repo import`, `nanite repo remove`, and `nanite repo refresh` keep the workspace registry aligned with the repositories on disk.
- Fast navigation: `nanite jumpto [QUERY]` selects a workspace repository and prints its path for shell wrappers or other tooling.
- Agent setup: `nanite skill sync codex|claude [--apply]` syncs bundled Nanite-managed skills into supported agent install locations.
- Fish shell integration: `nanite shell init fish` prints setup for wrappers and completions.

## Usage

A typical workflow looks like this:

```sh
nanite setup ~/workspace
nanite repo clone github.com/icepuma/nanite
nanite repo refresh
nanite jumpto nanite
```

Run `nanite init` inside a repository when you want to render a managed template into the current working tree.
Run `nanite generate gitignore` in any project directory when you want to search bundled templates, inspect their upstream source paths, and render a `.gitignore`.

For agent setup and shell integration:

```sh
nanite generate gitignore
nanite skill sync codex --apply
nanite shell init fish | source
```

## Development

Run the workspace checks from the repository root:

```sh
cargo test -q
just verify
```

Use `cargo build` and `cargo run -- --help` for local iteration while changing the CLI. If you want a locally installed binary from the workspace checkout, use `cargo install --locked --path crates/nanite-cli`. Refresh the vendored gitignore catalog from upstream with `just sync-gitignore-files`.

## License

Nanite is available under the [MIT License](LICENSE).
