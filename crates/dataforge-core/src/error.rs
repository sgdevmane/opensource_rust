// =============================================================================
// DataForge Core — Error Types
// =============================================================================
// Unified error hierarchy for all DataForge operations.
//
// Design principles:
// - Every error variant carries enough context to diagnose the issue
// - Errors implement `Display` for human-readable messages
// - Errors implement `std::error::Error` for composability
// - No panics — all failures are represented as `Result<T, DataForgeError>`
// - Error codes are stable for FFI consumers
// =============================================================================

use std::fmt;

use thiserror::Error;

/// Primary error type for all DataForge operations.
///
/// This enum covers every category of failure that can occur during
/// reading, writing, transforming, or converting spreadsheet data.
/// Each variant includes contextual information to help diagnose the problem.
#[derive(Error, Debug)]
pub enum DataForgeError {
    /// File system I/O failure (file not found, permission denied, disk full, etc.)
    #[error("I/O error: {message}")]
    Io {
        message: String,
        #[source]
        source: std::io::Error,
    },

    /// CSV parsing failure (malformed quoting, invalid encoding, etc.)
    #[error("CSV parse error at row {row}, column {column}: {message}")]
    CsvParse {
        row: u64,
        column: u32,
        message: String,
    },

    /// XLSX parsing failure (corrupt XML, missing required elements, etc.)
    #[error("XLSX parse error in {component}: {message}")]
    XlsxParse { component: String, message: String },

    /// ODS parsing failure (corrupt XML, unsupported features, etc.)
    #[error("ODS parse error in {component}: {message}")]
    OdsParse { component: String, message: String },

    /// ZIP archive error (corrupt archive, missing entries, etc.)
    #[error("ZIP error: {message}")]
    Zip { message: String },

    /// Schema validation failure (wrong type, missing required column, etc.)
    #[error("Schema error at row {row}, column '{column}': {message}")]
    Schema {
        row: u64,
        column: String,
        message: String,
    },

    /// Memory limit exceeded — backpressure mechanism triggered
    #[error("Memory limit exceeded: using {current_bytes} bytes, limit is {limit_bytes} bytes")]
    MemoryLimitExceeded { current_bytes: usize, limit_bytes: usize },

    /// Transform pipeline error (filter/map/aggregate failure)
    #[error("Transform error at stage '{stage}': {message}")]
    Transform { stage: String, message: String },

    /// Configuration error (invalid settings, conflicting options)
    #[error("Configuration error: {message}")]
    Config { message: String },

    /// Encoding error (unsupported or invalid character encoding)
    #[error("Encoding error: {message}")]
    Encoding { message: String },

    /// Column not found (referenced column doesn't exist)
    #[error("Column '{name}' not found (available: {available})")]
    ColumnNotFound { name: String, available: String },

    /// Type conversion error (cannot convert cell value to requested type)
    #[error("Type error at row {row}, column {column}: cannot convert {from_type} to {to_type}")]
    TypeConversion {
        row: u64,
        column: u32,
        from_type: String,
        to_type: String,
    },

    /// Sheet not found in workbook
    #[error("Sheet '{name}' not found in workbook (available: {available})")]
    SheetNotFound { name: String, available: String },

    /// Unsupported format or feature
    #[error("Unsupported: {message}")]
    Unsupported { message: String },

    /// Internal error (should not happen — indicates a bug)
    #[error("Internal error: {message}")]
    Internal { message: String },
}

/// Numeric error codes for FFI consumers who can't use Rust enums.
///
/// Each code corresponds to a `DataForgeError` variant.
/// Codes are grouped by category:
/// - 1xxx: I/O errors
/// - 2xxx: Parse errors
/// - 3xxx: Schema/type errors
/// - 4xxx: Resource errors
/// - 5xxx: Configuration errors
/// - 9xxx: Internal errors
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// No error — operation succeeded
    Ok = 0,

    // --- I/O errors (1xxx) ---
    IoError = 1000,
    FileNotFound = 1001,
    PermissionDenied = 1002,

    // --- Parse errors (2xxx) ---
    CsvParseError = 2000,
    XlsxParseError = 2001,
    OdsParseError = 2002,
    ZipError = 2003,
    EncodingError = 2004,

    // --- Schema/type errors (3xxx) ---
    SchemaError = 3000,
    ColumnNotFound = 3001,
    TypeConversion = 3002,
    SheetNotFound = 3003,

    // --- Resource errors (4xxx) ---
    MemoryLimitExceeded = 4000,

    // --- Configuration errors (5xxx) ---
    ConfigError = 5000,
    Unsupported = 5001,

    // --- Transform errors (6xxx) ---
    TransformError = 6000,

    // --- Internal errors (9xxx) ---
    InternalError = 9000,
}

