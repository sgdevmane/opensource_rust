// =============================================================================
// DataForge Core — Configuration
// =============================================================================
// Builder-pattern configuration types for readers, writers, and transforms.
//
// Design principles:
// - All configs have sensible defaults (usable without any customization)
// - Builder pattern with method chaining for ergonomic construction
// - Configs are `Clone` + `Serialize` for persistence/debugging
// - Invalid configurations are caught at build time, not at runtime
// =============================================================================

use serde::{Deserialize, Serialize};

use crate::types::ColumnSchema;

/// Character encoding for text-based formats (CSV, TSV).
///
/// Most modern files use UTF-8, but legacy systems may produce
/// Latin-1, Shift-JIS, or other encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encoding {
    /// UTF-8 (default, most common)
    Utf8,
    /// Latin-1 / ISO-8859-1 (common in legacy Windows/European systems)
    Latin1,
    /// UTF-16 Little Endian
    Utf16Le,
    /// UTF-16 Big Endian
    Utf16Be,
    /// Windows-1252 (superset of Latin-1, common in Excel exports)
    Windows1252,
    /// Shift-JIS (Japanese)
    ShiftJis,
    /// Auto-detect encoding from BOM or content analysis
    Auto,
}

impl Default for Encoding {
    fn default() -> Self {
        Encoding::Utf8
    }
}

/// CSV-specific configuration options.
///
/// Controls how CSV files are parsed and written, including
/// delimiter, quoting, escape characters, and comment handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvConfig {
    /// Field delimiter character (default: `,`)
    pub delimiter: u8,

    /// Quote character for enclosing fields with special chars (default: `"`)
    pub quote_char: u8,

    /// Escape character within quoted fields (default: `"` — RFC 4180 style)
    pub escape_char: Option<u8>,

    /// Whether the first row is a header row (default: `true`)
    pub has_header: bool,

    /// Comment character — lines starting with this are skipped (default: `None`)
    pub comment_char: Option<u8>,

    /// Whether to trim whitespace from field values (default: `false`)
    pub trim_fields: bool,

    /// Line terminator style for writing (default: system-dependent)
    pub line_terminator: LineTerminator,

    /// Whether to allow variable-length rows (default: `true`)
    /// When false, all rows must have the same number of fields as the header.
    pub flexible: bool,
}

impl Default for CsvConfig {
    fn default() -> Self {
        CsvConfig {
            delimiter: b',',
            quote_char: b'"',
            escape_char: None, // RFC 4180: use double-quote as escape
            has_header: true,
            comment_char: None,
            trim_fields: false,
            line_terminator: LineTerminator::default(),
            flexible: true,
        }
    }
}

/// Line terminator style for CSV writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineTerminator {
    /// CRLF (`\r\n`) — Windows standard, RFC 4180 compliant
    Crlf,
    /// LF (`\n`) — Unix/macOS standard
    Lf,
    /// CR (`\r`) — legacy Mac OS (pre-OS X)
    Cr,
}

impl Default for LineTerminator {
    fn default() -> Self {
        if cfg!(windows) {
            LineTerminator::Crlf
        } else {
            LineTerminator::Lf
        }
    }
}

/// XLSX-specific configuration options.
///
/// Controls how Excel files are read and written, including
/// sheet selection, date format interpretation, and formula handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XlsxConfig {
    /// Which sheet to read (default: first sheet).
    /// Can specify by name or by 0-based index.
    pub sheet_selector: SheetSelector,

    /// Whether to evaluate formulas or return the cached value (default: use cached)
    /// Note: DataForge does not have a formula engine — it reads cached values.
    pub use_cached_formula_values: bool,

    /// Date system: 1900-based (Excel default) or 1904-based (Mac Excel legacy)
    pub date_system: DateSystem,

    /// Custom date format patterns for parsing date strings
    pub date_formats: Vec<String>,

    /// Whether to read cell styles/formatting (default: false for speed)
    pub read_styles: bool,

    /// Whether to include empty rows in output (default: false)
    pub include_empty_rows: bool,
}

