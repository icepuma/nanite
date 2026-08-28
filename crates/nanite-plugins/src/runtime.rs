//! wasmtime runtime: engine, linker, host state, plugin instantiation.

use anyhow::{Context, Result};
use std::sync::OnceLock;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{
    WasiHttpCtx, WasiHttpCtxView, WasiHttpHooks, WasiHttpView, default_hooks,
};

/// Adapter so we can chain `.context(...)` onto `wasmtime::Result`.
/// wasmtime's `Error` type doesn't impl `std::error::Error` (it can't
/// without conflicting with anyhow's blanket From impl), so anyhow's
/// `Context` blanket isn't available against it directly. The
/// `wasmtime/anyhow` feature gives us `From<wasmtime::Error> for
/// anyhow::Error`, which is what `into_anyhow` exploits.
trait WtResultExt<T> {
    fn into_anyhow(self) -> Result<T>;
}

impl<T> WtResultExt<T> for wasmtime::Result<T> {
    fn into_anyhow(self) -> Result<T> {
        self.map_err(anyhow::Error::from)
    }
}

/// Ensures rustls' process-wide `CryptoProvider` is installed exactly
/// once. Required because `wasmtime-wasi-http/default-send-request`
/// pulls in rustls 0.23 without picking a default provider — without
/// this call, the first outbound HTTPS request panics from a worker
/// thread with "Could not automatically determine the process-level
/// `CryptoProvider`".
fn ensure_crypto_provider() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        // `install_default` returns `Err` if a provider is already
        // installed (e.g. tests sharing the process). Either outcome
        // is fine here — we just need *some* provider in place.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

use crate::host_impl;
use crate::types::{LogLevel, PluginError, PluginInfo, RepoEntry};

wasmtime::component::bindgen!({
    path: "../../content/wit/nanite-plugin",
    world: "clone-plugin",
});

// Re-export the generated `host` interface module so host_impl.rs can
// reach its trait and types without importing the full package path.
pub use nanite::plugin::host as wit_host;

pub type LogSink = Box<dyn Fn(LogLevel, &str) + Send + Sync>;

pub struct HostState {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
    pub env_allowlist: Vec<&'static str>,
    pub log_sink: LogSink,
}

impl HostState {
    pub fn new(env_allowlist: Vec<&'static str>, log_sink: LogSink) -> Self {
        // No env, no preopens, no stdio. Plugins get exactly what we hand them.
        let wasi = WasiCtxBuilder::new().build();
        Self {
            wasi,
            http: WasiHttpCtx::new(),
            table: ResourceTable::new(),
            env_allowlist,
            log_sink,
        }
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for HostState {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        // `default_hooks()` returns a `&'static mut dyn WasiHttpHooks`
        // pointing at a zero-sized type — the default behavior is fine
        // for our outbound-only use case.
        let hooks: &mut dyn WasiHttpHooks = default_hooks();
        WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks,
        }
    }
}

pub struct PluginRuntime {
    engine: Engine,
    linker: Linker<HostState>,
}

impl PluginRuntime {
    pub fn new() -> Result<Self> {
        ensure_crypto_provider();

        let mut config = wasmtime::Config::new();
        config.wasm_component_model(true);
        let engine = Engine::new(&config)
            .into_anyhow()
            .context("failed to build wasmtime engine")?;

        let mut linker: Linker<HostState> = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .into_anyhow()
            .context("failed to register wasi p2 imports")?;
        wasmtime_wasi_http::p2::add_only_http_to_linker_sync(&mut linker)
            .into_anyhow()
            .context("failed to register wasi-http imports")?;
        host_impl::add_to_linker(&mut linker).context("failed to register nanite:plugin/host")?;

        Ok(Self { engine, linker })
    }

    fn instantiate(
        &self,
        wasm_bytes: &[u8],
        state: HostState,
    ) -> Result<(ClonePlugin, Store<HostState>)> {
        let component = Component::from_binary(&self.engine, wasm_bytes)
            .into_anyhow()
            .context("invalid wasm component")?;
        let mut store = Store::new(&self.engine, state);
        let bindings = ClonePlugin::instantiate(&mut store, &component, &self.linker)
            .into_anyhow()
            .context("failed to instantiate plugin")?;
        Ok((bindings, store))
    }

    pub fn run_info(&self, wasm_bytes: &[u8], state: HostState) -> Result<PluginInfo> {
        let (bindings, mut store) = self.instantiate(wasm_bytes, state)?;
        let info = bindings
            .nanite_plugin_plugin()
            .call_info(&mut store)
            .into_anyhow()
            .context("plugin info() trapped")?;
        Ok(PluginInfo {
            name: info.name,
            version: info.version,
            supported_hosts: info.supported_hosts,
        })
    }

    pub fn run_list_repos(
        &self,
        wasm_bytes: &[u8],
        state: HostState,
        input: crate::types::ListInput,
    ) -> Result<Result<Vec<RepoEntry>, PluginError>> {
        let wit_input = exports::nanite::plugin::plugin::ListInput {
            target: input.target,
            include_archived: input.include_archived,
            include_forks: input.include_forks,
            exclude_patterns: input.exclude_patterns,
            page_size: input.page_size,
        };

        let (bindings, mut store) = self.instantiate(wasm_bytes, state)?;
        let result = bindings
            .nanite_plugin_plugin()
            .call_list_repos(&mut store, &wit_input)
            .into_anyhow()
            .context("plugin list-repos() trapped")?;

        Ok(match result {
            Ok(entries) => Ok(entries
                .into_iter()
                .map(|e| RepoEntry {
                    name: e.name,
                    clone_url: e.clone_url,
                    default_branch: e.default_branch,
                    archived: e.archived,
                    fork: e.fork,
                    description: e.description,
                })
                .collect()),
            Err(err) => Err(map_plugin_error(err)),
        })
    }
}

fn map_plugin_error(err: nanite::plugin::types::PluginError) -> PluginError {
    use nanite::plugin::types::PluginError as WitError;
    match err {
        WitError::AuthRequired(msg) => PluginError::AuthRequired(msg),
        WitError::NotFound(target) => PluginError::NotFound(target),
        WitError::RateLimited(limit) => PluginError::RateLimited(crate::types::RateLimit {
            retry_after_seconds: limit.retry_after_seconds,
            message: limit.message,
        }),
        WitError::Network(msg) => PluginError::Network(msg),
        WitError::Decode(msg) => PluginError::Decode(msg),
        WitError::Unsupported(msg) => PluginError::Unsupported(msg),
        WitError::Internal(msg) => PluginError::Internal(msg),
    }
}
