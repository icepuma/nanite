set shell := ["bash", "-c"]

verify: build-plugins
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings -W clippy::pedantic -W clippy::nursery -W clippy::cargo -A clippy::multiple-crate-versions
    cargo clippy --workspace --all-features --lib --bins -- -D clippy::unwrap_used -D clippy::expect_used -A clippy::multiple-crate-versions
    cargo nextest run --workspace --all-features
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
    cargo test --workspace --all-features --doc
    cargo deny check

build-plugins:
    #!/usr/bin/env bash
    set -euo pipefail
    rustup target add wasm32-wasip2 >/dev/null 2>&1 || true
    (cd plugins && cargo build --release -p github-org -p gitlab-group)
    mkdir -p content/plugins
    cp plugins/target/wasm32-wasip2/release/github_org.wasm \
       content/plugins/github-org.wasm
    cp plugins/target/wasm32-wasip2/release/gitlab_group.wasm \
       content/plugins/gitlab-group.wasm
    printf 'plugins built and copied to content/plugins/\n'

