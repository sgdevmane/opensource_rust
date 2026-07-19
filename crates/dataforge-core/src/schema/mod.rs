// =============================================================================
// DataForge Core — Schema Module
// =============================================================================
pub mod infer;
pub mod validate;
pub mod drift;

pub use infer::infer_schema;
pub use validate::{validate_batch, apply_schema};
pub use drift::{SchemaDriftHandler, SchemaDriftConfig};
