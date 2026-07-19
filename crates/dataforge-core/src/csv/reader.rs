// =============================================================================
// DataForge Core — Streaming CSV Reader
// =============================================================================
// High-performance CSV reader supporting both sequential and parallel modes.
//
// ## Sequential Mode (default for small files or piped input)
// Uses the `csv` crate's `Reader` with a `BufReader` for standard streaming.
// Memory usage is proportional to batch_size, not file size.
//
// ## Parallel Mode (for large files on disk)
// 1. Memory-maps the file via `memmap2` (OS handles paging)
// 2. Splits the mapped region into chunks at newline boundaries using `memchr`
// 3. Each chunk is parsed independently by a Rayon worker thread
// 4. Results are collected via crossbeam channels, maintaining row order
//
// This design achieves near-linear scaling with CPU core count on large files
// while keeping memory usage constant regardless of file size.
// =============================================================================

use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::Path;
use std::sync::Arc;

use compact_str::CompactString;
use crossbeam_channel::{bounded, Receiver};
use memchr::memchr;
use memmap2::Mmap;
use rayon::prelude::*;
use tracing::{debug, info, warn};

use super::sniffer::CsvSniffer;

use crate::config::{Encoding, ReaderConfig};
use crate::error::{DataForgeError, Result};
use crate::memory::MemoryTracker;
use crate::types::{CellValue, Row, RowBatch};

/// Streaming CSV reader that processes data in configurable batches.
///
/// The reader implements `Iterator<Item = Result<RowBatch>>`, producing
/// batches of rows until the file is fully consumed. Each batch contains
/// up to `config.batch_size` rows.
///
/// # Memory Guarantees
/// - Sequential mode: O(batch_size) memory
/// - Parallel mode: O(batch_size × num_threads) memory
/// - Total memory never exceeds `config.max_memory_bytes`
///
/// # Example
/// ```no_run
/// use dataforge_core::csv::CsvReader;
/// use dataforge_core::config::ReaderConfig;
///
/// let reader = CsvReader::open("data.csv", ReaderConfig::default()).unwrap();
/// for batch in reader {
///     let batch = batch.unwrap();
///     for row in batch.iter() {
///         println!("Row {}: {:?}", row.index, row.cells);
///     }
/// }
/// ```
pub struct CsvReader {
    /// The underlying data source (either batches from a receiver or direct iterator)
    inner: CsvReaderInner,

    /// Column headers (detected from first row or provided via config)
    headers: Option<Vec<String>>,

    /// Memory tracker for backpressure
    memory_tracker: Arc<MemoryTracker>,

    /// Whether we've finished reading
    exhausted: bool,

    /// Configuration snapshot
    config: ReaderConfig,

    /// Dynamic batch size determined by auto-tuning (or initial batch_size)
    current_batch_size: usize,
}

/// Internal implementation — either sequential or parallel mode.
enum CsvReaderInner {
    /// Sequential mode: reads rows one-at-a-time from a csv::Reader
    Sequential(SequentialReader),

    /// Parallel mode: receives pre-parsed batches from worker threads
    Parallel(ParallelReader),

    /// Bytes mode: reads from an in-memory byte slice (for WASM/FFI)
    Bytes(BytesReader),
}

/// Sequential CSV reading using the `csv` crate.
struct SequentialReader {
    /// The csv crate reader instance
    reader: csv::Reader<BufReader<File>>,

    /// Current row index
    row_index: u64,

    /// Number of rows read so far (data rows, not headers)
    rows_read: u64,
}

/// Sequential reader from in-memory bytes (for WASM or FFI use).
struct BytesReader {
    reader: csv::Reader<Cursor<Vec<u8>>>,
    row_index: u64,
    rows_read: u64,
}

/// Parallel CSV reading using memory mapping + Rayon.
struct ParallelReader {
    /// Channel receiver for completed batches from worker threads
    receiver: Receiver<Result<RowBatch>>,
}

/// Describes a chunk of a file for parallel processing.
///
/// Each chunk starts and ends at a complete row boundary,
/// so it can be parsed independently without coordination.
#[derive(Debug, Clone, Copy)]
struct ChunkRange {
    /// Byte offset from the start of the file
    offset: usize,
    /// Number of bytes in this chunk
    length: usize,
    /// Starting row index for this chunk (estimated for non-first chunks)
    start_row: u64,
}

