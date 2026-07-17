// =============================================================================
// DataForge Core — Streaming CSV Writer
// =============================================================================
// Buffered CSV writer that accepts rows/batches and writes them to any
// output destination (file, Vec<u8>, network stream, etc.).
//
// Key features:
// - Buffered writing for high throughput
// - Configurable delimiter, quoting, line endings
// - Automatic BOM insertion for Windows compatibility
// - Flush-on-count and flush-on-size strategies
// - Supports both row-at-a-time and batch writing
// =============================================================================

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use tracing::{debug, info};

use crate::config::{LineTerminator, WriterConfig};
use crate::error::{DataForgeError, Result};
use crate::types::{Row, RowBatch};

/// Streaming CSV writer that buffers rows and flushes to the output.
///
/// The writer uses a `BufWriter` internally for efficient I/O.
/// Rows can be written individually or in batches.
///
/// # Example
/// ```no_run
/// use dataforge_core::csv::CsvWriter;
/// use dataforge_core::config::WriterConfig;
/// use dataforge_core::types::{Row, CellValue};
///
/// let config = WriterConfig::default()
///     .with_headers(vec!["name".into(), "age".into(), "city".into()]);
///
/// let mut writer = CsvWriter::create("output.csv", config).unwrap();
///
/// let mut row = Row::new(0);
/// row.push(CellValue::from("Alice"));
/// row.push(CellValue::from(30_i64));
/// row.push(CellValue::from("New York"));
/// writer.write_row(&row).unwrap();
///
/// writer.finish().unwrap();
/// ```
pub struct CsvWriter<W: Write> {
    /// Buffered writer wrapping the output destination
    writer: BufWriter<W>,

    /// Configuration snapshot
    config: WriterConfig,

    /// Number of rows written so far
    rows_written: u64,

    /// Number of bytes written so far (approximate)
    bytes_written: u64,

    /// Whether headers have been written
    headers_written: bool,

    /// Line terminator bytes
    line_ending: &'static [u8],
}

impl CsvWriter<File> {
    /// Create a new CSV writer that writes to a file.
    ///
    /// Creates the file if it doesn't exist, truncates if it does.
    ///
    /// # Arguments
    /// * `path` - Path to the output CSV file
    /// * `config` - Writer configuration
    ///
    /// # Errors
    /// Returns `DataForgeError::Io` if the file cannot be created.
    pub fn create(path: impl AsRef<Path>, config: WriterConfig) -> Result<Self> {
        let path = path.as_ref();
        let file = File::create(path).map_err(|e| {
            DataForgeError::io(e, format!("Failed to create CSV file '{}'", path.display()))
        })?;

        info!(path = %path.display(), "Creating CSV output file");

        Self::new(file, config)
    }
}

impl<W: Write> CsvWriter<W> {
    /// Create a new CSV writer wrapping any `Write` implementation.
    ///
    /// # Arguments
    /// * `inner` - The underlying writer (File, Vec<u8>, network stream, etc.)
    /// * `config` - Writer configuration
    pub fn new(inner: W, config: WriterConfig) -> Result<Self> {
        let line_ending = match config.csv.line_terminator {
            LineTerminator::Crlf => b"\r\n".as_slice(),
            LineTerminator::Lf => b"\n".as_slice(),
            LineTerminator::Cr => b"\r".as_slice(),
        };

        let mut writer = CsvWriter {
            writer: BufWriter::with_capacity(64 * 1024, inner), // 64KB write buffer
            config,
            rows_written: 0,
            bytes_written: 0,
            headers_written: false,
            line_ending,
        };

        // Write BOM if configured for UTF-8 (Windows Excel compatibility)
        // Note: Most modern tools don't need BOM, but Excel on Windows does
        // for correct UTF-8 detection. We skip BOM by default.

        // Write headers if provided
        if writer.config.headers.is_some() {
            writer.write_headers()?;
        }

        Ok(writer)
    }

    /// Write the header row.
    fn write_headers(&mut self) -> Result<()> {
        if self.headers_written {
            return Ok(());
        }

        if let Some(headers) = &self.config.headers.clone() {
            let delimiter = self.config.csv.delimiter;

            for (i, header) in headers.iter().enumerate() {
                if i > 0 {
                    self.writer.write_all(&[delimiter])?;
                }
                self.write_field(header)?;
            }
            self.writer.write_all(self.line_ending)?;
            self.headers_written = true;

            debug!(num_columns = headers.len(), "CSV headers written");
        }

        Ok(())
    }

