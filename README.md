# nanite

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

When the same setup keeps showing up in different repositories, nanite keeps repository templates and workspace routines aligned around one local workflow. It reduces repeated setup work and keeps repository bootstrapping predictable, so you can generate consistent project files, keep docs predictable, and move between repositories without repeating the same setup.

## Quick Start

- From the repository root, run `cargo build`.
- Start the project with `cargo run`.

## Usage

- Run `nanite init` to render a template into the current repository.
- Use `nanite repo clone` and `nanite repo refresh` to add repositories to the workspace and keep them in sync.
- Use `nanite jumpto` to pick a workspace repository and print its path.

## Tests

- Run `cargo test -q` from the repository root.

## Contributing

- Keep changes focused, explain the user-facing impact, and run the relevant checks before handing work off.
- Update docs, tests, and examples alongside behavior changes so the repository stays trustworthy to new contributors.

## License

- Refer to the repository license files and metadata for the current license terms.
