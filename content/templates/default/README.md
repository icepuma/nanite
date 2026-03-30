---
filename: README.md
---
# {{ repo_name() }}

{{ai:Return the badge line for this README. Use only CI and license badges that are explicitly supported by the verified repo facts. Return one markdown line or leave it blank when the facts are insufficient.}}

{{ai:Write the overview for this README in 3 to 5 sentences. Make it feel curious, vivid, and slightly narrative: start with the kind of problem, friction, or payoff that would make a thoughtful reader want to keep reading, then explain why this project is interesting or useful, and close with the concrete kinds of things someone can do with it. Keep it grounded in the verified repo facts, but do not mention the programming language, build system, workspace layout, crates, packages, modules, internal architecture, installation steps, test commands, or implementation details. Do not sound like release notes or a technical inventory. Prefer an inviting, human explanation that helps someone picture when and why they would reach for this project.}}

## Quick Start

{{ai:Write 2 or 3 concise markdown bullet lines for the fastest verified way to install, boot, or run this project. Use only commands and files that are explicitly supported by the verified repo facts.}}

## Usage

{{ai:Write 2 or 3 concise markdown bullet lines describing what someone can do with this project once it is running. Focus on the main workflow and the most relevant verified commands. Do not describe internal crates, package layout, or implementation details.}}

## Tests

{{ai:Write 1 to 3 concise markdown bullet lines describing how to run tests or checks for this repository. If there is no verified test command, return exactly one bullet that says no verified test command was found.}}

## Contributing

- Keep changes focused, explain the user-facing impact, and run the relevant checks before handing work off.
- Update docs, tests, and examples alongside behavior changes so the repository stays trustworthy to new contributors.

## License

- Refer to the repository license files and metadata for the current license terms.
