# Agent Guide

This repository is `nanite`.

- Treat this as a Rust workspace rooted at `Cargo.toml`; make changes in the relevant crate(s) under `crates/nanite-cli`, `crates/nanite-core`, `crates/nanite-git`, and `crates/nanite-agents`, and keep edits workspace-aware.
- Preserve the pinned toolchain and lint posture in `Cargo.toml` (`edition = 2024`, `rust-version = 1.94`, `unsafe_code = "forbid"`); do not introduce new frameworks, build systems, or migration churn.
- Use the repo’s canonical Cargo commands for validation: `cargo build`, `cargo run`, and `cargo test -q`; when changing behavior, also run the workspace checks from `just verify` (`cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features`, and the test/doc checks) before handing off.
- Prefer small, idiomatic Rust changes that fit the existing module layout and keep dependencies aligned with the workspace pins in `Cargo.toml`; avoid ad hoc scripts or one-off command drift.
- Update code, tests, and any affected config together when behavior changes, and keep public-facing changes consistent across the workspace.
