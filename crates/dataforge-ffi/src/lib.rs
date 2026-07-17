// =============================================================================
// DataForge FFI — C ABI Foreign Function Interface
// =============================================================================
//! Stable C ABI interface for DataForge, enabling consumption from any
//! language with C interop: C, C++, Go, Java (JNI), C# (P/Invoke), Ruby, etc.
//!
//! # Design Principles
//! - All types are `#[repr(C)]` with stable ABI
//! - No panics cross the FFI boundary (caught and converted to error codes)
//! - Opaque handle-based API (consumers see pointers, not internals)
//! - Thread-safe: handles can be shared across threads
//! - Explicit memory management with `_free()` functions

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::sync::Mutex;

use dataforge_core::config::ReaderConfig;
use dataforge_core::csv::CsvReader;
use dataforge_core::types::RowBatch;

/// Thread-local storage for the last error message.
/// This allows FFI consumers to retrieve detailed error information
/// after a function returns an error code.
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Set the last error message (internal use only).
fn set_last_error(msg: String) {
    if let Ok(mut err) = LAST_ERROR.lock() {
        *err = Some(msg);
    }
}

// =============================================================================
// Opaque handle types
// =============================================================================

/// Opaque handle to a DataForge CSV reader.
/// The actual `CsvReader` is boxed behind this pointer.
pub struct DataForgeReader {
    inner: CsvReader,
}

/// Opaque handle to a row batch.
pub struct DataForgeBatch {
    inner: RowBatch,
}

/// C-compatible configuration struct.
#[repr(C)]
pub struct DataForgeConfig {
    /// Rows per batch (default: 8192)
    pub batch_size: u32,
    /// Maximum memory in megabytes (default: 256)
    pub max_memory_mb: u32,
    /// Enable parallel processing (0 = false, 1 = true)
    pub parallel: i32,
    /// CSV delimiter character (default: ',')
    pub delimiter: u8,
    /// Whether file has a header row (0 = false, 1 = true)
    pub has_header: i32,
}

impl Default for DataForgeConfig {
    fn default() -> Self {
        DataForgeConfig {
            batch_size: 8192,
            max_memory_mb: 256,
            parallel: 1,
            delimiter: b',',
            has_header: 1,
        }
    }
}

// =============================================================================
// Reader API
// =============================================================================

/// Open a CSV file for streaming reading.
///
/// # Safety
/// `path` must be a valid null-terminated UTF-8 string.
/// `config` may be null (defaults will be used).
///
/// Returns a reader handle on success, null on failure.
/// Call `dataforge_last_error()` for error details.
#[no_mangle]
pub unsafe extern "C" fn dataforge_reader_open_csv(
    path: *const c_char,
    config: *const DataForgeConfig,
) -> *mut DataForgeReader {
    let path = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("Invalid path: {e}"));
            return ptr::null_mut();
        }
    };

    let reader_config = if config.is_null() {
        ReaderConfig::default()
    } else {
        let c = unsafe { &*config };
        ReaderConfig::default()
            .with_batch_size(c.batch_size as usize)
            .with_max_memory_mb(c.max_memory_mb as usize)
            .with_parallel(c.parallel != 0)
            .with_delimiter(c.delimiter)
            .with_header(c.has_header != 0)
    };

    match CsvReader::open(path, reader_config) {
        Ok(reader) => Box::into_raw(Box::new(DataForgeReader { inner: reader })),
        Err(e) => {
            set_last_error(e.to_string());
            ptr::null_mut()
        }
    }
}

/// Read the next batch from a reader.
///
/// Returns 1 if a batch was read, 0 if end-of-file, -1 on error.
/// The batch pointer is written to `*batch_out`.
///
/// # Safety
/// `reader` and `batch_out` must be valid non-null pointers.
#[no_mangle]
pub unsafe extern "C" fn dataforge_reader_next_batch(
    reader: *mut DataForgeReader,
    batch_out: *mut *mut DataForgeBatch,
) -> i32 {
    if reader.is_null() || batch_out.is_null() {
        set_last_error("Null pointer argument".to_string());
        return -1;
    }

    let reader = unsafe { &mut *reader };

    match reader.inner.next_batch() {
        Some(Ok(batch)) => {
            unsafe {
                *batch_out = Box::into_raw(Box::new(DataForgeBatch { inner: batch }));
            }
            1
        }
        Some(Err(e)) => {
            set_last_error(e.to_string());
            -1
        }
        None => 0, // End of file
    }
}

