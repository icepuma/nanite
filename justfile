set shell := ["bash", "-c"]

verify:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings -W clippy::pedantic -W clippy::nursery -W clippy::cargo -A clippy::multiple-crate-versions
    cargo clippy --workspace --all-features --lib --bins -- -D clippy::unwrap_used -D clippy::expect_used -A clippy::multiple-crate-versions
    cargo nextest run --workspace --all-features
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
    cargo test --workspace --all-features --doc
    cargo deny check
