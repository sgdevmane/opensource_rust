// =============================================================================
// DataForge Core — CSV Module
// =============================================================================
// Streaming CSV read and write with support for both sequential and parallel
// processing modes. This module is the primary entry point for CSV operations.
// =============================================================================

pub mod reader;
pub mod writer;
pub mod sniffer;

pub use reader::CsvReader;
pub use writer::CsvWriter;
pub use sniffer::{CsvSniffer, SniffedDialect};
