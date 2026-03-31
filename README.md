# nanite

[![Verify](https://github.com/icepuma/nanite/actions/workflows/verify.yml/badge.svg?branch=main)](https://github.com/icepuma/nanite/actions/workflows/verify.yml) [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Nanite is a local CLI for managing an AI-first repository workspace. It creates a fixed workspace layout for repositories, templates, and skills so cloning repos, rendering standard files, syncing agent setup, and moving between projects all happen through one repeatable workflow.

## Quick Start

Build the CLI from the repository root:

```sh
cargo build
cargo run -- --help
```

Create a workspace in an empty directory:

```sh
cargo run -- setup ~/workspace
```

If you want to run the compiled binary directly, use `target/debug/nanite` in place of `cargo run --`.

## Core Features

- Workspace bootstrap: `nanite setup` creates the expected `repos/`, `templates/`, and `skills/` layout.
- Template rendering: `nanite init` renders a template into the current repository and supports `--force` when replacing an existing target file.
- Repository management: `nanite repo clone`, `nanite repo import`, `nanite repo remove`, and `nanite repo refresh` keep the workspace registry aligned with the repositories on disk.
- Fast navigation: `nanite jumpto [QUERY]` selects a workspace repository and prints its path for shell wrappers or other tooling.
- Agent setup: `nanite skill sync codex|claude [--apply]` syncs bundled Nanite-managed skills into supported agent install locations.
- Fish shell integration: `nanite shell init fish` prints setup for wrappers and completions.

## Usage

A typical workflow looks like this:

```sh
cargo run -- setup ~/workspace
cargo run -- repo clone github.com/icepuma/nanite
cargo run -- repo refresh
cargo run -- jumpto nanite
```

Run `nanite init` inside a repository when you want to render a managed template into the current working tree.

For agent setup and shell integration:

```sh
cargo run -- skill sync codex --apply
cargo run -- shell init fish | source
```

## Development

Run the workspace checks from the repository root:

```sh
cargo test -q
just verify
```

Use `cargo build` and `cargo run -- --help` for local iteration while changing the CLI.

## License

Nanite is available under the [MIT License](LICENSE).