impl CsvReader {
    /// Open a CSV file from a filesystem path.
    ///
    /// Automatically selects sequential or parallel mode based on file size
    /// and configuration. Files larger than 10MB default to parallel mode.
    ///
    /// # Arguments
    /// * `path` - Path to the CSV file
    /// * `config` - Reader configuration
    ///
    /// # Errors
    /// Returns `DataForgeError::Io` if the file cannot be opened.
    /// Returns `DataForgeError::Config` if the configuration is invalid.
    pub fn open(path: impl AsRef<Path>, mut config: ReaderConfig) -> Result<Self> {
        let path = path.as_ref();
        config.validate()?;

        // Sniff CSV dialect if enabled
        if config.csv.auto_detect_dialect {
            let mut file = std::fs::File::open(path).map_err(|e| {
                DataForgeError::io(e, format!("Failed to open CSV file '{}' for sniffing", path.display()))
            })?;
            use std::io::Read;
            let mut sample = vec![0; 2048];
            let bytes_read = file.read(&mut sample).unwrap_or(0);
            sample.truncate(bytes_read);
            if let Ok(dialect) = CsvSniffer::sniff(&sample) {
                config.csv.delimiter = dialect.delimiter;
                config.csv.quote_char = dialect.quote_char;
                config.csv.has_header = dialect.has_header;
                info!(
                    delimiter = %((dialect.delimiter as char).to_string()),
                    quote = %((dialect.quote_char as char).to_string()),
                    has_header = dialect.has_header,
                    "Auto-detected CSV dialect"
                );
            }
        }

        // Create memory tracker based on config
        let memory_tracker = MemoryTracker::new(
            config.max_memory_bytes,
            config.backpressure,
        );

        // Get file metadata to decide on mode
        let metadata = std::fs::metadata(path).map_err(|e| {
            DataForgeError::io(e, format!("Failed to read metadata for '{}'", path.display()))
        })?;
        let file_size = metadata.len() as usize;

        info!(
            path = %path.display(),
            file_size_mb = file_size as f64 / 1_048_576.0,
            parallel = config.parallel,
            batch_size = config.batch_size,
            "Opening CSV file"
        );

        // Use parallel mode for large files when enabled
        let use_parallel = config.parallel && file_size > 10 * 1024 * 1024; // > 10MB

        if use_parallel {
            Self::open_parallel(path, config, memory_tracker, file_size)
        } else {
            Self::open_sequential(path, config, memory_tracker)
        }
    }

    /// Open a CSV from in-memory bytes.
    ///
    /// This is the primary entry point for WASM and FFI consumers
    /// who provide data as byte arrays rather than file paths.
    ///
    /// # Arguments
    /// * `data` - Raw CSV bytes
    /// * `config` - Reader configuration
    pub fn from_bytes(data: Vec<u8>, mut config: ReaderConfig) -> Result<Self> {
        config.validate()?;

        // Sniff CSV dialect if enabled
        if config.csv.auto_detect_dialect {
            let sample_len = data.len().min(2048);
            if let Ok(dialect) = CsvSniffer::sniff(&data[..sample_len]) {
                config.csv.delimiter = dialect.delimiter;
                config.csv.quote_char = dialect.quote_char;
                config.csv.has_header = dialect.has_header;
                info!(
                    delimiter = %((dialect.delimiter as char).to_string()),
                    quote = %((dialect.quote_char as char).to_string()),
                    has_header = dialect.has_header,
                    "Auto-detected CSV dialect from bytes"
                );
            }
        }

        let memory_tracker = MemoryTracker::new(
            config.max_memory_bytes,
            config.backpressure,
        );

        // Handle encoding conversion if needed
        let data = convert_encoding(&data, config.encoding)?;

        // Use the cursor-based reader path
        Self::open_from_cursor(data.into_owned(), config, memory_tracker)
    }

    /// Internal: open from a Cursor (for bytes-based reading).
    fn open_from_cursor(
        data: Vec<u8>,
        config: ReaderConfig,
        memory_tracker: Arc<MemoryTracker>,
    ) -> Result<Self> {
        let data = convert_encoding(&data, config.encoding)?;

        let mut csv_builder = csv::ReaderBuilder::new();
        csv_builder
            .delimiter(config.csv.delimiter)
            .quote(config.csv.quote_char)
            .has_headers(config.csv.has_header)
            .flexible(config.csv.flexible)
            .trim(if config.csv.trim_fields {
                csv::Trim::All
            } else {
                csv::Trim::None
            });

        if let Some(escape) = config.csv.escape_char {
            csv_builder.escape(Some(escape));
        }
        if let Some(comment) = config.csv.comment_char {
            csv_builder.comment(Some(comment));
        }

        let mut reader = csv_builder.from_reader(Cursor::new(data.into_owned()));

        // Extract headers if present
        let headers = if config.csv.has_header {
            let hdrs = reader.headers().map_err(|e| DataForgeError::CsvParse {
                row: 0,
                column: 0,
                message: format!("Failed to read CSV headers: {e}"),
            })?;
            Some(hdrs.iter().map(String::from).collect::<Vec<_>>())
        } else {
            None
        };

        let batch_size = config.batch_size;
        // Use a different enum variant for bytes reading
        Ok(CsvReader {
            inner: CsvReaderInner::Bytes(BytesReader {
                reader,
                row_index: if config.csv.has_header { 1 } else { 0 },
                rows_read: 0,
            }),
            headers,
            memory_tracker,
            exhausted: false,
            config,
            current_batch_size: batch_size,
        })
    }

