// =============================================================================
// DataForge Core — Data Types
// =============================================================================
// Core data types used throughout the library for representing spreadsheet data.
//
// Design principles:
// - `CellValue` is the universal cell representation across all formats
// - `Row` uses `SmallVec` to avoid heap allocation for typical spreadsheets (≤32 cols)
// - `CompactString` provides small-string optimization (inline for ≤24 bytes)
// - `RowBatch` groups rows for efficient bulk transfer across language boundaries
// - All types implement `Clone`, `Debug`, and `Serialize` for flexibility
// =============================================================================

use std::fmt;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// The value contained in a single spreadsheet cell.
///
/// This enum represents every possible value type that can appear in
/// CSV, XLSX, or ODS cells. It is the universal interchange type between
/// all DataForge components.
///
/// # Memory Layout
/// - `Null`, `Bool`, `Int`, `Float`: 16 bytes (discriminant + value)
/// - `String`: 24 bytes inline (CompactString SSO), heap-allocated if longer
/// - `DateTime`, `Date`, `Time`: 16-24 bytes
/// - `Bytes`: heap-allocated (for binary/embedded data)
///
/// # Example
/// ```
/// use dataforge_core::types::CellValue;
///
/// let cell = CellValue::from(42_i64);
/// assert!(cell.is_int());
/// assert_eq!(cell.as_int(), Some(42));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CellValue {
    /// Empty/null cell — no data present
    Null,

    /// Boolean value (TRUE/FALSE in spreadsheets)
    Bool(bool),

    /// 64-bit signed integer
    Int(i64),

    /// 64-bit floating point number
    Float(f64),

    /// UTF-8 string with small-string optimization.
    /// Strings ≤ 24 bytes are stored inline without heap allocation.
    String(CompactString),

    /// Date and time combined (e.g., "2024-01-15 14:30:00")
    DateTime(NaiveDateTime),

    /// Date only (e.g., "2024-01-15")
    Date(NaiveDate),

    /// Time only (e.g., "14:30:00")
    Time(NaiveTime),

    /// Duration / time interval in seconds (e.g., elapsed time)
    Duration(f64),

    /// Cell error value (e.g., #VALUE!, #REF!, #DIV/0! in Excel)
    Error(CellError),

    /// Raw binary data (rare, used for embedded objects)
    Bytes(Vec<u8>),
}

/// Excel-compatible cell error types.
///
/// These correspond to the standard error values that Excel/Calc can display
/// in cells. They are preserved during read/write to maintain fidelity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellError {
    /// #NULL! — intersection of two ranges that don't intersect
    Null,
    /// #DIV/0! — division by zero
    DivZero,
    /// #VALUE! — wrong type of operand or argument
    Value,
    /// #REF! — invalid cell reference
    Ref,
    /// #NAME? — unrecognized formula name
    Name,
    /// #NUM! — invalid numeric value
    Num,
    /// #N/A — value not available
    Na,
    /// #GETTING_DATA — data is being fetched (async formulas)
    GettingData,
}

/// Data type classification for schema inference and validation.
///
/// This is a simpler enum than `CellValue` — it describes the *type* of a column
/// rather than a specific *value*. Used in `ColumnSchema` for type enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DataType {
    /// Unknown or not yet inferred
    Unknown,
    /// Null / empty
    Null,
    /// Boolean
    Bool,
    /// Integer (i64)
    Int,
    /// Floating point (f64)
    Float,
    /// UTF-8 string
    String,
    /// Date and time
    DateTime,
    /// Date only
    Date,
    /// Time only
    Time,
    /// Duration / interval
    Duration,
    /// Raw binary data
    Bytes,
}

/// Schema definition for a single column.
///
/// Used for both schema inference (auto-detected) and schema validation
/// (user-provided constraints). Columns can specify type, nullability,
/// and optional constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnSchema {
    /// Column name (header text)
    pub name: String,

    /// Expected data type for values in this column
    pub data_type: DataType,

    /// Whether null/empty values are allowed
    pub nullable: bool,

    /// Maximum string length (if applicable)
    pub max_length: Option<usize>,

    /// Column index (0-based)
    pub index: usize,
}

