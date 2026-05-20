//! Implementation of the `nanite:plugin/host` interface imports.

use anyhow::Result;
use wasmtime::component::{HasSelf, Linker};

use crate::runtime::{HostState, wit_host};
use crate::types::LogLevel;

pub fn add_to_linker(linker: &mut Linker<HostState>) -> Result<()> {
    wit_host::add_to_linker::<HostState, HasSelf<HostState>>(linker, |state| state)
        .map_err(anyhow::Error::from)
}

impl wit_host::Host for HostState {
    fn get_env(&mut self, name: String) -> Option<String> {
        let allowed = self
            .env_allowlist
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&name));
        if !allowed {
            return None;
        }
        std::env::var(&name).ok()
    }

    fn log(&mut self, level: wit_host::LogLevel, message: String) {
        let mapped = match level {
            wit_host::LogLevel::Trace => LogLevel::Trace,
            wit_host::LogLevel::Debug => LogLevel::Debug,
            wit_host::LogLevel::Info => LogLevel::Info,
            wit_host::LogLevel::Warn => LogLevel::Warn,
            wit_host::LogLevel::Error => LogLevel::Error,
        };
        (self.log_sink)(mapped, &message);
    }
}
