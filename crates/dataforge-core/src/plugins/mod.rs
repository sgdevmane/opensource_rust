// =============================================================================
// DataForge Core — Plugins Module
// =============================================================================
// Custom user extension plugins using WebAssembly.
// =============================================================================

pub mod wasm;
pub mod js;

pub use wasm::WasmPlugin;
pub use js::JsEngine;