/// Metadata about a single sheet/worksheet.
///
/// Contains structural information about a sheet without loading any cell data.
/// Useful for discovering available sheets and their dimensions before reading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SheetMetadata {
    /// Sheet name (tab label in the spreadsheet UI)
    pub name: String,

    /// Sheet index (0-based position in the workbook)
    pub index: usize,

    /// Total number of rows, if known before reading.
    /// CSV files may not know this without a full scan. XLSX usually provides it.
    pub row_count: Option<u64>,

    /// Number of columns detected
    pub column_count: usize,

    /// Column schemas (may be empty if headers haven't been read yet)
    pub columns: Vec<ColumnSchema>,

    /// Whether this is the active/default sheet
    pub is_active: bool,
}

/// A single row of spreadsheet data.
///
/// Uses `SmallVec` to store cells inline for rows with ≤32 columns,
/// avoiding heap allocation for the vast majority of real-world spreadsheets.
///
/// # Memory
/// - Stack-allocated for ≤32 columns (most spreadsheets)
/// - Heap-allocated for >32 columns (rare, wide datasets)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    /// 0-based row index in the source file
    pub index: u64,

    /// Cell values. SmallVec stores up to 32 cells on the stack.
    pub cells: SmallVec<[CellValue; 32]>,
}

/// A batch of rows for efficient bulk processing and cross-language transfer.
///
/// Instead of transferring rows one-at-a-time across FFI/napi boundaries
/// (which incurs per-call overhead), we group them into batches.
/// The batch size is configurable via `ReaderConfig::batch_size`.
///
/// # Typical Batch Sizes
/// - 1,024 rows: Low-latency streaming (good for real-time display)
/// - 8,192 rows: Default (good balance of throughput and memory)
/// - 65,536 rows: High-throughput bulk processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RowBatch {
    /// The rows in this batch
    pub rows: Vec<Row>,

    /// 0-based index of the first row in this batch (within the source file)
    pub start_index: u64,

    /// Column headers, if available (shared across all rows)
    pub headers: Option<Vec<String>>,

    /// Whether this is the last batch in the stream
    pub is_last: bool,
}

// =============================================================================
// CellValue — Type checking and conversion methods
// =============================================================================

impl CellValue {
    /// Returns `true` if this cell is null/empty.
    pub fn is_null(&self) -> bool {
        matches!(self, CellValue::Null)
    }

    /// Returns `true` if this cell contains a boolean.
    pub fn is_bool(&self) -> bool {
        matches!(self, CellValue::Bool(_))
    }

    /// Returns `true` if this cell contains an integer.
    pub fn is_int(&self) -> bool {
        matches!(self, CellValue::Int(_))
    }

    /// Returns `true` if this cell contains a float.
    pub fn is_float(&self) -> bool {
        matches!(self, CellValue::Float(_))
    }

    /// Returns `true` if this cell contains a string.
    pub fn is_string(&self) -> bool {
        matches!(self, CellValue::String(_))
    }

    /// Returns `true` if this cell contains a numeric value (int or float).
    pub fn is_numeric(&self) -> bool {
        matches!(self, CellValue::Int(_) | CellValue::Float(_))
    }