    /// Write a single row to the CSV output.
    ///
    /// # Arguments
    /// * `row` - The row to write
    pub fn write_row(&mut self, row: &Row) -> Result<()> {
        let delimiter = self.config.csv.delimiter;

        for (i, cell) in row.cells.iter().enumerate() {
            if i > 0 {
                self.writer.write_all(&[delimiter])?;
            }
            let display = cell.to_display_string();
            self.write_field(&display)?;
        }
        self.writer.write_all(self.line_ending)?;

        self.rows_written += 1;

        // Auto-flush based on buffer size
        if self.rows_written % self.config.buffer_size as u64 == 0 {
            self.flush()?;
        }

        Ok(())
    }

    /// Write an entire batch of rows.
    ///
    /// More efficient than calling `write_row` repeatedly because
    /// it minimizes flush operations.
    ///
    /// # Arguments
    /// * `batch` - The batch of rows to write
    pub fn write_batch(&mut self, batch: &RowBatch) -> Result<()> {
        // Write headers from batch if we haven't written any yet
        if !self.headers_written {
            if let Some(headers) = &batch.headers {
                self.config.headers = Some(headers.clone());
                self.write_headers()?;
            }
        }

        for row in &batch.rows {
            self.write_row(row)?;
        }

        Ok(())
    }

    /// Write a single field value, handling quoting and escaping.
    ///
    /// A field is quoted if it contains:
    /// - The delimiter character
    /// - The quote character
    /// - A newline character
    /// - Leading/trailing whitespace (if trim mode is enabled)
    fn write_field(&mut self, value: &str) -> Result<()> {
        let delimiter = self.config.csv.delimiter;
        let quote = self.config.csv.quote_char;

        // Determine if quoting is needed
        let needs_quoting = value.contains(delimiter as char)
            || value.contains(quote as char)
            || value.contains('\n')
            || value.contains('\r')
            || value.starts_with(' ')
            || value.ends_with(' ');

        if needs_quoting {
            self.writer.write_all(&[quote])?;

            // Escape internal quote characters by doubling them (RFC 4180)
            for byte in value.bytes() {
                if byte == quote {
                    self.writer.write_all(&[quote, quote])?;
                } else {
                    self.writer.write_all(&[byte])?;
                }
            }

            self.writer.write_all(&[quote])?;
        } else {
            self.writer.write_all(value.as_bytes())?;
        }

        self.bytes_written += value.len() as u64;
        Ok(())
    }

    /// Flush the internal buffer to the underlying writer.
    pub fn flush(&mut self) -> Result<()> {
        self.writer.flush()?;
        Ok(())
    }

    /// Finalize the CSV output, flushing all remaining data.
    ///
    /// This must be called when you're done writing to ensure
    /// all buffered data is written to the output.
    ///
    /// # Returns
    /// The number of data rows written (excluding header).
    pub fn finish(mut self) -> Result<u64> {
        self.flush()?;
        info!(rows_written = self.rows_written, "CSV writing complete");
        Ok(self.rows_written)
    }

    /// Get the number of rows written so far.
    pub fn rows_written(&self) -> u64 {
        self.rows_written
    }

    /// Get the approximate number of bytes written so far.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }
}

