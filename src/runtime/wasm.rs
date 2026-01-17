use wasmer::{Store, Module, Instance, Value, Imports};
use anyhow::{Result, Context};
use std::path::Path;

pub struct WasmRuntime {
    store: Store,
}

impl WasmRuntime {
    pub fn new() -> Self {
        Self {
            store: Store::default(),
        }
    }

    /// Run a specific function from a WASM file with numeric arguments
    /// Currently supports (i32, i32) -> i32 for simplicity.
    pub fn execute(&mut self, wasm_path: &Path, func_name: &str, args: &[i32]) -> Result<i32> {
        let wasm_bytes = std::fs::read(wasm_path).context("Failed to read WASM file")?;
        let module = Module::new(&self.store, wasm_bytes).context("Failed to compile WASM module")?;
        
        let import_object = Imports::new();
        let instance = Instance::new(&mut self.store, &module, &import_object).context("Failed to instantiate WASM module")?;

        let func = instance.exports.get_function(func_name).context("Function not found")?;
        
        let wasm_args: Vec<Value> = args.iter().map(|&x| Value::I32(x)).collect();
        let result = func.call(&mut self.store, &wasm_args).context("Failed to call function")?;

        if let Some(Value::I32(res)) = result.get(0) {
            Ok(*res)
        } else {
            Err(anyhow::anyhow!("Function returned unexpected type or no value"))
        }
    }
}