    /// Try to extract a boolean value.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            CellValue::Bool(v) => Some(*v),
            _ => None,
        }
    }

    /// Try to extract an integer value.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            CellValue::Int(v) => Some(*v),
            CellValue::Float(v) => Some(*v as i64),
            _ => None,
        }
    }

    /// Try to extract a float value.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            CellValue::Float(v) => Some(*v),
            CellValue::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    /// Try to extract a string reference.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            CellValue::String(v) => Some(v.as_str()),
            _ => None,
        }
    }

    /// Try to extract a DateTime value.
    pub fn as_datetime(&self) -> Option<NaiveDateTime> {
        match self {
            CellValue::DateTime(v) => Some(*v),
            _ => None,
        }
    }

    /// Try to extract a Date value.
    pub fn as_date(&self) -> Option<NaiveDate> {
        match self {
            CellValue::Date(v) => Some(*v),
            CellValue::DateTime(v) => Some(v.date()),
            _ => None,
        }
    }

    /// Try to extract a Time value.
    pub fn as_time(&self) -> Option<NaiveTime> {
        match self {
            CellValue::Time(v) => Some(*v),
            CellValue::DateTime(v) => Some(v.time()),
            _ => None,
        }
    }

    /// Get the `DataType` classification of this cell's value.
    pub fn data_type(&self) -> DataType {
        match self {
            CellValue::Null => DataType::Null,
            CellValue::Bool(_) => DataType::Bool,
            CellValue::Int(_) => DataType::Int,
            CellValue::Float(_) => DataType::Float,
            CellValue::String(_) => DataType::String,
            CellValue::DateTime(_) => DataType::DateTime,
            CellValue::Date(_) => DataType::Date,
            CellValue::Time(_) => DataType::Time,
            CellValue::Duration(_) => DataType::Duration,
            CellValue::Error(_) => DataType::String, // Errors display as strings
            CellValue::Bytes(_) => DataType::Bytes,
        }
    }

    /// Convert this cell value to a display string.
    ///
    /// Unlike `Debug` formatting, this produces human-readable output
    /// suitable for CSV writing or UI display.
    pub fn to_display_string(&self) -> String {
        match self {
            CellValue::Null => String::new(),
            CellValue::Bool(v) => if *v { "TRUE".to_string() } else { "FALSE".to_string() },
            CellValue::Int(v) => v.to_string(),
            CellValue::Float(v) => {
                // Use a reasonable default precision, stripping trailing zeros
                let s = format!("{v:.10}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
            CellValue::String(v) => v.to_string(),
            CellValue::DateTime(v) => v.format("%Y-%m-%d %H:%M:%S").to_string(),
            CellValue::Date(v) => v.format("%Y-%m-%d").to_string(),
            CellValue::Time(v) => v.format("%H:%M:%S").to_string(),
            CellValue::Duration(secs) => format!("{secs}s"),
            CellValue::Error(e) => e.to_string(),
            CellValue::Bytes(v) => format!("<{} bytes>", v.len()),
        }
    }
}

// =============================================================================
// From implementations — convenient construction of CellValue
// =============================================================================

impl From<bool> for CellValue {
    fn from(v: bool) -> Self {
        CellValue::Bool(v)
    }
}

impl From<i64> for CellValue {
    fn from(v: i64) -> Self {
        CellValue::Int(v)
    }
}

impl From<i32> for CellValue {
    fn from(v: i32) -> Self {
        CellValue::Int(v as i64)
    }
}

impl From<f64> for CellValue {
    fn from(v: f64) -> Self {
        CellValue::Float(v)
    }
}

impl From<f32> for CellValue {
    fn from(v: f32) -> Self {
        CellValue::Float(v as f64)
    }
}

impl From<String> for CellValue {
    fn from(v: String) -> Self {
        CellValue::String(CompactString::from(v))
    }
}

impl From<&str> for CellValue {
    fn from(v: &str) -> Self {
        CellValue::String(CompactString::from(v))
    }
}

impl From<CompactString> for CellValue {
    fn from(v: CompactString) -> Self {
        CellValue::String(v)
    }
}

impl From<NaiveDateTime> for CellValue {
    fn from(v: NaiveDateTime) -> Self {
        CellValue::DateTime(v)
    }
}

impl From<NaiveDate> for CellValue {
    fn from(v: NaiveDate) -> Self {
        CellValue::Date(v)
    }
}

impl From<NaiveTime> for CellValue {
    fn from(v: NaiveTime) -> Self {
        CellValue::Time(v)
    }
}

impl<T: Into<CellValue>> From<Option<T>> for CellValue {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(val) => val.into(),
            None => CellValue::Null,
        }
    }
}

// =============================================================================
// Row — construction and access methods
// =============================================================================

