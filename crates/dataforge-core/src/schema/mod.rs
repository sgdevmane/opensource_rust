// =============================================================================
// DataForge Core — Schema Module
// =============================================================================
pub mod infer;
pub mod validate;

pub use infer::infer_schema;
pub use validate::validate_batch;