impl DataForgeError {
    /// Convert this error to a numeric error code for FFI consumers.
    ///
    /// This provides a stable numeric API that C, Go, Java, etc. can match on
    /// without needing to parse error message strings.
    pub fn error_code(&self) -> ErrorCode {
        match self {
            DataForgeError::Io { .. } => ErrorCode::IoError,
            DataForgeError::CsvParse { .. } => ErrorCode::CsvParseError,
            DataForgeError::XlsxParse { .. } => ErrorCode::XlsxParseError,
            DataForgeError::OdsParse { .. } => ErrorCode::OdsParseError,
            DataForgeError::Zip { .. } => ErrorCode::ZipError,
            DataForgeError::Schema { .. } => ErrorCode::SchemaError,
            DataForgeError::MemoryLimitExceeded { .. } => ErrorCode::MemoryLimitExceeded,
            DataForgeError::Transform { .. } => ErrorCode::TransformError,
            DataForgeError::Config { .. } => ErrorCode::ConfigError,
            DataForgeError::Encoding { .. } => ErrorCode::EncodingError,
            DataForgeError::ColumnNotFound { .. } => ErrorCode::ColumnNotFound,
            DataForgeError::TypeConversion { .. } => ErrorCode::TypeConversion,
            DataForgeError::SheetNotFound { .. } => ErrorCode::SheetNotFound,
            DataForgeError::Unsupported { .. } => ErrorCode::Unsupported,
            DataForgeError::Internal { .. } => ErrorCode::InternalError,
        }
    }

    /// Create an I/O error with contextual message.
    pub fn io(source: std::io::Error, message: impl Into<String>) -> Self {
        DataForgeError::Io {
            message: message.into(),
            source,
        }
    }

    /// Create a configuration error.
    pub fn config(message: impl Into<String>) -> Self {
        DataForgeError::Config {
            message: message.into(),
        }
    }

    /// Create an internal error (indicates a bug).
    pub fn internal(message: impl Into<String>) -> Self {
        DataForgeError::Internal {
            message: message.into(),
        }
    }
}

/// Convenience type alias for DataForge results.
pub type Result<T> = std::result::Result<T, DataForgeError>;

// ---------------------------------------------------------------------------
// Conversions from standard library and third-party error types
// ---------------------------------------------------------------------------

impl From<std::io::Error> for DataForgeError {
    fn from(err: std::io::Error) -> Self {
        DataForgeError::Io {
            message: err.to_string(),
            source: err,
        }
    }
}

impl From<csv::Error> for DataForgeError {
    fn from(err: csv::Error) -> Self {
        // Extract position info if available from the CSV error
        let (row, col) = match err.position() {
            Some(pos) => (pos.line(), 0),
            None => (0, 0),
        };
        DataForgeError::CsvParse {
            row,
            column: col,
            message: err.to_string(),
        }
    }
}

impl From<quick_xml::Error> for DataForgeError {
    fn from(err: quick_xml::Error) -> Self {
        DataForgeError::XlsxParse {
            component: "xml".to_string(),
            message: err.to_string(),
        }
    }
}

impl From<zip::result::ZipError> for DataForgeError {
    fn from(err: zip::result::ZipError) -> Self {
        DataForgeError::Zip {
            message: err.to_string(),
        }
    }
}

impl From<quick_xml::events::attributes::AttrError> for DataForgeError {
    fn from(err: quick_xml::events::attributes::AttrError) -> Self {
        DataForgeError::XlsxParse {
            component: "xml_attribute".to_string(),
            message: err.to_string(),
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}({})", self, *self as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = DataForgeError::CsvParse {
            row: 42,
            column: 3,
            message: "unexpected quote".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "CSV parse error at row 42, column 3: unexpected quote"
        );
    }

    #[test]
    fn test_error_codes() {
        let err = DataForgeError::MemoryLimitExceeded {
            current_bytes: 512_000_000,
            limit_bytes: 256_000_000,
        };
        assert_eq!(err.error_code(), ErrorCode::MemoryLimitExceeded);
        assert_eq!(err.error_code() as i32, 4000);
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let df_err: DataForgeError = io_err.into();
        assert_eq!(df_err.error_code(), ErrorCode::IoError);
    }

    #[test]
    fn test_config_error() {
        let err = DataForgeError::config("batch_size must be > 0");
        assert!(err.to_string().contains("batch_size must be > 0"));
    }
}