impl Row {
    /// Create a new empty row with the given index.
    pub fn new(index: u64) -> Self {
        Row {
            index,
            cells: SmallVec::new(),
        }
    }

    /// Create a new row with pre-allocated capacity.
    pub fn with_capacity(index: u64, capacity: usize) -> Self {
        Row {
            index,
            cells: SmallVec::with_capacity(capacity),
        }
    }

    /// Get the number of cells in this row.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Check if this row has no cells.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Get a cell value by column index (0-based).
    pub fn get(&self, col: usize) -> Option<&CellValue> {
        self.cells.get(col)
    }

    /// Get a mutable reference to a cell value by column index.
    pub fn get_mut(&mut self, col: usize) -> Option<&mut CellValue> {
        self.cells.get_mut(col)
    }

    /// Set a cell value at the given column index.
    /// Extends the row with Null values if the index is beyond current length.
    pub fn set(&mut self, col: usize, value: CellValue) {
        if col >= self.cells.len() {
            self.cells.resize(col + 1, CellValue::Null);
        }
        self.cells[col] = value;
    }

    /// Push a cell value to the end of the row.
    pub fn push(&mut self, value: CellValue) {
        self.cells.push(value);
    }

    /// Get a float value from a column, returning None if not numeric.
    pub fn get_float(&self, col: usize) -> Option<f64> {
        self.cells.get(col).and_then(|c| c.as_float())
    }

    /// Get a string reference from a column, returning None if not a string.
    pub fn get_str(&self, col: usize) -> Option<&str> {
        self.cells.get(col).and_then(|c| c.as_str())
    }

    /// Get an integer value from a column, returning None if not numeric.
    pub fn get_int(&self, col: usize) -> Option<i64> {
        self.cells.get(col).and_then(|c| c.as_int())
    }
}

// =============================================================================
// RowBatch — construction and access methods
// =============================================================================

impl RowBatch {
    /// Create a new empty batch.
    pub fn new(start_index: u64) -> Self {
        RowBatch {
            rows: Vec::new(),
            start_index,
            headers: None,
            is_last: false,
        }
    }

    /// Create a new batch with pre-allocated capacity.
    pub fn with_capacity(start_index: u64, capacity: usize) -> Self {
        RowBatch {
            rows: Vec::with_capacity(capacity),
            start_index,
            headers: None,
            is_last: false,
        }
    }

    /// Get the number of rows in this batch.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Check if this batch is empty.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Push a row into this batch.
    pub fn push(&mut self, row: Row) {
        self.rows.push(row);
    }

    /// Iterate over rows in this batch.
    pub fn iter(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter()
    }

    /// Get approximate memory usage of this batch in bytes.
    ///
    /// This is used by the memory backpressure system to track
    /// how much memory is consumed by in-flight batches.
    pub fn estimated_memory_bytes(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        for row in &self.rows {
            total += std::mem::size_of::<Row>();
            for cell in &row.cells {
                total += match cell {
                    CellValue::String(s) => s.len(),
                    CellValue::Bytes(b) => b.len(),
                    _ => 0,
                };
                total += std::mem::size_of::<CellValue>();
            }
        }
        if let Some(headers) = &self.headers {
            for h in headers {
                total += h.len();
            }
        }
        total
    }
}

// =============================================================================
// Display implementations
// =============================================================================

impl fmt::Display for CellValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_display_string())
    }
}

impl fmt::Display for CellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            CellError::Null => "#NULL!",
            CellError::DivZero => "#DIV/0!",
            CellError::Value => "#VALUE!",
            CellError::Ref => "#REF!",
            CellError::Name => "#NAME?",
            CellError::Num => "#NUM!",
            CellError::Na => "#N/A",
            CellError::GettingData => "#GETTING_DATA",
        };
        write!(f, "{s}")
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            DataType::Unknown => "unknown",
            DataType::Null => "null",
            DataType::Bool => "bool",
            DataType::Int => "int",
            DataType::Float => "float",
            DataType::String => "string",
            DataType::DateTime => "datetime",
            DataType::Date => "date",
            DataType::Time => "time",
            DataType::Duration => "duration",
            DataType::Bytes => "bytes",
        };
        write!(f, "{s}")
    }
}

