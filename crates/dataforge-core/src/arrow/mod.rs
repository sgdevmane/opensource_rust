// =============================================================================
// DataForge Core — Apache Arrow Module
// =============================================================================
// Exposes functions for converting and streaming to Apache Arrow IPC format.
// =============================================================================

pub mod export;

pub use export::{ArrowIpcWriter, ArrowIpcStreamWriter, infer_arrow_schema, row_batch_to_arrow_columns};
