// =============================================================================
// DataForge Core — Transform Pipeline
// =============================================================================
// Composable data transformation pipeline for streaming row processing.
// All transformations are lazy and operate on batches, maintaining the
// streaming property — no full-file materialization needed.
// =============================================================================

pub mod aggregate;
pub mod filter;
pub mod map;
pub mod pipeline;
pub mod sort;

pub use pipeline::Pipeline;
