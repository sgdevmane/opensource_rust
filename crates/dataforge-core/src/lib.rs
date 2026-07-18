// =============================================================================
// DataForge Core — Public API Surface
// =============================================================================
//! # DataForge
//!
//! High-performance streaming spreadsheet engine for massive CSV/XLSX/ODS
//! data processing. Process millions of rows with constant memory usage.
//!
//! ## Key Features
//!
//! - **Streaming**: Row-by-row processing without loading entire files into memory
//! - **Parallel**: Multi-threaded chunk processing for large CSV files
//! - **Memory-bounded**: Configurable memory limits with backpressure
//! - **Cross-format**: Read/write CSV, XLSX, and ODS formats
//! - **Transform pipeline**: Filter, map, aggregate, sort — all streaming
//! - **Schema inference**: Automatic column type detection
//!
//! ## Quick Start
//!
//! ```no_run
//! use dataforge_core::csv::CsvReader;
//! use dataforge_core::config::ReaderConfig;
//!
//! // Stream a large CSV file in batches
//! let config = ReaderConfig::default()
//!     .with_batch_size(8192)
//!     .with_parallel(true);
//!
//! let reader = CsvReader::open("huge.csv", config).unwrap();
//! for batch in reader {
//!     let batch = batch.unwrap();
//!     println!("Processing {} rows", batch.len());
//! }
//! ```
//!
//! ## Architecture
//!
//! DataForge is organized into these modules:
//!
//! - [`csv`] — Streaming CSV reader/writer with parallel support
//! - [`xlsx`] — Streaming XLSX reader/writer (SAX-style XML parsing)
//! - [`ods`] — Streaming ODS reader/writer (OpenDocument format)
//! - [`transform`] — Composable pipeline: filter, map, aggregate, sort
//! - [`schema`] — Automatic type inference and validation
//! - [`convert`] — Format conversion (CSV ↔ XLSX ↔ ODS)
//! - [`memory`] — Memory tracking with backpressure
//! - [`config`] — Builder-pattern configuration
//! - [`types`] — Core data types (CellValue, Row, RowBatch)
//! - [`error`] — Unified error handling with FFI error codes
//! - [`parallel`] — Thread pool and file chunking

// Use mimalloc as the global allocator for better performance
// with many small allocations (which is common in spreadsheet processing).
// mimalloc reduces fragmentation and improves multi-threaded allocation.
#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Configuration types for readers, writers, and transforms.
pub mod config;

/// Format conversion utilities (CSV ↔ XLSX ↔ ODS).
pub mod convert;

/// Streaming CSV reader and writer.
pub mod csv;

/// Unified error types with FFI-compatible error codes.
pub mod error;

/// Memory tracking and backpressure system.
pub mod memory;

/// Streaming ODS (OpenDocument Spreadsheet) reader and writer.
pub mod ods;

/// Parallel processing utilities (chunking, thread pool).
pub mod parallel;

/// Schema inference and validation.
pub mod schema;

/// Data transformation pipeline (filter, map, aggregate, sort).
pub mod transform;

/// Spreadsheet formula evaluation engine.
pub mod formula;

/// Custom WASM plugin engine
pub mod plugins;

/// Streaming JSON/JSONL reader.
pub mod json;

/// Apache Arrow integration and IPC export.
pub mod arrow;

/// Core data types: CellValue, Row, RowBatch, ColumnSchema, DataType.
pub mod types;

/// Streaming XLSX (Excel 2007+) reader and writer.
pub mod xlsx;

/// Apache Parquet streaming integration.
pub mod parquet;

/// Delta Lake exporter.
pub mod delta;

// =============================================================================
// Re-exports for convenient access
// =============================================================================

pub use config::{FileFormat, ReaderConfig, WriterConfig};
pub use error::{DataForgeError, Result};
pub use formula::{FormulaEvaluator, SqlEngine};
pub use plugins::WasmPlugin;
pub use json::JsonReader;
pub use arrow::ArrowIpcWriter;
pub use parquet::{ParquetReader, ParquetWriter};
pub use delta::DeltaLakeExporter;
pub use types::{CellValue, ColumnSchema, DataType, Row, RowBatch, SheetMetadata};