    /// Open in sequential mode — reads rows one at a time.
    fn open_sequential(
        path: &Path,
        config: ReaderConfig,
        memory_tracker: Arc<MemoryTracker>,
    ) -> Result<Self> {
        let file = File::open(path).map_err(|e| {
            DataForgeError::io(e, format!("Failed to open CSV file '{}'", path.display()))
        })?;
        let buf_reader = BufReader::with_capacity(64 * 1024, file); // 64KB read buffer

        let mut csv_builder = csv::ReaderBuilder::new();
        csv_builder
            .delimiter(config.csv.delimiter)
            .quote(config.csv.quote_char)
            .has_headers(config.csv.has_header)
            .flexible(config.csv.flexible)
            .trim(if config.csv.trim_fields {
                csv::Trim::All
            } else {
                csv::Trim::None
            });

        if let Some(escape) = config.csv.escape_char {
            csv_builder.escape(Some(escape));
        }
        if let Some(comment) = config.csv.comment_char {
            csv_builder.comment(Some(comment));
        }

        let mut reader = csv_builder.from_reader(buf_reader);

        // Extract headers if present
        let headers = if config.csv.has_header {
            let hdrs = reader.headers().map_err(|e| DataForgeError::CsvParse {
                row: 0,
                column: 0,
                message: format!("Failed to read CSV headers: {e}"),
            })?;
            Some(hdrs.iter().map(String::from).collect::<Vec<_>>())
        } else {
            None
        };

        debug!(
            headers = ?headers,
            skip_rows = config.skip_rows,
            "CSV sequential reader initialized"
        );

        let batch_size = config.batch_size;
        Ok(CsvReader {
            inner: CsvReaderInner::Sequential(SequentialReader {
                reader,
                row_index: if config.csv.has_header { 1 } else { 0 },
                rows_read: 0,
            }),
            headers,
            memory_tracker,
            exhausted: false,
            config,
            current_batch_size: batch_size,
        })
    }