impl Default for XlsxConfig {
    fn default() -> Self {
        XlsxConfig {
            sheet_selector: SheetSelector::First,
            use_cached_formula_values: true,
            date_system: DateSystem::Base1900,
            date_formats: Vec::new(),
            read_styles: false,
            include_empty_rows: false,
        }
    }
}

/// ODS-specific configuration options.
///
/// Controls how OpenDocument Spreadsheet files are read and written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OdsConfig {
    /// Which sheet to read (default: first sheet)
    pub sheet_selector: SheetSelector,

    /// Whether to include empty rows in output (default: false)
    pub include_empty_rows: bool,
}

impl Default for OdsConfig {
    fn default() -> Self {
        OdsConfig {
            sheet_selector: SheetSelector::First,
            include_empty_rows: false,
        }
    }
}

/// Selects which sheet to read from a multi-sheet workbook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SheetSelector {
    /// Read the first sheet (default)
    First,
    /// Read the sheet with the given name
    ByName(String),
    /// Read the sheet at the given 0-based index
    ByIndex(usize),
    /// Read all sheets (produces metadata for each, reads sequentially)
    All,
}

/// Excel date system — determines how serial date numbers are interpreted.
///
/// Excel stores dates as floating-point numbers representing days since
/// a base date. The base date differs between systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DateSystem {
    /// January 1, 1900 (Windows Excel default)
    /// Note: Excel incorrectly considers 1900 a leap year (Lotus 1-2-3 bug)
    Base1900,
    /// January 1, 1904 (legacy Mac Excel)
    Base1904,
}

impl Default for DateSystem {
    fn default() -> Self {
        DateSystem::Base1900
    }
}

/// Memory backpressure policy — what to do when memory limit is reached.
///
/// When the in-flight data (buffered batches, pending transforms) exceeds
/// the configured memory limit, the backpressure policy determines behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackpressurePolicy {
    /// Block the producer thread until memory is freed (default).
    /// This is the safest option — no data loss, but may reduce throughput.
    Block,

    /// Return an error immediately.
    /// The caller must handle the error and retry or abort.
    Error,

    /// Drop the oldest unprocessed batch to make room.
    /// WARNING: This causes data loss. Only use for non-critical sampling.
    DropOldest,
}

impl Default for BackpressurePolicy {
    fn default() -> Self {
        BackpressurePolicy::Block
    }
}

/// Main configuration for reading spreadsheet data.
///
/// This struct controls all aspects of how data is read, parsed,
/// and delivered. Use the builder methods for ergonomic construction.
///
/// # Example
/// ```
/// use dataforge_core::config::ReaderConfig;
///
/// let config = ReaderConfig::default()
///     .with_batch_size(4096)
///     .with_max_memory_mb(128)
///     .with_parallel(true)
///     .with_skip_rows(1);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReaderConfig {
    /// Number of rows per batch (default: 8192).
    /// Larger batches = higher throughput, more memory.
    /// Smaller batches = lower latency, less memory.
    pub batch_size: usize,

    /// Maximum memory usage in bytes (default: 256 MB).
    /// When exceeded, the backpressure policy is applied.
    pub max_memory_bytes: usize,

    /// Whether to enable parallel chunk processing (default: true for CSV).
    /// XLSX/ODS always use sequential parsing due to XML structure.
    pub parallel: bool,

    /// Number of worker threads for parallel processing.
    /// `None` = use all available CPU cores (default).
    pub num_threads: Option<usize>,

    /// Skip this many rows from the beginning of the file (default: 0).
    /// Useful for files with metadata rows before the header.
    pub skip_rows: u64,

    /// Read at most this many data rows (default: `None` = read all).
    /// Does not count header or skipped rows.
    pub max_rows: Option<u64>,

    /// Read only these column indices (0-based).
    /// `None` = read all columns (default).
    /// This is applied at parse time to avoid allocating unused columns.
    pub columns: Option<Vec<usize>>,

    /// Column name filter — read only columns with these names.
    /// Applied after header detection. Mutually exclusive with `columns`.
    pub column_names: Option<Vec<String>>,

    /// Enforce this schema during reading. Values that don't match
    /// will be coerced or produce errors depending on `strict_schema`.
    pub schema: Option<Vec<ColumnSchema>>,

    /// Whether schema mismatches cause errors (true) or best-effort coercion (false).
    pub strict_schema: bool,

    /// Character encoding for text files (default: UTF-8).
    pub encoding: Encoding,

    /// Memory backpressure policy (default: Block).
    pub backpressure: BackpressurePolicy,

    /// CSV-specific options
    pub csv: CsvConfig,

    /// XLSX-specific options
    pub xlsx: XlsxConfig,

    /// ODS-specific options
    pub ods: OdsConfig,

    /// Number of rows to sample for schema inference (default: 1000).
    /// Set to 0 to skip inference.
    pub inference_sample_size: usize,
}

