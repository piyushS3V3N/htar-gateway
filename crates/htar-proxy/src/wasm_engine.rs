use dashmap::DashMap;
use std::sync::Arc;
use tracing::{info, warn};
use wasmtime::{Config, Engine, Linker, Module, Store};

/// Result action returned by Wasm Plugin Guest Execution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmActionResult {
    Continue = 0,
    HeaderRewrite = 1,
    AccessDenied = 2,
}

impl From<i32> for WasmActionResult {
    fn from(val: i32) -> Self {
        match val {
            1 => WasmActionResult::HeaderRewrite,
            2 => WasmActionResult::AccessDenied,
            _ => WasmActionResult::Continue,
        }
    }
}

pub struct WasmPlugin {
    pub name: String,
    pub module: Module,
}

#[derive(Clone)]
pub struct WasmPluginEngine {
    engine: Engine,
    plugins: Arc<DashMap<String, WasmPlugin>>,
}

impl WasmPluginEngine {
    pub fn new() -> anyhow::Result<Self> {
        let mut config = Config::new();
        config.async_support(false);
        config.cranelift_opt_level(wasmtime::OptLevel::Speed);
        
        let engine = Engine::new(&config)?;
        Ok(Self {
            engine,
            plugins: Arc::new(DashMap::new()),
        })
    }

    /// Dynamically load and compile a WebAssembly plugin module from binary bytecode
    pub fn register_plugin(&self, name: String, wasm_bytes: &[u8]) -> anyhow::Result<()> {
        let module = Module::new(&self.engine, wasm_bytes)?;
        info!("Successfully compiled and registered Wasm Plugin '{}' (size: {} bytes)", name, wasm_bytes.len());
        self.plugins.insert(name.clone(), WasmPlugin { name, module });
        Ok(())
    }

    pub fn list_plugins(&self) -> Vec<String> {
        self.plugins.iter().map(|p| p.key().clone()).collect()
    }

    /// Execute on_request_headers export for a given plugin with context ID
    pub fn execute_request_headers(&self, plugin_name: &str, context_id: u32) -> WasmActionResult {
        let Some(plugin) = self.plugins.get(plugin_name) else {
            return WasmActionResult::Continue;
        };

        let mut store = Store::new(&self.engine, ());
        let linker = Linker::new(&self.engine);

        match linker.instantiate(&mut store, &plugin.module) {
            Ok(instance) => {
                if let Ok(func) = instance.get_typed_func::<u32, i32>(&mut store, "on_request_headers") {
                    match func.call(&mut store, context_id) {
                        Ok(res) => res.into(),
                        Err(e) => {
                            warn!("Wasm plugin '{}' runtime error in on_request_headers: {}", plugin_name, e);
                            WasmActionResult::Continue
                        }
                    }
                } else {
                    WasmActionResult::Continue
                }
            }
            Err(e) => {
                warn!("Failed to instantiate Wasm plugin '{}': {}", plugin_name, e);
                WasmActionResult::Continue
            }
        }
    }

    /// Execute all registered WASM plugins in sequence
    pub fn execute_all_request_plugins(&self, context_id: u32) -> WasmActionResult {
        for entry in self.plugins.iter() {
            let res = self.execute_request_headers(entry.key(), context_id);
            if res != WasmActionResult::Continue {
                return res;
            }
        }
        WasmActionResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasm_engine_register_and_execute() {
        let engine = WasmPluginEngine::new().expect("Failed to initialize Wasm engine");
        
        // Minimal valid WebAssembly binary bytecode (magic \0asm + version 1)
        // containing a function that returns an i32
        let wasm_bytes = include_bytes!("../../../examples/sample_plugin.wasm");
        
        let res = engine.register_plugin("sample_plugin".to_string(), wasm_bytes);
        assert!(res.is_ok(), "Failed to register sample wasm plugin: {:?}", res.err());
        assert_eq!(engine.list_plugins(), vec!["sample_plugin"]);

        // Calling execution when on_request_headers export is absent returns Continue gracefully
        let result = engine.execute_request_headers("sample_plugin", 1);
        assert_eq!(result, WasmActionResult::Continue);
    }
}

