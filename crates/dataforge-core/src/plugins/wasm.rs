// =============================================================================
// DataForge Core — Custom WASM Plugin Engine
// =============================================================================
// Runs compiled WebAssembly plugin modules over RowBatch cell streams.
// =============================================================================

use wasmi::{Engine, Linker, Module, Store, Instance, Value};
use crate::error::{DataForgeError, Result};
use crate::types::{CellValue, RowBatch};

/// WebAssembly transformation plugin.
pub struct WasmPlugin {
    store: Store<()>,
    instance: Instance,
}

impl WasmPlugin {
    /// Compile and instantiate a WASM plugin from binary bytecode bytes.
    pub fn new(wasm_bytes: &[u8]) -> Result<Self> {
        let engine = Engine::default();
        let module = Module::new(&engine, wasm_bytes).map_err(|e| {
            DataForgeError::config(format!("Failed to compile WASM module: {e}"))
        })?;
        let mut store = Store::new(&engine, ());
        let linker = Linker::new(&engine);
        
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| DataForgeError::config(format!("Failed to instantiate WASM: {e}")))?
            .start(&mut store)
            .map_err(|e| DataForgeError::config(format!("Failed to start WASM: {e}")))?;

        Ok(WasmPlugin { store, instance })
    }

    /// Run the custom `transform_number` function on a single CellValue.
    pub fn transform_cell(&mut self, val: &CellValue) -> Result<CellValue> {
        let func = self
            .instance
            .get_func(&self.store, "transform_number")
            .ok_or_else(|| {
                DataForgeError::config("WASM function 'transform_number' not found")
            })?;

        let val_f64 = match val {
            CellValue::Int(i) => *i as f64,
            CellValue::Float(f) => *f,
            _ => return Ok(val.clone()),
        };

        let mut results = [Value::F64(0.0.into())];
        func.call(&mut self.store, &[Value::F64(val_f64.into())], &mut results)
            .map_err(|e| {
                DataForgeError::internal(format!("WASM call failed: {e}"))
            })?;

        let res = match results[0] {
            Value::F64(v) => v.to_float(),
            _ => return Err(DataForgeError::internal("WASM returned non-f64 value")),
        };

        match val {
            CellValue::Int(_) => Ok(CellValue::Int(res as i64)),
            CellValue::Float(_) => Ok(CellValue::Float(res)),
            _ => Ok(val.clone()),
        }
    }

    /// Apply the transformation to a specific column index across all rows in a RowBatch.
    pub fn transform_column(&mut self, batch: &mut RowBatch, col_idx: usize) -> Result<()> {
        for row in &mut batch.rows {
            if let Some(cell) = row.cells.get_mut(col_idx) {
                *cell = self.transform_cell(cell)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compiled WASM bytecode for:
    // fn transform_number(x: f64) -> f64 { x * 2.0 }
    const WASM_BYTES: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, // Magic
        0x01, 0x00, 0x00, 0x00, // Version
        // Type section
        0x01, 0x06, 0x01, 0x60, 0x01, 0x7c, 0x01, 0x7c,
        // Function section
        0x03, 0x02, 0x01, 0x00,
        // Export section
        0x07, 0x14, 0x01, 0x10,
        0x74, 0x72, 0x61, 0x6e, 0x73, 0x66, 0x6f, 0x72, 0x6d, 0x5f, 0x6e, 0x75, 0x6d, 0x62, 0x65, 0x72,
        0x00, 0x00,
        // Code section
        0x0a, 0x10, 0x01, 0x0e, 0x00,
        0x20, 0x00, // local.get 0
        0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, // f64.const 2.0 (little-endian)
        0xa2, // f64.mul
        0x0b, // end
    ];

    #[test]
    fn test_wasm_plugin() {
        let mut plugin = WasmPlugin::new(WASM_BYTES).unwrap();

        let val_int = CellValue::Int(10);
        let res_int = plugin.transform_cell(&val_int).unwrap();
        assert_eq!(res_int, CellValue::Int(20));

        let val_float = CellValue::Float(2.5);
        let res_float = plugin.transform_cell(&val_float).unwrap();
        assert!((res_float.as_float().unwrap() - 5.0).abs() < 1e-9);
    }
}