impl ColumnSchema {
    /// Create a new column schema with the given name and type.
    pub fn new(name: impl Into<String>, data_type: DataType, index: usize) -> Self {
        ColumnSchema {
            name: name.into(),
            data_type,
            nullable: true,
            max_length: None,
            index,
        }
    }

    /// Set whether this column allows null values.
    pub fn with_nullable(mut self, nullable: bool) -> Self {
        self.nullable = nullable;
        self
    }

    /// Set the maximum string length for this column.
    pub fn with_max_length(mut self, max_length: usize) -> Self {
        self.max_length = Some(max_length);
        self
    }
}

/// Column-wise compression (RLE/Dict) to optimize memory footprints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompressedColumn {
    /// Uncompressed column payload.
    Uncompressed(Vec<CellValue>),
    /// Run-length encoded column layout.
    Rle {
        values: Vec<CellValue>,
        lengths: Vec<u32>,
    },
    /// Dictionary-encoded column layout.
    Dictionary {
        keys: Vec<u32>,
        values: Vec<CellValue>,
    },
}

impl CompressedColumn {
    /// Compress a slice of CellValues into an RLE, Dictionary, or Uncompressed variant.
    pub fn compress(values: &[CellValue]) -> Self {
        if values.is_empty() {
            return CompressedColumn::Uncompressed(Vec::new());
        }

        // Try RLE compression
        let mut rle_values = Vec::new();
        let mut rle_lengths = Vec::new();
        let mut last_val = &values[0];
        let mut run_len = 1u32;

        for val in &values[1..] {
            if val == last_val {
                run_len += 1;
            } else {
                rle_values.push(last_val.clone());
                rle_lengths.push(run_len);
                last_val = val;
                run_len = 1;
            }
        }
        rle_values.push(last_val.clone());
        rle_lengths.push(run_len);

        let rle_footprint = rle_values.len() * std::mem::size_of::<CellValue>() + rle_lengths.len() * 4;
        let raw_footprint = values.len() * std::mem::size_of::<CellValue>();

        // Try Dictionary compression
        let mut dict_values = Vec::new();
        let mut dict_keys = Vec::new();
        for val in values {
            if let Some(pos) = dict_values.iter().position(|x| x == val) {
                dict_keys.push(pos as u32);
            } else {
                let pos = dict_values.len();
                dict_values.push(val.clone());
                dict_keys.push(pos as u32);
            }
        }

        let dict_footprint = dict_values.len() * std::mem::size_of::<CellValue>() + dict_keys.len() * 4;

        if rle_footprint < raw_footprint && rle_footprint < dict_footprint {
            CompressedColumn::Rle {
                values: rle_values,
                lengths: rle_lengths,
            }
        } else if dict_footprint < raw_footprint {
            CompressedColumn::Dictionary {
                keys: dict_keys,
                values: dict_values,
            }
        } else {
            CompressedColumn::Uncompressed(values.to_vec())
        }
    }