    /// Open in parallel mode — memory-maps the file and distributes chunks to workers.
    fn open_parallel(
        path: &Path,
        config: ReaderConfig,
        memory_tracker: Arc<MemoryTracker>,
        file_size: usize,
    ) -> Result<Self> {
        let file = File::open(path).map_err(|e| {
            DataForgeError::io(e, format!("Failed to open CSV file '{}'", path.display()))
        })?;

        // SAFETY: We memory-map the file read-only. The file must not be modified
        // by another process while we have it mapped. This is the standard pattern
        // for parallel file processing in Rust.
        let mmap = unsafe { Mmap::map(&file) }.map_err(|e| {
            DataForgeError::io(e, "Failed to memory-map CSV file")
        })?;

        // Detect and skip BOM if present
        let (data, bom_offset) = skip_bom(&mmap);

        // Read headers from the first line
        let (headers, header_end) = if config.csv.has_header {
            let (hdrs, end) = parse_header_line(data, &config)?;
            (Some(hdrs), end)
        } else {
            (None, 0)
        };

        // Calculate effective data region (after BOM + header)
        let data_start = bom_offset + header_end;
        let data_region = &mmap[data_start..];

        // Determine number of chunks
        let num_threads = config.num_threads.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        });
        let num_chunks = num_threads.max(1);

        // Split file into chunks at newline boundaries
        let chunks = split_into_chunks(data_region, num_chunks, data_start);

        debug!(
            num_chunks = chunks.len(),
            num_threads,
            file_size,
            "CSV parallel reader initialized"
        );

        // Create bounded channel for delivering parsed batches
        // Buffer size limits memory usage: at most (buffer_size × batch) in flight
        let channel_buffer = num_threads * 2;
        let (sender, receiver) = bounded::<Result<RowBatch>>(channel_buffer);

        // Clone config values needed in the worker closure
        let batch_size = config.batch_size;
        let delimiter = config.csv.delimiter;
        let quote_char = config.csv.quote_char;
        let escape_char = config.csv.escape_char;
        let skip_rows = config.skip_rows;
        let max_rows = config.max_rows;
        let trim_fields = config.csv.trim_fields;
        let flexible = config.csv.flexible;
        let comment_char = config.csv.comment_char;
        let selected_columns = config.columns.clone();
        let headers_clone = headers.clone();
        let null_values = config.null_values.clone();

        // We need the mmap to outlive the workers — wrap in Arc
        let mmap = Arc::new(mmap);
        let mmap_clone = Arc::clone(&mmap);

        // Spawn parallel processing on Rayon's thread pool
        std::thread::spawn(move || {
            // Process chunks in parallel using Rayon
            let results: Vec<Vec<RowBatch>> = chunks
                .par_iter()
                .map(|chunk| {
                    parse_chunk(
                        &mmap_clone[chunk.offset..chunk.offset + chunk.length],
                        chunk.start_row,
                        batch_size,
                        delimiter,
                        quote_char,
                        escape_char,
                        trim_fields,
                        flexible,
                        comment_char,
                        &selected_columns,
                        &null_values,
                    )
                })
                .collect();

            // Send results through channel in order
            let mut global_row_index = if skip_rows > 0 { skip_rows } else { 0 };
            let mut total_sent: u64 = 0;

            'outer: for chunk_batches in results {
                for mut batch in chunk_batches {
                    // Apply skip_rows and max_rows filtering
                    if !batch.rows.is_empty() {
                        // Re-index rows globally
                        for row in &mut batch.rows {
                            row.index = global_row_index;
                            global_row_index += 1;
                        }
                        batch.headers = headers_clone.clone();

                        if let Some(max) = max_rows {
                            if total_sent >= max {
                                break 'outer;
                            }
                            let remaining = (max - total_sent) as usize;
                            if batch.rows.len() > remaining {
                                batch.rows.truncate(remaining);
                                batch.is_last = true;
                            }
                        }

                        total_sent += batch.rows.len() as u64;

                        if sender.send(Ok(batch)).is_err() {
                            break 'outer; // Receiver dropped
                        }
                    }
                }
            }
            // Channel closes when sender is dropped
        });

        let batch_size = config.batch_size;
        Ok(CsvReader {
            inner: CsvReaderInner::Parallel(ParallelReader { receiver }),
            headers,
            memory_tracker,
            exhausted: false,
            config,
            current_batch_size: batch_size,
        })
    }

    /// Get the column headers (if detected or provided).
    pub fn headers(&self) -> Option<&[String]> {
        self.headers.as_deref()
    }

    /// Get the current memory usage stats.
    pub fn memory_stats(&self) -> crate::memory::MemoryStats {
        self.memory_tracker.stats()
    }

    /// Read the next batch of rows.
    ///
    /// Returns `None` when the file has been fully consumed.
    /// Returns `Some(Err(...))` on parsing errors.
    pub fn next_batch(&mut self) -> Option<Result<RowBatch>> {
        if self.exhausted {
            return None;
        }

        let mut result = match &mut self.inner {
            CsvReaderInner::Sequential(seq) => {
                Self::read_sequential_batch(seq, &self.config, &self.headers, self.current_batch_size)
            }
            CsvReaderInner::Bytes(seq) => {
                Self::read_bytes_batch(seq, &self.config, &self.headers, self.current_batch_size)
            }
            CsvReaderInner::Parallel(par) => {
                match par.receiver.recv() {
                    Ok(result) => Some(result),
                    Err(_) => None,
                }
            }
        };

        if let Some(Ok(ref mut batch)) = result {
            if self.config.auto_tune_batch_size {
                let mem_bytes = batch.estimated_memory_bytes();
                self.current_batch_size = self.config.tune_batch_size(self.current_batch_size, mem_bytes);
            }
            if let Err(e) = crate::schema::apply_schema(batch, &self.config) {
                result = Some(Err(e));
            }
        }

        match &result {
            Some(Ok(batch)) if batch.is_last => self.exhausted = true,
            Some(Err(_)) => self.exhausted = true,
            None => self.exhausted = true,
            _ => {}
        }
        result
    }

    /// Read a single batch in sequential mode.
    fn read_sequential_batch(
        seq: &mut SequentialReader,
        config: &ReaderConfig,
        headers: &Option<Vec<String>>,
        batch_size: usize,
    ) -> Option<Result<RowBatch>> {
        let mut batch = RowBatch::with_capacity(seq.row_index, batch_size);
        batch.headers = headers.clone();

        let mut record = csv::StringRecord::new();

        for _ in 0..batch_size {
            // Check max_rows limit
            if let Some(max) = config.max_rows {
                if seq.rows_read >= max {
                    batch.is_last = true;
                    break;
                }
            }

            match seq.reader.read_record(&mut record) {
                Ok(true) => {
                    seq.row_index += 1;

                    // Skip rows if configured
                    if seq.row_index - 1 < config.skip_rows + if config.csv.has_header { 1 } else { 0 } {
                        continue;
                    }

                    // Build row from record
                    let row = record_to_row(
                        &record,
                        seq.row_index - 1,
                        &config.columns,
                        &config.null_values,
                    );
                    batch.push(row);
                    seq.rows_read += 1;
                }
                Ok(false) => {
                    // End of file
                    batch.is_last = true;
                    break;
                }
                Err(e) => {
                    return Some(Err(e.into()));
                }
            }
        }

        if batch.is_empty() && batch.is_last {
            return None;
        }

        Some(Ok(batch))
    }

    /// Read a single batch from bytes in memory.
    fn read_bytes_batch(
        seq: &mut BytesReader,
        config: &ReaderConfig,
        headers: &Option<Vec<String>>,
        batch_size: usize,
    ) -> Option<Result<RowBatch>> {
        let mut batch = RowBatch::with_capacity(seq.row_index, batch_size);
        batch.headers = headers.clone();

        let mut record = csv::StringRecord::new();

        for _ in 0..batch_size {
            // Check max_rows limit
            if let Some(max) = config.max_rows {
                if seq.rows_read >= max {
                    batch.is_last = true;
                    break;
                }
            }

            match seq.reader.read_record(&mut record) {
                Ok(true) => {
                    seq.row_index += 1;

                    // Skip rows if configured
                    if seq.row_index - 1 < config.skip_rows + if config.csv.has_header { 1 } else { 0 } {
                        continue;
                    }

                    // Build row from record
                    let row = record_to_row(
                        &record,
                        seq.row_index - 1,
                        &config.columns,
                        &config.null_values,
                    );
                    batch.push(row);
                    seq.rows_read += 1;
                }
                Ok(false) => {
                    // End of file
                    batch.is_last = true;
                    break;
                }
                Err(e) => {
                    return Some(Err(e.into()));
                }
            }
        }

        if batch.is_empty() && batch.is_last {
            return None;
        }

        Some(Ok(batch))
    }
}

