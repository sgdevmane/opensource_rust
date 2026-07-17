// =============================================================================
// DataForge WebAssembly — wasm-bindgen Bindings
// =============================================================================
// Exposes the streaming engine to browsers and Edge environments.
//
// Key features:
// - Parses from Uint8Array buffers (in-memory) instead of file system
// - Returns deserialized JS arrays of objects for seamless UI binding
// =============================================================================

use wasm_bindgen::prelude::*;

use dataforge_core::config::ReaderConfig;
use dataforge_core::csv::CsvReader;
use dataforge_core::xlsx::XlsxReader;
use dataforge_core::ods::OdsReader;

/// Lightweight JS RowBatch wrapper.
#[wasm_bindgen]
pub struct WasmRowBatch {
    inner: dataforge_core::types::RowBatch,
}

#[wasm_bindgen]
impl WasmRowBatch {
    #[wasm_bindgen(getter)]
    pub fn row_count(&self) -> u32 {
        self.inner.len() as u32
    }

    #[wasm_bindgen(getter)]
    pub fn headers(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.inner.headers).unwrap_or(JsValue::NULL)
    }

    /// Convert batch to a JS Array of Plain Objects.
    #[wasm_bindgen]
    pub fn to_json_objects(&self) -> JsValue {
        let headers = self.inner.headers.as_ref();
        let list: Vec<serde_json::Value> = self.inner.rows.iter().map(|row| {
            let mut map = serde_json::Map::new();
            for (col_idx, cell) in row.cells.iter().enumerate() {
                let col_name = headers
                    .and_then(|h| h.get(col_idx))
                    .map(|s| s.clone())
                    .unwrap_or_else(|| format!("col_{}", col_idx));
                
                let val = match cell {
                    dataforge_core::types::CellValue::Null => serde_json::Value::Null,
                    dataforge_core::types::CellValue::Bool(b) => serde_json::Value::Bool(*b),
                    dataforge_core::types::CellValue::Int(i) => serde_json::Value::Number((*i).into()),
                    dataforge_core::types::CellValue::Float(f) => serde_json::Value::Number(
                        serde_json::Number::from_f64(*f).unwrap_or_else(|| 0.into())
                    ),
                    _ => serde_json::Value::String(cell.to_display_string()),
                };
                map.insert(col_name, val);
            }
            serde_json::Value::Object(map)
        }).collect();

        serde_wasm_bindgen::to_value(&list).unwrap_or(JsValue::NULL)
    }
}

/// CSV Streaming Reader for WASM.
#[wasm_bindgen]
pub struct WasmCsvReader {
    inner: CsvReader,
}

#[wasm_bindgen]
impl WasmCsvReader {
    #[wasm_bindgen(constructor)]
    pub fn new(data: &[u8], batch_size: Option<u32>) -> Result<WasmCsvReader, JsValue> {
        let mut config = ReaderConfig::default().with_parallel(false); // Single-threaded in WASM
        if let Some(bs) = batch_size {
            config = config.with_batch_size(bs as usize);
        }

        CsvReader::from_bytes(data.to_vec(), config)
            .map(|r| WasmCsvReader { inner: r })
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(getter)]
    pub fn headers(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.inner.headers()).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn next_batch(&mut self) -> Result<Option<WasmRowBatch>, JsValue> {
        match self.inner.next_batch() {
            Some(Ok(batch)) => Ok(Some(WasmRowBatch { inner: batch })),
            Some(Err(e)) => Err(JsValue::from_str(&e.to_string())),
            None => Ok(None),
        }
    }
}

/// XLSX Streaming Reader for WASM.
#[wasm_bindgen]
pub struct WasmXlsxReader {
    inner: XlsxReader,
}

#[wasm_bindgen]
impl WasmXlsxReader {
    #[wasm_bindgen(constructor)]
    pub fn new(data: &[u8], batch_size: Option<u32>, sheet_name: Option<String>) -> Result<WasmXlsxReader, JsValue> {
        let mut config = ReaderConfig::default();
        if let Some(bs) = batch_size {
            config = config.with_batch_size(bs as usize);
        }
        if let Some(ref sn) = sheet_name {
            config = config.with_sheet_name(sn);
        }

        XlsxReader::from_bytes(data.to_vec(), config)
            .map(|r| WasmXlsxReader { inner: r })
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(getter)]
    pub fn headers(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.inner.headers()).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn next_batch(&mut self) -> Result<Option<WasmRowBatch>, JsValue> {
        match self.inner.next_batch() {
            Some(Ok(batch)) => Ok(Some(WasmRowBatch { inner: batch })),
            Some(Err(e)) => Err(JsValue::from_str(&e.to_string())),
            None => Ok(None),
        }
    }
}

/// ODS Streaming Reader for WASM.
#[wasm_bindgen]
pub struct WasmOdsReader {
    inner: OdsReader,
}

#[wasm_bindgen]
impl WasmOdsReader {
    #[wasm_bindgen(constructor)]
    pub fn new(data: &[u8], batch_size: Option<u32>, sheet_name: Option<String>) -> Result<WasmOdsReader, JsValue> {
        let mut config = ReaderConfig::default();
        if let Some(bs) = batch_size {
            config = config.with_batch_size(bs as usize);
        }
        if let Some(ref sn) = sheet_name {
            config = config.with_sheet_name(sn);
        }

        OdsReader::from_bytes(data.to_vec(), config)
            .map(|r| WasmOdsReader { inner: r })
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(getter)]
    pub fn headers(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.inner.headers()).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn next_batch(&mut self) -> Result<Option<WasmRowBatch>, JsValue> {
        match self.inner.next_batch() {
            Some(Ok(batch)) => Ok(Some(WasmRowBatch { inner: batch })),
            Some(Err(e)) => Err(JsValue::from_str(&e.to_string())),
            None => Ok(None),
        }
     }
}

/// Initialize Rayon thread pool in WASM (only works when compiled with atomics).
#[wasm_bindgen]
pub fn init_thread_pool(num_threads: usize) -> Result<(), JsValue> {
    #[cfg(target_feature = "atomics")]
    {
        wasm_bindgen_rayon::init_thread_pool(num_threads)
    }
    #[cfg(not(target_feature = "atomics"))]
    {
        let _ = num_threads;
        Err(JsValue::from_str("Multi-threading is not supported without atomics. Compile with RUSTFLAGS='-C target-feature=+atomics'"))
    }
}