    /// Decompress a column layout back into a vector of raw CellValues.
    pub fn decompress(&self) -> Vec<CellValue> {
        match self {
            CompressedColumn::Uncompressed(v) => v.clone(),
            CompressedColumn::Rle { values, lengths } => {
                let mut out = Vec::new();
                for (val, &len) in values.iter().zip(lengths.iter()) {
                    for _ in 0..len {
                        out.push(val.clone());
                    }
                }
                out
            }
            CompressedColumn::Dictionary { keys, values } => {
                let mut out = Vec::new();
                for &key in keys {
                    if let Some(val) = values.get(key as usize) {
                        out.push(val.clone());
                    } else {
                        out.push(CellValue::Null);
                    }
                }
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cell_value_from_conversions() {
        assert_eq!(CellValue::from(42_i64), CellValue::Int(42));
        assert_eq!(CellValue::from(3.14_f64), CellValue::Float(3.14));
        assert_eq!(CellValue::from(true), CellValue::Bool(true));
        assert_eq!(
            CellValue::from("hello"),
            CellValue::String(CompactString::from("hello"))
        );
    }

    #[test]
    fn test_cell_value_type_checks() {
        assert!(CellValue::Null.is_null());
        assert!(CellValue::Int(42).is_int());
        assert!(CellValue::Int(42).is_numeric());
        assert!(CellValue::Float(3.14).is_float());
        assert!(CellValue::Float(3.14).is_numeric());
        assert!(CellValue::from("text").is_string());
    }

    #[test]
    fn test_cell_value_extraction() {
        assert_eq!(CellValue::Int(42).as_int(), Some(42));
        assert_eq!(CellValue::Int(42).as_float(), Some(42.0));
        assert_eq!(CellValue::Float(3.14).as_float(), Some(3.14));
        assert_eq!(CellValue::from("hello").as_str(), Some("hello"));
        assert_eq!(CellValue::Null.as_int(), None);
    }

    #[test]
    fn test_row_operations() {
        let mut row = Row::new(0);
        row.push(CellValue::from("Alice"));
        row.push(CellValue::from(30_i64));
        row.push(CellValue::from(75000.50_f64));

        assert_eq!(row.len(), 3);
        assert_eq!(row.get_str(0), Some("Alice"));
        assert_eq!(row.get_int(1), Some(30));
        assert_eq!(row.get_float(2), Some(75000.50));
    }

    #[test]
    fn test_row_set_extends() {
        let mut row = Row::new(0);
        row.set(5, CellValue::from("value"));

        // Should have 6 cells: 5 nulls + the value
        assert_eq!(row.len(), 6);
        assert!(row.get(0).unwrap().is_null());
        assert!(row.get(4).unwrap().is_null());
        assert_eq!(row.get_str(5), Some("value"));
    }

    #[test]
    fn test_batch_memory_estimation() {
        let mut batch = RowBatch::new(0);
        let mut row = Row::new(0);
        row.push(CellValue::from("test string data"));
        row.push(CellValue::from(42_i64));
        batch.push(row);

        let mem = batch.estimated_memory_bytes();
        assert!(mem > 0);
    }

    #[test]
    fn test_display_string() {
        assert_eq!(CellValue::Null.to_display_string(), "");
        assert_eq!(CellValue::Bool(true).to_display_string(), "TRUE");
        assert_eq!(CellValue::Int(42).to_display_string(), "42");
        assert_eq!(CellValue::from("hello").to_display_string(), "hello");
    }

    #[test]
    fn test_cell_error_display() {
        assert_eq!(CellError::DivZero.to_string(), "#DIV/0!");
        assert_eq!(CellError::Na.to_string(), "#N/A");
        assert_eq!(CellError::Ref.to_string(), "#REF!");
    }

    #[test]
    fn test_option_into_cell_value() {
        let some_val: CellValue = Some(42_i64).into();
        let none_val: CellValue = Option::<i64>::None.into();
        assert_eq!(some_val, CellValue::Int(42));
        assert_eq!(none_val, CellValue::Null);
    }

    #[test]
    fn test_column_compression() {
        // Test RLE compression
        let values_rle = vec![
            CellValue::from("status_active"),
            CellValue::from("status_active"),
            CellValue::from("status_active"),
            CellValue::from("status_pending"),
            CellValue::from("status_pending"),
        ];
        let compressed_rle = CompressedColumn::compress(&values_rle);
        assert!(matches!(compressed_rle, CompressedColumn::Rle { .. }));
        assert_eq!(compressed_rle.decompress(), values_rle);

        // Test Dictionary compression
        let values_dict = vec![
            CellValue::from("A"),
            CellValue::from("B"),
            CellValue::from("A"),
            CellValue::from("C"),
            CellValue::from("B"),
            CellValue::from("C"),
        ];
        let compressed_dict = CompressedColumn::compress(&values_dict);
        assert!(matches!(compressed_dict, CompressedColumn::Dictionary { .. }));
        assert_eq!(compressed_dict.decompress(), values_dict);
    }
}