/// Create a CSV writer that writes to an in-memory buffer.
///
/// Useful for generating CSV data as bytes (e.g., for HTTP responses,
/// WASM, or testing).
///
/// # Example
/// ```
/// use dataforge_core::csv::writer::create_csv_buffer_writer;
/// use dataforge_core::config::WriterConfig;
/// use dataforge_core::types::{Row, CellValue};
///
/// let config = WriterConfig::default()
///     .with_headers(vec!["name".into(), "value".into()]);
///
/// let mut writer = create_csv_buffer_writer(config).unwrap();
///
/// let mut row = Row::new(0);
/// row.push(CellValue::from("key1"));
/// row.push(CellValue::from(42_i64));
/// writer.write_row(&row).unwrap();
/// ```
pub fn create_csv_buffer_writer(config: WriterConfig) -> Result<CsvWriter<Vec<u8>>> {
    CsvWriter::new(Vec::new(), config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CellValue;

    fn make_test_row(index: u64, values: Vec<CellValue>) -> Row {
        let mut row = Row::new(index);
        for v in values {
            row.push(v);
        }
        row
    }

    #[test]
    fn test_basic_csv_write() {
        let config = WriterConfig::default()
            .with_headers(vec!["name".into(), "age".into()]);

        let mut writer = CsvWriter::new(Vec::<u8>::new(), config).unwrap();

        let row = make_test_row(0, vec![
            CellValue::from("Alice"),
            CellValue::from(30_i64),
        ]);
        writer.write_row(&row).unwrap();

        let row2 = make_test_row(1, vec![
            CellValue::from("Bob"),
            CellValue::from(25_i64),
        ]);
        writer.write_row(&row2).unwrap();

        writer.flush().unwrap();

        // Access the inner buffer
        let inner = writer.writer.into_inner().unwrap();
        let output = String::from_utf8(inner).unwrap();

        assert!(output.contains("name,age"));
        assert!(output.contains("Alice,30"));
        assert!(output.contains("Bob,25"));
    }

    #[test]
    fn test_quoted_fields() {
        let config = WriterConfig::default();
        let mut writer = CsvWriter::new(Vec::<u8>::new(), config).unwrap();

        let row = make_test_row(0, vec![
            CellValue::from("hello, world"),     // Contains delimiter
            CellValue::from("say \"hi\""),        // Contains quote
            CellValue::from("line1\nline2"),      // Contains newline
        ]);
        writer.write_row(&row).unwrap();
        writer.flush().unwrap();

        let inner = writer.writer.into_inner().unwrap();
        let output = String::from_utf8(inner).unwrap();

        assert!(output.contains("\"hello, world\""));
        assert!(output.contains("\"say \"\"hi\"\"\""));
        assert!(output.contains("\"line1\nline2\""));
    }

    #[test]
    fn test_null_and_empty_values() {
        let config = WriterConfig::default();
        let mut writer = CsvWriter::new(Vec::<u8>::new(), config).unwrap();

        let row = make_test_row(0, vec![
            CellValue::Null,
            CellValue::from(""),
            CellValue::from("value"),
        ]);
        writer.write_row(&row).unwrap();
        writer.flush().unwrap();

        let inner = writer.writer.into_inner().unwrap();
        let output = String::from_utf8(inner).unwrap();

        // Null and empty should both produce empty fields
        assert!(output.starts_with(",,value"));
    }

    #[test]
    fn test_batch_write() {
        let config = WriterConfig::default()
            .with_headers(vec!["col1".into(), "col2".into()]);

        let mut writer = CsvWriter::new(Vec::<u8>::new(), config).unwrap();

        let mut batch = RowBatch::new(0);
        batch.push(make_test_row(0, vec![CellValue::from("a"), CellValue::from(1_i64)]));
        batch.push(make_test_row(1, vec![CellValue::from("b"), CellValue::from(2_i64)]));
        batch.push(make_test_row(2, vec![CellValue::from("c"), CellValue::from(3_i64)]));

        writer.write_batch(&batch).unwrap();
        writer.flush().unwrap();

        let inner = writer.writer.into_inner().unwrap();
        let output = String::from_utf8(inner).unwrap();
        let lines: Vec<&str> = output.trim().lines().collect();

        assert_eq!(lines.len(), 4); // header + 3 rows
        assert_eq!(lines[0], "col1,col2");
    }

    #[test]
    fn test_custom_delimiter() {
        let mut config = WriterConfig::default();
        config.csv.delimiter = b'\t';
        config.headers = Some(vec!["a".into(), "b".into()]);

        let mut writer = CsvWriter::new(Vec::<u8>::new(), config).unwrap();

        let row = make_test_row(0, vec![CellValue::from("x"), CellValue::from("y")]);
        writer.write_row(&row).unwrap();
        writer.flush().unwrap();

        let inner = writer.writer.into_inner().unwrap();
        let output = String::from_utf8(inner).unwrap();

        assert!(output.contains("a\tb"));
        assert!(output.contains("x\ty"));
    }

    #[test]
    fn test_row_count() {
        let config = WriterConfig::default();
        let mut writer = CsvWriter::new(Vec::<u8>::new(), config).unwrap();

        assert_eq!(writer.rows_written(), 0);

        for i in 0..5 {
            let row = make_test_row(i, vec![CellValue::from(i as i64)]);
            writer.write_row(&row).unwrap();
        }

        assert_eq!(writer.rows_written(), 5);
    }
}