/// Implement Iterator for ergonomic `for batch in reader` usage.
impl Iterator for CsvReader {
    type Item = Result<RowBatch>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_batch()
    }
}

// =============================================================================
// Internal helper functions
// =============================================================================

/// Convert a csv::StringRecord into a DataForge Row.
///
/// Applies column selection if configured, and uses type inference
/// to create appropriate CellValue variants (Int, Float, Bool, String).
fn record_to_row(
    record: &csv::StringRecord,
    row_index: u64,
    selected_columns: &Option<Vec<usize>>,
    null_values: &Option<Vec<String>>,
) -> Row {
    let cols = match selected_columns {
        Some(cols) => cols.as_slice(),
        None => &[], // Empty = all columns
    };

    let mut row = Row::with_capacity(
        row_index,
        if cols.is_empty() { record.len() } else { cols.len() },
    );

    if cols.is_empty() {
        // All columns
        for field in record.iter() {
            row.push(parse_cell_value(field, null_values));
        }
    } else {
        // Selected columns only
        for &col_idx in cols {
            if let Some(field) = record.get(col_idx) {
                row.push(parse_cell_value(field, null_values));
            } else {
                row.push(CellValue::Null);
            }
        }
    }

    row
}

/// Parse a string field into the most appropriate CellValue type.
///
/// Attempts type detection in order of specificity:
/// 1. Empty → Null
/// 2. Boolean ("true"/"false", case-insensitive)
/// 3. Integer (pure digits, optional sign)
/// 4. Float (digits with decimal point)
/// 5. String (fallback)
///
/// This avoids forcing all values to strings, which is wasteful for
/// numeric-heavy datasets (typical in data engineering).
fn parse_cell_value(field: &str, null_values: &Option<Vec<String>>) -> CellValue {
    let trimmed = field.trim();

    // Check custom null values
    if let Some(null_list) = null_values {
        if null_list.iter().any(|nv| nv == trimmed || nv == field) {
            return CellValue::Null;
        }
    }

    // Empty fields → Null
    if trimmed.is_empty() {
        return CellValue::Null;
    }

    // Boolean detection (case-insensitive)
    match trimmed.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" if trimmed.len() <= 4 && !trimmed.chars().all(|c| c.is_ascii_digit()) => {
            return CellValue::Bool(true);
        }
        "false" | "no" | "0" if trimmed.len() <= 5 && !trimmed.chars().all(|c| c.is_ascii_digit()) => {
            return CellValue::Bool(false);
        }
        _ => {}
    }

    // Integer detection (using fast lexical parsing)
    if let Ok(v) = lexical_core::parse::<i64>(trimmed.as_bytes()) {
        // Only treat as int if it looks like an integer (no decimal point)
        if !trimmed.contains('.') {
            return CellValue::Int(v);
        }
    }

    // Float detection (using fast lexical parsing)
    if let Ok(v) = lexical_core::parse::<f64>(trimmed.as_bytes()) {
        return CellValue::Float(v);
    }

    // Fallback: string with small-string optimization
    CellValue::String(CompactString::new(trimmed))
}