impl Default for ReaderConfig {
    fn default() -> Self {
        ReaderConfig {
            batch_size: 8192,
            max_memory_bytes: 256 * 1024 * 1024, // 256 MB
            parallel: true,
            num_threads: None,
            skip_rows: 0,
            max_rows: None,
            columns: None,
            column_names: None,
            schema: None,
            strict_schema: false,
            encoding: Encoding::default(),
            backpressure: BackpressurePolicy::default(),
            csv: CsvConfig::default(),
            xlsx: XlsxConfig::default(),
            ods: OdsConfig::default(),
            inference_sample_size: 1000,
        }
    }
}

// =============================================================================
// ReaderConfig — Builder methods
// =============================================================================

impl ReaderConfig {
    /// Set the batch size (rows per batch).
    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = size.max(1); // Minimum 1 row per batch
        self
    }

    /// Set the maximum memory usage in megabytes.
    pub fn with_max_memory_mb(mut self, mb: usize) -> Self {
        self.max_memory_bytes = mb * 1024 * 1024;
        self
    }

    /// Set the maximum memory usage in bytes.
    pub fn with_max_memory_bytes(mut self, bytes: usize) -> Self {
        self.max_memory_bytes = bytes;
        self
    }

    /// Enable or disable parallel processing.
    pub fn with_parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// Set the number of worker threads.
    pub fn with_num_threads(mut self, n: usize) -> Self {
        self.num_threads = Some(n.max(1));
        self
    }

    /// Skip N rows from the beginning of the file.
    pub fn with_skip_rows(mut self, n: u64) -> Self {
        self.skip_rows = n;
        self
    }

    /// Read at most N data rows.
    pub fn with_max_rows(mut self, n: u64) -> Self {
        self.max_rows = Some(n);
        self
    }

    /// Read only specific columns by index (0-based).
    pub fn with_columns(mut self, cols: Vec<usize>) -> Self {
        self.columns = Some(cols);
        self.column_names = None; // Mutually exclusive
        self
    }

    /// Read only specific columns by name.
    pub fn with_column_names(mut self, names: Vec<String>) -> Self {
        self.column_names = Some(names);
        self.columns = None; // Mutually exclusive
        self
    }

    /// Set the character encoding.
    pub fn with_encoding(mut self, encoding: Encoding) -> Self {
        self.encoding = encoding;
        self
    }

    /// Set the CSV delimiter character.
    pub fn with_delimiter(mut self, delim: u8) -> Self {
        self.csv.delimiter = delim;
        self
    }

    /// Set whether the file has a header row.
    pub fn with_header(mut self, has_header: bool) -> Self {
        self.csv.has_header = has_header;
        self
    }

    /// Set the backpressure policy.
    pub fn with_backpressure(mut self, policy: BackpressurePolicy) -> Self {
        self.backpressure = policy;
        self
    }

    /// Set the sheet to read by name.
    pub fn with_sheet_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.xlsx.sheet_selector = SheetSelector::ByName(name.clone());
        self.ods.sheet_selector = SheetSelector::ByName(name);
        self
    }

    /// Set the sheet to read by index (0-based).
    pub fn with_sheet_index(mut self, index: usize) -> Self {
        self.xlsx.sheet_selector = SheetSelector::ByIndex(index);
        self.ods.sheet_selector = SheetSelector::ByIndex(index);
        self
    }

    /// Enforce a schema during reading.
    pub fn with_schema(mut self, schema: Vec<ColumnSchema>) -> Self {
        self.schema = Some(schema);
        self
    }

    /// Enable strict schema enforcement.
    pub fn with_strict_schema(mut self, strict: bool) -> Self {
        self.strict_schema = strict;
        self
    }

    /// Set the number of rows to sample for schema inference.
    pub fn with_inference_sample_size(mut self, n: usize) -> Self {
        self.inference_sample_size = n;
        self
    }

    /// Validate this configuration, returning an error if any settings are invalid.
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.batch_size == 0 {
            return Err(crate::error::DataForgeError::config(
                "batch_size must be greater than 0",
            ));
        }
        if self.max_memory_bytes < 1024 * 1024 {
            return Err(crate::error::DataForgeError::config(
                "max_memory_bytes must be at least 1 MB",
            ));
        }
        if self.columns.is_some() && self.column_names.is_some() {
            return Err(crate::error::DataForgeError::config(
                "cannot specify both column indices and column names",
            ));
        }
        Ok(())
    }
}

