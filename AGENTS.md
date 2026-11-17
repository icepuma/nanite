# Agent Guide

This repository is `nanite`.

- Treat this as a Rust workspace rooted at `Cargo.toml`; make changes in the relevant crate(s) under `crates/nanite-cli`, `crates/nanite-core`, `crates/nanite-git`, and `crates/nanite-plugins`, and keep edits workspace-aware.
- Preserve the pinned toolchain and lint posture in `Cargo.toml` (`edition = 2024`, `rust-version = 1.100`, `unsafe_code = "forbid"`); do not introduce new frameworks, build systems, or migration churn.
- Use the repo’s canonical Cargo commands for validation: `cargo build`, `cargo run`, and `cargo test -q`; when changing behavior, also run `dagger check` (fmt, clippy, nextest, doctests, rustdoc, cargo-deny) before handing off. Run `dagger generate -y` first on a fresh checkout and after editing `plugins/` or `content/wit/`; host builds need the WASM plugins it writes.
- Dagger is the only task runner (`.dagger/main.dang`, Dang SDK, pinned in `dagger-module.toml`); CI runs the same `dagger check`. Add new checks there, not as scripts.
- Prefer small, idiomatic Rust changes that fit the existing module layout and keep dependencies aligned with the workspace pins in `Cargo.toml`; avoid ad hoc scripts or one-off command drift.
- Update code, tests, and any affected config together when behavior changes, and keep public-facing changes consistent across the workspace.