/// Detect and skip UTF-8 BOM (Byte Order Mark) at the start of a file.
///
/// Many Windows applications (especially Excel) add a BOM (0xEF, 0xBB, 0xBF)
/// to UTF-8 files. We need to skip it to avoid polluting the first field.
fn skip_bom(data: &[u8]) -> (&[u8], usize) {
    if data.len() >= 3 && data[0] == 0xEF && data[1] == 0xBB && data[2] == 0xBF {
        (&data[3..], 3)
    } else {
        (data, 0)
    }
}

/// Parse the header line from raw bytes.
///
/// Returns the header strings and the byte offset where data rows begin.
fn parse_header_line(data: &[u8], config: &ReaderConfig) -> Result<(Vec<String>, usize)> {
    // Find the first newline to extract the header line
    let newline_pos = memchr(b'\n', data).unwrap_or(data.len());
    let header_bytes = &data[..newline_pos];

    // Handle CRLF
    let header_str = std::str::from_utf8(header_bytes)
        .map_err(|e| DataForgeError::Encoding {
            message: format!("Header line is not valid UTF-8: {e}"),
        })?
        .trim_end_matches('\r');

    // Split by delimiter
    let delimiter = config.csv.delimiter as char;
    let headers: Vec<String> = header_str
        .split(delimiter)
        .map(|s| s.trim_matches(config.csv.quote_char as char).to_string())
        .collect();

    // Return headers and offset past the newline
    let offset = if newline_pos < data.len() {
        newline_pos + 1
    } else {
        newline_pos
    };

    Ok((headers, offset))
}

/// Split a byte slice into chunks at newline boundaries.
///
/// Each chunk is guaranteed to start and end at a complete row boundary,
/// making it safe to parse chunks independently in parallel threads.
///
/// # Algorithm
/// 1. Divide the data into N equal-sized regions
/// 2. For each boundary (except the first), scan forward to find the next newline
/// 3. Adjust the boundary to the byte after the newline
/// 4. This ensures each chunk contains only complete rows
///
/// # Arguments
/// * `data` - The byte slice to split (should not include header)
/// * `num_chunks` - Number of chunks to create
/// * `base_offset` - Byte offset of `data` within the original file
fn split_into_chunks(data: &[u8], num_chunks: usize, base_offset: usize) -> Vec<ChunkRange> {
    if data.is_empty() || num_chunks == 0 {
        return Vec::new();
    }

    if num_chunks == 1 || data.len() < 1024 {
        // Single chunk for small data
        return vec![ChunkRange {
            offset: base_offset,
            length: data.len(),
            start_row: 0,
        }];
    }

    let chunk_size = data.len() / num_chunks;
    let mut chunks = Vec::with_capacity(num_chunks);
    let mut current_offset = 0;

    for i in 0..num_chunks {
        if i == num_chunks - 1 {
            // Last chunk takes everything remaining
            if current_offset < data.len() {
                chunks.push(ChunkRange {
                    offset: base_offset + current_offset,
                    length: data.len() - current_offset,
                    start_row: 0, // Will be corrected later
                });
            }
            break;
        }

        // Target end of this chunk
        let target_end = ((i + 1) * chunk_size).min(data.len());

        // Find the next newline at or after the target end
        let actual_end = if target_end >= data.len() {
            data.len()
        } else {
            match memchr(b'\n', &data[target_end..]) {
                Some(pos) => target_end + pos + 1, // Include the newline in this chunk
                None => data.len(),                 // No more newlines — take everything
            }
        };

        if current_offset < actual_end {
            chunks.push(ChunkRange {
                offset: base_offset + current_offset,
                length: actual_end - current_offset,
                start_row: 0, // Will be corrected in the consumer
            });
        }

        current_offset = actual_end;
    }

    debug!(num_chunks = chunks.len(), "File split into chunks");
    chunks
}