/// Configuration for writing spreadsheet data.
///
/// Controls output format, buffering, and formatting options.
///
/// # Example
/// ```
/// use dataforge_core::config::WriterConfig;
///
/// let config = WriterConfig::default()
///     .with_buffer_size(16384);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriterConfig {
    /// Number of rows to buffer before flushing to disk (default: 8192).
    pub buffer_size: usize,

    /// CSV-specific write options
    pub csv: CsvConfig,

    /// XLSX-specific write options
    pub xlsx: XlsxWriteConfig,

    /// ODS-specific write options
    pub ods: OdsWriteConfig,

    /// Column headers to write (if not provided, no header row is written)
    pub headers: Option<Vec<String>>,

    /// Whether to auto-detect optimal column widths (XLSX/ODS only, default: true)
    pub auto_column_width: bool,
}

impl Default for WriterConfig {
    fn default() -> Self {
        WriterConfig {
            buffer_size: 8192,
            csv: CsvConfig::default(),
            xlsx: XlsxWriteConfig::default(),
            ods: OdsWriteConfig::default(),
            headers: None,
            auto_column_width: true,
        }
    }
}

impl WriterConfig {
    /// Set the write buffer size.
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size.max(1);
        self
    }

    /// Set column headers.
    pub fn with_headers(mut self, headers: Vec<String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Set the CSV delimiter.
    pub fn with_delimiter(mut self, delim: u8) -> Self {
        self.csv.delimiter = delim;
        self
    }

    /// Enable or disable auto column width detection.
    pub fn with_auto_column_width(mut self, enabled: bool) -> Self {
        self.auto_column_width = enabled;
        self
    }
}

/// XLSX-specific write configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XlsxWriteConfig {
    /// Sheet name (default: "Sheet1")
    pub sheet_name: String,

    /// Whether to freeze the header row (default: true)
    pub freeze_header: bool,

    /// Whether to enable auto-filter on the header row (default: false)
    pub auto_filter: bool,

    /// Default number format for numeric cells (Excel format string)
    pub number_format: Option<String>,

    /// Default date format for date cells (Excel format string)
    pub date_format: String,
}

impl Default for XlsxWriteConfig {
    fn default() -> Self {
        XlsxWriteConfig {
            sheet_name: "Sheet1".to_string(),
            freeze_header: true,
            auto_filter: false,
            number_format: None,
            date_format: "yyyy-mm-dd".to_string(),
        }
    }
}

/// ODS-specific write configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OdsWriteConfig {
    /// Sheet name (default: "Sheet1")
    pub sheet_name: String,
}