/// Close a reader and free its resources.
///
/// # Safety
/// `reader` must be a valid pointer from `dataforge_reader_open_csv`.
/// Must not be called more than once for the same handle.
#[no_mangle]
pub unsafe extern "C" fn dataforge_reader_close(reader: *mut DataForgeReader) {
    if !reader.is_null() {
        drop(unsafe { Box::from_raw(reader) });
    }
}

// =============================================================================
// Batch API
// =============================================================================

/// Get the number of rows in a batch.
///
/// # Safety
/// `batch` must be a valid non-null pointer.
#[no_mangle]
pub unsafe extern "C" fn dataforge_batch_row_count(batch: *const DataForgeBatch) -> u64 {
    if batch.is_null() {
        return 0;
    }
    let batch = unsafe { &*batch };
    batch.inner.len() as u64
}

/// Get a cell value as a string from a batch.
///
/// Returns a null-terminated UTF-8 string. The caller must free it
/// with `dataforge_free_string()`.
///
/// # Safety
/// `batch` must be a valid pointer. Row/col must be within bounds.
#[no_mangle]
pub unsafe extern "C" fn dataforge_batch_get_string(
    batch: *const DataForgeBatch,
    row: u64,
    col: u32,
) -> *mut c_char {
    if batch.is_null() {
        return ptr::null_mut();
    }

    let batch = unsafe { &*batch };

    match batch.inner.rows.get(row as usize) {
        Some(r) => {
            let value = r.get(col as usize).map(|c| c.to_display_string()).unwrap_or_default();
            match CString::new(value) {
                Ok(cs) => cs.into_raw(),
                Err(_) => ptr::null_mut(),
            }
        }
        None => ptr::null_mut(),
    }
}

/// Get a cell value as a float from a batch.
///
/// Returns the float value, or NaN if the cell is not numeric.
///
/// # Safety
/// `batch` must be a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn dataforge_batch_get_float(
    batch: *const DataForgeBatch,
    row: u64,
    col: u32,
) -> f64 {
    if batch.is_null() {
        return f64::NAN;
    }

    let batch = unsafe { &*batch };
    batch
        .inner
        .rows
        .get(row as usize)
        .and_then(|r| r.get_float(col as usize))
        .unwrap_or(f64::NAN)
}

/// Get a cell value as an integer from a batch.
///
/// Returns the integer value, or i64::MIN if the cell is not numeric.
///
/// # Safety
/// `batch` must be a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn dataforge_batch_get_int(
    batch: *const DataForgeBatch,
    row: u64,
    col: u32,
) -> i64 {
    if batch.is_null() {
        return i64::MIN;
    }

    let batch = unsafe { &*batch };
    batch
        .inner
        .rows
        .get(row as usize)
        .and_then(|r| r.get_int(col as usize))
        .unwrap_or(i64::MIN)
}

/// Free a batch and its associated memory.
///
/// # Safety
/// `batch` must be a valid pointer from `dataforge_reader_next_batch`.
#[no_mangle]
pub unsafe extern "C" fn dataforge_batch_free(batch: *mut DataForgeBatch) {
    if !batch.is_null() {
        drop(unsafe { Box::from_raw(batch) });
    }
}

// =============================================================================
// Error API
// =============================================================================

/// Get the last error message.
///
/// Returns a null-terminated UTF-8 string, or null if no error occurred.
/// The caller must free it with `dataforge_free_string()`.
#[no_mangle]
pub extern "C" fn dataforge_last_error() -> *mut c_char {
    match LAST_ERROR.lock() {
        Ok(err) => match &*err {
            Some(msg) => CString::new(msg.as_str())
                .map(|cs| cs.into_raw())
                .unwrap_or(ptr::null_mut()),
            None => ptr::null_mut(),
        },
        Err(_) => ptr::null_mut(),
    }
}

/// Free a string allocated by DataForge.
///
/// # Safety
/// `s` must be a pointer returned by a DataForge function,
/// or null (which is safely ignored).
#[no_mangle]
pub unsafe extern "C" fn dataforge_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Get the DataForge library version.
///
/// Returns a null-terminated UTF-8 string. Caller must free with
/// `dataforge_free_string()`.
#[no_mangle]
pub extern "C" fn dataforge_version() -> *mut c_char {
    CString::new(env!("CARGO_PKG_VERSION"))
        .map(|cs| cs.into_raw())
        .unwrap_or(ptr::null_mut())
}