/// Parse a single chunk of CSV data into batches of rows.
///
/// This function is called by each Rayon worker thread. It operates
/// on a byte slice from the memory-mapped file and produces RowBatch values.
fn parse_chunk(
    data: &[u8],
    start_row: u64,
    batch_size: usize,
    delimiter: u8,
    quote_char: u8,
    escape_char: Option<u8>,
    trim_fields: bool,
    flexible: bool,
    comment_char: Option<u8>,
    selected_columns: &Option<Vec<usize>>,
    null_values: &Option<Vec<String>>,
) -> Vec<RowBatch> {
    let mut csv_builder = csv::ReaderBuilder::new();
    csv_builder
        .delimiter(delimiter)
        .quote(quote_char)
        .has_headers(false) // Headers already parsed separately
        .flexible(flexible)
        .trim(if trim_fields {
            csv::Trim::All
        } else {
            csv::Trim::None
        });

    if let Some(escape) = escape_char {
        csv_builder.escape(Some(escape));
    }
    if let Some(comment) = comment_char {
        csv_builder.comment(Some(comment));
    }

    let mut reader = csv_builder.from_reader(data);
    let mut batches = Vec::new();
    let mut current_batch = RowBatch::with_capacity(start_row, batch_size);
    let mut row_index = start_row;
    let mut record = csv::StringRecord::new();

    loop {
        match reader.read_record(&mut record) {
            Ok(true) => {
                let row = record_to_row(&record, row_index, selected_columns, null_values);
                current_batch.push(row);
                row_index += 1;

                // Flush batch when full
                if current_batch.len() >= batch_size {
                    batches.push(current_batch);
                    current_batch = RowBatch::with_capacity(row_index, batch_size);
                }
            }
            Ok(false) => break, // End of chunk
            Err(e) => {
                warn!(error = %e, row = row_index, "CSV parse error in chunk, skipping row");
                row_index += 1;
                continue;
            }
        }
    }

    // Push any remaining rows
    if !current_batch.is_empty() {
        current_batch.is_last = true;
        batches.push(current_batch);
    }

    batches
}

