set shell := ["bash", "-c"]

sync-gitignore-files:
    #!/usr/bin/env bash
    set -euo pipefail
    destination_dir="$PWD/content/gitignores"
    temp_dir="$(mktemp -d)"
    trap 'rm -rf "$temp_dir"' EXIT
    clone_dir="$temp_dir/gitignore"

    git clone --depth=1 https://github.com/github/gitignore.git "$clone_dir"

    mkdir -p "$destination_dir"
    find "$destination_dir" -type f -name '*.gitignore' -delete

    count=0
    while IFS= read -r -d '' source_path; do
      relative_path="${source_path#"$clone_dir"/}"
      target_path="$destination_dir/$relative_path"
      mkdir -p "$(dirname "$target_path")"
      cp "$source_path" "$target_path"
      count=$((count + 1))
    done < <(find "$clone_dir" -type f -name '*.gitignore' -print0 | LC_ALL=C sort -z)

    find "$destination_dir" -depth -type d -empty -delete
    rm -rf "$clone_dir"

    printf 'destination: %s\n' "$destination_dir"
    printf 'files synced: %s\n' "$count"

check-gitignores:
    just sync-gitignore-files
    git diff --exit-code -- content/gitignores

verify:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings -W clippy::pedantic -W clippy::nursery -W clippy::cargo -A clippy::multiple-crate-versions
    cargo clippy --workspace --all-features --lib --bins -- -D clippy::unwrap_used -D clippy::expect_used -A clippy::multiple-crate-versions
    cargo nextest run --workspace --all-features
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
    cargo test --workspace --all-features --doc
    cargo deny check