impl Default for OdsWriteConfig {
    fn default() -> Self {
        OdsWriteConfig {
            sheet_name: "Sheet1".to_string(),
        }
    }
}

/// Configuration for format conversion (e.g., CSV→XLSX, XLSX→CSV).
///
/// Combines reader and writer configs with conversion-specific options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertConfig {
    /// Reader configuration for the source file
    pub reader: ReaderConfig,

    /// Writer configuration for the output file
    pub writer: WriterConfig,

    /// Whether to auto-detect source format from file extension (default: true)
    pub auto_detect_format: bool,
}

impl Default for ConvertConfig {
    fn default() -> Self {
        ConvertConfig {
            reader: ReaderConfig::default(),
            writer: WriterConfig::default(),
            auto_detect_format: true,
        }
    }
}

/// Detected file format based on extension or content inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileFormat {
    /// Comma-separated values
    Csv,
    /// Tab-separated values
    Tsv,
    /// Microsoft Excel 2007+ (Open XML)
    Xlsx,
    /// OpenDocument Spreadsheet
    Ods,
}

impl FileFormat {
    /// Detect format from a file path based on its extension.
    ///
    /// Returns `None` if the extension is not recognized.
    pub fn from_path(path: &str) -> Option<Self> {
        let lower = path.to_lowercase();
        if lower.ends_with(".csv") {
            Some(FileFormat::Csv)
        } else if lower.ends_with(".tsv") || lower.ends_with(".tab") {
            Some(FileFormat::Tsv)
        } else if lower.ends_with(".xlsx") {
            Some(FileFormat::Xlsx)
        } else if lower.ends_with(".ods") {
            Some(FileFormat::Ods)
        } else {
            None
        }
    }

    /// Get the typical file extension for this format (without dot).
    pub fn extension(&self) -> &str {
        match self {
            FileFormat::Csv => "csv",
            FileFormat::Tsv => "tsv",
            FileFormat::Xlsx => "xlsx",
            FileFormat::Ods => "ods",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_reader_config() {
        let config = ReaderConfig::default();
        assert_eq!(config.batch_size, 8192);
        assert_eq!(config.max_memory_bytes, 256 * 1024 * 1024);
        assert!(config.parallel);
        assert!(config.csv.has_header);
        assert_eq!(config.csv.delimiter, b',');
    }

    #[test]
    fn test_reader_config_builder() {
        let config = ReaderConfig::default()
            .with_batch_size(4096)
            .with_max_memory_mb(128)
            .with_parallel(false)
            .with_delimiter(b'\t');

        assert_eq!(config.batch_size, 4096);
        assert_eq!(config.max_memory_bytes, 128 * 1024 * 1024);
        assert!(!config.parallel);
        assert_eq!(config.csv.delimiter, b'\t');
    }

    #[test]
    fn test_config_validation() {
        let mut config = ReaderConfig::default();
        config.batch_size = 0;
        assert!(config.validate().is_err());

        let config = ReaderConfig::default().with_max_memory_bytes(100);
        assert!(config.validate().is_err());

        let config = ReaderConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_file_format_detection() {
        assert_eq!(FileFormat::from_path("data.csv"), Some(FileFormat::Csv));
        assert_eq!(FileFormat::from_path("data.CSV"), Some(FileFormat::Csv));
        assert_eq!(FileFormat::from_path("data.xlsx"), Some(FileFormat::Xlsx));
        assert_eq!(FileFormat::from_path("data.ods"), Some(FileFormat::Ods));
        assert_eq!(FileFormat::from_path("data.tsv"), Some(FileFormat::Tsv));
        assert_eq!(FileFormat::from_path("data.txt"), None);
    }

    #[test]
    fn test_columns_mutual_exclusion() {
        let config = ReaderConfig::default()
            .with_columns(vec![0, 1, 2])
            .with_column_names(vec!["a".into(), "b".into()]);

        // column_names should have cleared columns
        assert!(config.columns.is_none());
        assert!(config.column_names.is_some());
    }
}