/// Convert bytes from the configured encoding to UTF-8.
///
/// Most files are already UTF-8, in which case this is a no-op.
/// For other encodings, we convert using the `encoding_rs` crate.
fn convert_encoding(data: &[u8], encoding: Encoding) -> Result<std::borrow::Cow<'_, [u8]>> {
    match encoding {
        Encoding::Utf8 | Encoding::Auto => {
            // For Auto, assume UTF-8 (most common case)
            Ok(std::borrow::Cow::Borrowed(data))
        }
        Encoding::Latin1 => {
            let (cow, _, had_errors) = encoding_rs::WINDOWS_1252.decode(data);
            if had_errors {
                warn!("Encoding conversion had errors (Latin-1)");
            }
            Ok(std::borrow::Cow::Owned(cow.as_bytes().to_vec()))
        }
        Encoding::Windows1252 => {
            let (cow, _, had_errors) = encoding_rs::WINDOWS_1252.decode(data);
            if had_errors {
                warn!("Encoding conversion had errors (Windows-1252)");
            }
            Ok(std::borrow::Cow::Owned(cow.as_bytes().to_vec()))
        }
        Encoding::Utf16Le => {
            let (cow, _, had_errors) = encoding_rs::UTF_16LE.decode(data);
            if had_errors {
                warn!("Encoding conversion had errors (UTF-16LE)");
            }
            Ok(std::borrow::Cow::Owned(cow.as_bytes().to_vec()))
        }
        Encoding::Utf16Be => {
            let (cow, _, had_errors) = encoding_rs::UTF_16BE.decode(data);
            if had_errors {
                warn!("Encoding conversion had errors (UTF-16BE)");
            }
            Ok(std::borrow::Cow::Owned(cow.as_bytes().to_vec()))
        }
        Encoding::ShiftJis => {
            let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.decode(data);
            if had_errors {
                warn!("Encoding conversion had errors (Shift-JIS)");
            }
            Ok(std::borrow::Cow::Owned(cow.as_bytes().to_vec()))
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cell_value() {
        assert_eq!(parse_cell_value("", &None), CellValue::Null);
        assert_eq!(parse_cell_value("  ", &None), CellValue::Null);
        assert_eq!(parse_cell_value("42", &None), CellValue::Int(42));
        assert_eq!(parse_cell_value("-7", &None), CellValue::Int(-7));
        assert_eq!(parse_cell_value("3.15", &None), CellValue::Float(3.15));
        assert_eq!(parse_cell_value("true", &None), CellValue::Bool(true));
        assert_eq!(parse_cell_value("FALSE", &None), CellValue::Bool(false));
        assert!(matches!(parse_cell_value("hello", &None), CellValue::String(_)));
    }

    #[test]
    fn test_skip_bom() {
        let with_bom = b"\xEF\xBB\xBFhello";
        let (data, offset) = skip_bom(with_bom);
        assert_eq!(data, b"hello");
        assert_eq!(offset, 3);

        let without_bom = b"hello";
        let (data, offset) = skip_bom(without_bom);
        assert_eq!(data, b"hello");
        assert_eq!(offset, 0);
    }

    #[test]
    fn test_split_into_chunks() {
        let row = b"row_data_here_some_longer_fields_to_quickly_fill_the_buffer_size_to_exceed_one_thousand_bytes\n";
        let mut data = Vec::new();
        for _ in 0..20 {
            data.extend_from_slice(row);
        }
        let chunks = split_into_chunks(&data, 2, 0);
        assert_eq!(chunks.len(), 2);

        // Both chunks should cover the entire data
        let total_len: usize = chunks.iter().map(|c| c.length).sum();
        assert_eq!(total_len, data.len());
    }

    #[test]
    fn test_split_single_chunk() {
        let data = b"small";
        let chunks = split_into_chunks(data, 4, 0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].length, 5);
    }

    #[test]
    fn test_split_empty() {
        let data = b"";
        let chunks = split_into_chunks(data, 4, 0);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_parse_header_line() {
        let data = b"name,age,city\nAlice,30,NYC\n";
        let config = ReaderConfig::default();
        let (headers, offset) = parse_header_line(data, &config).unwrap();
        assert_eq!(headers, vec!["name", "age", "city"]);
        assert_eq!(offset, 14); // "name,age,city\n".len()
    }

    #[test]
    fn test_parse_chunk_basic() {
        let data = b"Alice,30,NYC\nBob,25,LA\nCharlie,35,SF\n";
        let batches = parse_chunk(data, 0, 10, b',', b'"', None, false, true, None, &None, &None);

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].rows.len(), 3);
        assert_eq!(batches[0].rows[0].get_str(0), Some("Alice"));
        assert_eq!(batches[0].rows[0].get_int(1), Some(30));
    }

    #[test]
    fn test_record_to_row_selected_columns() {
        let mut record = csv::StringRecord::new();
        record.push_field("Alice");
        record.push_field("30");
        record.push_field("NYC");
        record.push_field("extra");

        let selected = Some(vec![0, 2]); // Only name and city
        let row = record_to_row(&record, 0, &selected, &None);
        assert_eq!(row.len(), 2);
        assert_eq!(row.get_str(0), Some("Alice"));
        assert_eq!(row.get_str(1), Some("NYC"));
    }

    #[test]
    fn test_custom_null_values() {
        let nulls = Some(vec!["N/A".to_string(), "NULL".to_string(), "\\N".to_string()]);
        assert_eq!(parse_cell_value("N/A", &nulls), CellValue::Null);
        assert_eq!(parse_cell_value("NULL", &nulls), CellValue::Null);
        assert_eq!(parse_cell_value("\\N", &nulls), CellValue::Null);
        assert_eq!(parse_cell_value("Normal", &nulls), CellValue::from("Normal"));
    }

    #[test]
    fn test_auto_tune_batch_size() {
        let data = b"name,age,city\nAlice,30,NYC\nBob,25,LA\nCharlie,35,SF\n";
        // Enable auto_tune and set small memory limit to trigger downscale or test scaling logic
        let config = ReaderConfig::default()
            .with_batch_size(100)
            .with_max_memory_mb(1) // 1MB limit
            .with_auto_tune_batch_size(true);

        let mut reader = CsvReader::from_bytes(data.to_vec(), config).unwrap();
        assert_eq!(reader.current_batch_size, 100);

        // Next batch should trigger auto tuning logic
        let batch = reader.next_batch().unwrap().unwrap();
        assert_eq!(batch.len(), 3);
        
        // Since the batch memory size is extremely tiny compared to 1MB (1024*1024 / 50 = ~20KB),
        // it should scale up the batch size!
        assert!(reader.current_batch_size > 100);
    }
}
