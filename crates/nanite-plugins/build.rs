use std::path::Path;

fn main() {
    let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") else {
        panic!("CARGO_MANIFEST_DIR must be set by cargo");
    };
    let Some(workspace_root) = Path::new(&manifest_dir).parent().and_then(Path::parent) else {
        panic!("workspace root above crates/nanite-plugins must exist");
    };

    for plugin in ["github-org.wasm", "gitlab-group.wasm"] {
        let path = workspace_root.join("content/plugins").join(plugin);
        assert!(
            path.exists(),
            "bundled plugin {plugin} is missing at {}.\n\
             WASM plugins are not committed to git — they are produced from\n\
             `plugins/` and CI rebuilds them on every job. Locally, run\n\
             `just build-plugins` once (and again after editing anything\n\
             under `plugins/` or `content/wit/`).",
            path.display(),
        );
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let wit_root = workspace_root.join("content/wit/nanite-plugin");
    for wit in ["world.wit", "plugin.wit", "types.wit"] {
        let path = wit_root.join(wit);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
