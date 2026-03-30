set shell := ["bash", "-c"]

verify:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings -W clippy::pedantic -W clippy::nursery -W clippy::cargo -A clippy::multiple-crate-versions
    cargo nextest run --workspace --all-features
    cargo test --workspace --all-features --doc
    cargo deny check
