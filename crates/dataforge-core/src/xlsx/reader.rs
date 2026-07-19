// =============================================================================
// DataForge Core — Streaming XLSX Reader
// =============================================================================
// SAX-style streaming XML parser for Excel 2007+ (.xlsx) files.
//
// The reader opens the ZIP archive, pre-loads the shared strings table
// and styles, then streams the worksheet XML row-by-row, producing
// RowBatch values without ever loading the full worksheet into memory.
//
// Memory usage is O(shared_strings + styles + batch_size), which is
// typically a few MB even for files containing millions of rows.
// =============================================================================

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use compact_str::CompactString;
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use tracing::{debug, info, warn};
use zip::ZipArchive;

use super::shared_strings::SharedStrings;
use super::styles::Styles;
use super::decrypt::XlsxDecrypter;
use crate::config::{DateSystem, ReaderConfig, SheetSelector};
use crate::error::{DataForgeError, Result};
use crate::memory::MemoryTracker;
use crate::types::{CellValue, Row, RowBatch, SheetMetadata};

/// Streaming XLSX reader that processes worksheets row-by-row.
///
/// # Architecture
/// 1. Opens the .xlsx ZIP archive
/// 2. Reads xl/sharedStrings.xml (string deduplication table)
/// 3. Reads xl/styles.xml (number format detection for dates)
/// 4. Reads xl/workbook.xml (sheet names and metadata)
/// 5. Streams xl/worksheets/sheet{N}.xml row by row
///
/// Steps 2-4 are done upfront (these files are typically small).
/// Step 5 is the streaming part — we never load the full worksheet XML.
///
/// # Example
/// ```no_run
/// use dataforge_core::xlsx::XlsxReader;
/// use dataforge_core::config::ReaderConfig;
///
/// let reader = XlsxReader::open("data.xlsx", ReaderConfig::default()).unwrap();
/// for batch in reader {
///     let batch = batch.unwrap();
///     println!("Batch with {} rows", batch.len());
/// }
/// ```
pub struct XlsxReader {
    /// Pre-loaded shared strings table
    shared_strings: SharedStrings,

    /// Pre-loaded styles for date detection
    styles: Styles,

    /// The worksheet XML data (read from ZIP into memory)
    /// For very large worksheets, this could be significant, but
    /// it's still much smaller than the parsed DOM would be.
    worksheet_data: Vec<u8>,

    /// Current position in the XML parsing
    parse_state: ParseState,

    /// Column headers (from first row or user-provided)
    headers: Option<Vec<String>>,

    /// Memory tracker for backpressure
    memory_tracker: Arc<MemoryTracker>,

    /// Sheet metadata
    sheet_metadata: SheetMetadata,

    /// Configuration
    config: ReaderConfig,

    /// Date system (1900 or 1904 based)
    date_system: DateSystem,

    /// Whether the reader has been exhausted
    exhausted: bool,
}

/// Internal parsing state machine for the worksheet XML.
struct ParseState {
    /// Current row index being processed
    current_row: u64,

    /// Number of data rows emitted (after skip_rows)
    rows_emitted: u64,

    /// Byte offset into worksheet_data (for resumed parsing)
    byte_offset: usize,

    /// Whether we've read the first row as headers
    headers_read: bool,
}

impl XlsxReader {
    /// Open an XLSX file from a filesystem path.
    ///
    /// # Arguments
    /// * `path` - Path to the .xlsx file
    /// * `config` - Reader configuration
    ///
    /// # Errors
    /// - `DataForgeError::Io` if the file cannot be opened
    /// - `DataForgeError::Zip` if the file is not a valid ZIP archive
    /// - `DataForgeError::XlsxParse` if required XML components are missing
    pub fn open(path: impl AsRef<Path>, config: ReaderConfig) -> Result<Self> {
        let path = path.as_ref();
        config.validate()?;

        info!(path = %path.display(), "Opening XLSX file");

        let mut file = File::open(path).map_err(|e| {
            DataForgeError::io(e, format!("Failed to open XLSX file '{}'", path.display()))
        })?;

        // Read first 8 bytes to check if encrypted
        let mut magic = [0u8; 8];
        let bytes_read = file.read(&mut magic).unwrap_or(0);
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(0))?;

        if bytes_read == 8 && XlsxDecrypter::is_encrypted(&magic) {
            let mut encrypted_bytes = Vec::new();
            file.read_to_end(&mut encrypted_bytes)?;
            let password = config.xlsx.password.as_deref().ok_or_else(|| {
                DataForgeError::config("File is encrypted. Please specify a password.")
            })?;
            let decrypted_bytes = XlsxDecrypter::decrypt(&encrypted_bytes, password)?;
            return Self::from_bytes(decrypted_bytes, config);
        }

        let buf_reader = BufReader::with_capacity(64 * 1024, file);
        Self::from_reader(buf_reader, config)
    }

    /// Open an XLSX from in-memory bytes.
    ///
    /// Primary entry point for WASM and FFI consumers.
    pub fn from_bytes(data: Vec<u8>, config: ReaderConfig) -> Result<Self> {
        if XlsxDecrypter::is_encrypted(&data) {
            let password = config.xlsx.password.as_deref().ok_or_else(|| {
                DataForgeError::config("File is encrypted. Please specify a password.")
            })?;
            let decrypted_bytes = XlsxDecrypter::decrypt(&data, password)?;
            let cursor = std::io::Cursor::new(decrypted_bytes);
            return Self::from_reader(cursor, config);
        }
        let cursor = std::io::Cursor::new(data);
        Self::from_reader(cursor, config)
    }

    /// Internal constructor from any Read + Seek source.
    fn from_reader<R: Read + std::io::Seek>(reader: R, config: ReaderConfig) -> Result<Self> {
        let memory_tracker = MemoryTracker::new(config.max_memory_bytes, config.backpressure);

        let mut archive = ZipArchive::new(reader)?;

        // Step 1: Parse shared strings table
        let shared_strings = match read_zip_entry(&mut archive, "xl/sharedStrings.xml") {
            Ok(data) => SharedStrings::parse(&data)?,
            Err(_) => {
                debug!("No shared strings table found (file may have no string cells)");
                SharedStrings::new()
            }
        };

        // Step 2: Parse styles for date detection
        let styles = match read_zip_entry(&mut archive, "xl/styles.xml") {
            Ok(data) => Styles::parse(&data)?,
            Err(_) => {
                debug!("No styles table found");
                Styles::new()
            }
        };

        // Step 3: Read workbook to get sheet names
        let sheet_names = parse_workbook_sheets(&mut archive)?;

        // Step 4: Determine which sheet to read
        let (sheet_name, sheet_index) = select_sheet(&sheet_names, &config.xlsx.sheet_selector)?;

        // Step 5: Determine the date system
        let date_system = detect_date_system(&mut archive).unwrap_or(config.xlsx.date_system);

        // Step 6: Read the worksheet XML data
        let sheet_path = format!("xl/worksheets/sheet{}.xml", sheet_index + 1);
        let worksheet_data = read_zip_entry(&mut archive, &sheet_path).map_err(|_| {
            DataForgeError::XlsxParse {
                component: "worksheet".to_string(),
                message: format!("Worksheet '{}' not found in archive", sheet_path),
            }
        })?;

        info!(
            sheet = %sheet_name,
            sheet_index,
            shared_strings = shared_strings.len(),
            worksheet_size_mb = worksheet_data.len() as f64 / 1_048_576.0,
            "XLSX file loaded, starting stream"
        );

        let sheet_metadata = SheetMetadata {
            name: sheet_name.clone(),
            index: sheet_index,
            row_count: None,
            column_count: 0,
            columns: Vec::new(),
            is_active: true,
        };

        Ok(XlsxReader {
            shared_strings,
            styles,
            worksheet_data,
            parse_state: ParseState {
                current_row: 0,
                rows_emitted: 0,
                byte_offset: 0,
                headers_read: false,
            },
            headers: None,
            memory_tracker: memory_tracker,
            sheet_metadata,
            config,
            date_system,
            exhausted: false,
        })
    }

    /// Get column headers.
    pub fn headers(&self) -> Option<&[String]> {
        self.headers.as_deref()
    }

    /// Get sheet metadata.
    pub fn sheet_metadata(&self) -> &SheetMetadata {
        &self.sheet_metadata
    }

    /// Get current memory stats.
    pub fn memory_stats(&self) -> crate::memory::MemoryStats {
        self.memory_tracker.stats()
    }

    /// Get all sheet names from the workbook.
    pub fn sheet_names(path: impl AsRef<Path>) -> Result<Vec<String>> {
        let file = File::open(path.as_ref())?;
        let mut archive = ZipArchive::new(BufReader::new(file))?;
        parse_workbook_sheets(&mut archive)
    }

    /// Fast scanner for sheet metadata without parsing all cell/row values.
    pub fn scan_metadata(path: impl AsRef<Path>) -> Result<Vec<crate::types::SheetMetadata>> {
        let file = File::open(path.as_ref())?;
        let mut archive = ZipArchive::new(BufReader::new(file))?;
        let sheet_names = parse_workbook_sheets(&mut archive)?;

        let mut metadata_list = Vec::new();
        for (idx, name) in sheet_names.iter().enumerate() {
            metadata_list.push(crate::types::SheetMetadata {
                name: name.clone(),
                index: idx,
                row_count: None,
                column_count: 0,
                columns: Vec::new(),
                is_active: idx == 0,
            });
        }

        Ok(metadata_list)
    }

    /// Read the next batch of rows from the worksheet.
    ///
    /// Returns `None` when the worksheet has been fully consumed.
    pub fn next_batch(&mut self) -> Option<Result<RowBatch>> {
        if self.exhausted {
            return None;
        }

        let mut res = match self.parse_next_batch() {
            Ok(Some(batch)) => Some(Ok(batch)),
            Ok(None) => {
                self.exhausted = true;
                None
            }
            Err(e) => {
                self.exhausted = true;
                Some(Err(e))
            }
        };

        if let Some(Ok(ref mut batch)) = res {
            if let Err(e) = crate::schema::apply_schema(batch, &self.config) {
                res = Some(Err(e));
                self.exhausted = true;
            }
        }
        res
    }

    /// Internal: parse the next batch of rows from the worksheet XML.
    fn parse_next_batch(&mut self) -> Result<Option<RowBatch>> {
        let batch_size = self.config.batch_size;
        let data = &self.worksheet_data[self.parse_state.byte_offset..];

        if data.is_empty() {
            return Ok(None);
        }

        let mut reader = XmlReader::from_reader(data);
        reader.config_mut().trim_text(true);

        let mut batch = RowBatch::with_capacity(self.parse_state.current_row, batch_size);
        let mut buf = Vec::with_capacity(1024);

        // Current row being built
        let mut current_cells: Vec<CellValue> = Vec::with_capacity(64);
        let mut in_row = false;
        let mut in_cell = false;
        let mut in_value = false;
        let mut in_inline_str = false;
        let mut cell_type: Option<String> = None;
        let mut cell_style: Option<u32> = None;
        let mut cell_value_text = String::new();
        let mut cell_ref: Option<String>;

        loop {
            // Check max_rows limit
            if let Some(max) = self.config.max_rows {
                if self.parse_state.rows_emitted >= max {
                    batch.is_last = true;
                    break;
                }
            }

            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    match e.local_name().as_ref() {
                        b"row" => {
                            in_row = true;
                            current_cells.clear();
                        }
                        b"c" if in_row => {
                            // Cell element: <c r="A1" t="s" s="1">
                            in_cell = true;
                            cell_type = None;
                            cell_style = None;
                            cell_value_text.clear();
                            cell_ref = None;

                            for attr in e.attributes() {
                                if let Ok(attr) = attr {
                                    match attr.key.as_ref() {
                                        b"t" => {
                                            cell_type = Some(
                                                String::from_utf8_lossy(&attr.value).into_owned(),
                                            );
                                        }
                                        b"s" => {
                                            if let Ok(val) = String::from_utf8_lossy(&attr.value)
                                                .parse::<u32>()
                                            {
                                                cell_style = Some(val);
                                            }
                                        }
                                        b"r" => {
                                            cell_ref = Some(
                                                String::from_utf8_lossy(&attr.value).into_owned(),
                                            );
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            // Handle column gaps (cells may be sparse)
                            if let Some(ref r) = cell_ref {
                                let target_col = column_ref_to_index(r);
                                while current_cells.len() < target_col {
                                    current_cells.push(CellValue::Null);
                                }
                            }
                        }
                        b"v" if in_cell => {
                            // Value element
                            in_value = true;
                            cell_value_text.clear();
                        }
                        b"is" if in_cell => {
                            // Inline string
                            in_inline_str = true;
                            cell_value_text.clear();
                        }
                        b"t" if in_inline_str => {
                            // Text within inline string
                            in_value = true;
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(ref e)) => {
                    match e.local_name().as_ref() {
                        b"row" => {
                            in_row = false;
                            self.parse_state.current_row += 1;

                            // Skip rows if configured
                            if self.parse_state.current_row <= self.config.skip_rows {
                                continue;
                            }

                            // Handle header row
                            if !self.parse_state.headers_read && self.config.csv.has_header {
                                let headers: Vec<String> = current_cells
                                    .iter()
                                    .map(|c| c.to_display_string())
                                    .collect();
                                self.headers = Some(headers);
                                self.parse_state.headers_read = true;
                                continue;
                            }

                            // Apply column selection
                            let cells = if let Some(ref cols) = self.config.columns {
                                cols.iter()
                                    .map(|&i| {
                                        current_cells
                                            .get(i)
                                            .cloned()
                                            .unwrap_or(CellValue::Null)
                                    })
                                    .collect()
                            } else {
                                std::mem::take(&mut current_cells)
                            };

                            let mut row = Row::new(self.parse_state.current_row - 1);
                            row.cells = cells.into();
                            batch.push(row);
                            self.parse_state.rows_emitted += 1;

                            // Flush batch when full
                            if batch.len() >= batch_size {
                                batch.headers = self.headers.clone();
                                // Update byte offset for next batch
                                self.parse_state.byte_offset += reader.buffer_position() as usize;
                                return Ok(Some(batch));
                            }
                        }
                        b"c" => {
                            // End of cell — resolve the value
                            in_cell = false;

                            let cell_value = resolve_cell_value(
                                &cell_value_text,
                                cell_type.as_deref(),
                                cell_style,
                                &self.shared_strings,
                                &self.styles,
                                self.date_system,
                            );

                            current_cells.push(cell_value);
                        }
                        b"v" => {
                            in_value = false;
                        }
                        b"is" => {
                            in_inline_str = false;
                        }
                        b"t" if in_inline_str => {
                            in_value = false;
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(ref e)) if in_value => {
                    if let Ok(text) = e.unescape() {
                        cell_value_text.push_str(&text);
                    }
                }
                Ok(Event::Eof) => {
                    // End of worksheet
                    batch.is_last = true;
                    break;
                }
                Err(e) => {
                    return Err(DataForgeError::XlsxParse {
                        component: "worksheet".to_string(),
                        message: format!("XML parse error: {e}"),
                    });
                }
                _ => {}
            }
            buf.clear();
        }

        batch.headers = self.headers.clone();

        if batch.is_empty() {
            Ok(None)
        } else {
            self.parse_state.byte_offset = self.worksheet_data.len(); // Mark as fully consumed
            Ok(Some(batch))
        }
    }
}

/// Implement Iterator for ergonomic `for batch in reader` usage.
impl Iterator for XlsxReader {
    type Item = Result<RowBatch>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_batch()
    }
}

// =============================================================================
// Internal helper functions
// =============================================================================

/// Resolve a cell value from its raw text, type indicator, and style.
///
/// Cell types in XLSX:
/// - `s` → Shared string (value is an index into shared strings table)
/// - `b` → Boolean (0 or 1)
/// - `e` → Error (#VALUE!, #REF!, etc.)
/// - `str` → Inline formula result string
/// - `inlineStr` → Inline string (value from <is><t> elements)
/// - (none) → Number (could be int, float, or date depending on style)
fn resolve_cell_value(
    raw_value: &str,
    cell_type: Option<&str>,
    cell_style: Option<u32>,
    shared_strings: &SharedStrings,
    styles: &Styles,
    date_system: DateSystem,
) -> CellValue {
    if raw_value.is_empty() && cell_type.is_none() {
        return CellValue::Null;
    }

    match cell_type {
        Some("s") => {
            // Shared string — value is the index
            match raw_value.parse::<usize>() {
                Ok(idx) => match shared_strings.get(idx) {
                    Some(s) => CellValue::String(CompactString::new(s)),
                    None => {
                        warn!(index = idx, "Shared string index out of bounds");
                        CellValue::Null
                    }
                },
                Err(_) => CellValue::Null,
            }
        }
        Some("b") => {
            // Boolean
            CellValue::Bool(raw_value == "1" || raw_value.eq_ignore_ascii_case("true"))
        }
        Some("e") => {
            // Error cell
            let error = match raw_value {
                "#NULL!" => crate::types::CellError::Null,
                "#DIV/0!" => crate::types::CellError::DivZero,
                "#VALUE!" => crate::types::CellError::Value,
                "#REF!" => crate::types::CellError::Ref,
                "#NAME?" => crate::types::CellError::Name,
                "#NUM!" => crate::types::CellError::Num,
                "#N/A" => crate::types::CellError::Na,
                _ => crate::types::CellError::Value,
            };
            CellValue::Error(error)
        }
        Some("str") | Some("inlineStr") => {
            // Inline string
            CellValue::String(CompactString::new(raw_value))
        }
        _ => {
            // Numeric value — could be int, float, or date
            if raw_value.is_empty() {
                return CellValue::Null;
            }

            // Check if this is a date format
            if let Some(style_idx) = cell_style {
                if styles.is_date_format(style_idx) {
                    if let Ok(serial) = raw_value.parse::<f64>() {
                        if let Some(dt) = serial_to_datetime(serial, date_system) {
                            return CellValue::DateTime(dt);
                        }
                    }
                }
            }

            // Try integer first (more specific)
            if !raw_value.contains('.') && !raw_value.contains('E') && !raw_value.contains('e') {
                if let Ok(v) = raw_value.parse::<i64>() {
                    return CellValue::Int(v);
                }
            }

            // Try float
            if let Ok(v) = raw_value.parse::<f64>() {
                return CellValue::Float(v);
            }

            // Fallback to string
            CellValue::String(CompactString::new(raw_value))
        }
    }
}

/// Convert an Excel serial date number to a NaiveDateTime.
///
/// Excel serial dates count days from a base date:
/// - 1900 system: Day 1 = January 1, 1900
/// - 1904 system: Day 0 = January 1, 1904
///
/// The fractional part represents the time of day (0.5 = noon).
///
/// Note: Excel incorrectly treats 1900 as a leap year (Feb 29, 1900 exists
/// in Excel but not in reality). This is a well-known Lotus 1-2-3 bug.
fn serial_to_datetime(serial: f64, date_system: DateSystem) -> Option<NaiveDateTime> {
    if serial < 0.0 {
        return None;
    }

    let (base_date, serial_adj) = match date_system {
        DateSystem::Base1900 => {
            // Account for the Lotus 1-2-3 leap year bug:
            // Excel thinks Feb 29, 1900 exists, so dates after that are off by 1
            let adj = if serial > 60.0 { serial - 2.0 } else { serial - 1.0 };
            (NaiveDate::from_ymd_opt(1900, 1, 1)?, adj)
        }
        DateSystem::Base1904 => (NaiveDate::from_ymd_opt(1904, 1, 1)?, serial),
    };

    let days = serial_adj.floor() as i64;
    let time_fraction = serial_adj - serial_adj.floor();

    // Convert fractional day to time components
    let total_seconds = (time_fraction * 86400.0).round() as u32;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    let date = base_date.checked_add_signed(chrono::Duration::days(days))?;
    let time = NaiveTime::from_hms_opt(hours, minutes, seconds)?;

    Some(NaiveDateTime::new(date, time))
}

/// Convert an Excel column reference (e.g., "A1", "AB2", "ZZ100") to a 0-based column index.
///
/// Only the letter part is used (e.g., "A" → 0, "B" → 1, "Z" → 25, "AA" → 26).
fn column_ref_to_index(cell_ref: &str) -> usize {
    let mut col = 0usize;
    for ch in cell_ref.chars() {
        if ch.is_ascii_alphabetic() {
            col = col * 26 + (ch.to_ascii_uppercase() as usize - b'A' as usize + 1);
        } else {
            break; // Hit a digit — stop
        }
    }
    col.saturating_sub(1) // Convert from 1-based to 0-based
}

/// Read a specific entry from a ZIP archive into a byte vector.
fn read_zip_entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>> {
    let mut file = archive.by_name(name).map_err(|e| DataForgeError::Zip {
        message: format!("Entry '{}' not found in ZIP: {}", name, e),
    })?;

    let mut data = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut data)?;
    Ok(data)
}

/// Parse sheet names from xl/workbook.xml.
fn parse_workbook_sheets<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Vec<String>> {
    let data = read_zip_entry(archive, "xl/workbook.xml")?;
    let mut reader = XmlReader::from_reader(data.as_slice());
    reader.config_mut().trim_text(true);

    let mut sheets = Vec::new();
    let mut buf = Vec::with_capacity(512);
    let mut in_sheets = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                match e.local_name().as_ref() {
                    b"sheets" => {
                        in_sheets = true;
                    }
                    b"sheet" if in_sheets => {
                        for attr in e.attributes() {
                            if let Ok(attr) = attr {
                                if attr.key.as_ref() == b"name" {
                                    let name = String::from_utf8_lossy(&attr.value).into_owned();
                                    sheets.push(name);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                if e.local_name().as_ref() == b"sheets" {
                    in_sheets = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(DataForgeError::XlsxParse {
                    component: "workbook".to_string(),
                    message: format!("XML parse error: {e}"),
                });
            }
            _ => {}
        }
        buf.clear();
    }

    debug!(sheets = ?sheets, "Parsed workbook sheet names");
    Ok(sheets)
}

/// Detect the date system from xl/workbook.xml.
///
/// If the workbook contains `<workbookPr date1904="1"/>`, use the 1904 system.
fn detect_date_system<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Option<DateSystem> {
    let data = read_zip_entry(archive, "xl/workbook.xml").ok()?;
    let content = std::str::from_utf8(&data).ok()?;

    if content.contains("date1904=\"1\"") || content.contains("date1904=\"true\"") {
        Some(DateSystem::Base1904)
    } else {
        Some(DateSystem::Base1900)
    }
}

/// Select a sheet based on the SheetSelector config.
fn select_sheet(
    sheet_names: &[String],
    selector: &SheetSelector,
) -> Result<(String, usize)> {
    if sheet_names.is_empty() {
        return Err(DataForgeError::XlsxParse {
            component: "workbook".to_string(),
            message: "Workbook contains no sheets".to_string(),
        });
    }

    match selector {
        SheetSelector::First => Ok((sheet_names[0].clone(), 0)),
        SheetSelector::ByName(name) => {
            let index = sheet_names.iter().position(|s| s == name).ok_or_else(|| {
                DataForgeError::SheetNotFound {
                    name: name.clone(),
                    available: sheet_names.join(", "),
                }
            })?;
            Ok((name.clone(), index))
        }
        SheetSelector::ByIndex(idx) => {
            if *idx >= sheet_names.len() {
                return Err(DataForgeError::SheetNotFound {
                    name: format!("index {idx}"),
                    available: sheet_names.join(", "),
                });
            }
            Ok((sheet_names[*idx].clone(), *idx))
        }
        SheetSelector::All => {
            // Default to first sheet; the consumer can re-open for other sheets
            Ok((sheet_names[0].clone(), 0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_ref_to_index() {
        assert_eq!(column_ref_to_index("A1"), 0);
        assert_eq!(column_ref_to_index("B1"), 1);
        assert_eq!(column_ref_to_index("Z1"), 25);
        assert_eq!(column_ref_to_index("AA1"), 26);
        assert_eq!(column_ref_to_index("AB1"), 27);
        assert_eq!(column_ref_to_index("AZ1"), 51);
        assert_eq!(column_ref_to_index("BA1"), 52);
    }

    #[test]
    fn test_serial_to_datetime_1900() {
        // January 1, 1900 (serial = 1)
        let dt = serial_to_datetime(1.0, DateSystem::Base1900).unwrap();
        assert_eq!(dt.date(), NaiveDate::from_ymd_opt(1900, 1, 1).unwrap());

        // January 1, 2024 (serial = 45292)
        let dt = serial_to_datetime(45292.0, DateSystem::Base1900).unwrap();
        assert_eq!(dt.date(), NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
    }

    #[test]
    fn test_serial_to_datetime_with_time() {
        // 0.5 = noon
        let dt = serial_to_datetime(1.5, DateSystem::Base1900).unwrap();
        assert_eq!(dt.time(), NaiveTime::from_hms_opt(12, 0, 0).unwrap());

        // 0.75 = 6:00 PM
        let dt = serial_to_datetime(1.75, DateSystem::Base1900).unwrap();
        assert_eq!(dt.time(), NaiveTime::from_hms_opt(18, 0, 0).unwrap());
    }

    #[test]
    fn test_resolve_shared_string() {
        // Manually create shared strings for testing
        let xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
            <si><t>Hello</t></si>
            <si><t>World</t></si>
        </sst>"#;
        let sst = SharedStrings::parse(xml).unwrap();
        let styles = Styles::new();

        let val = resolve_cell_value("0", Some("s"), None, &sst, &styles, DateSystem::Base1900);
        assert_eq!(val.as_str(), Some("Hello"));

        let val = resolve_cell_value("1", Some("s"), None, &sst, &styles, DateSystem::Base1900);
        assert_eq!(val.as_str(), Some("World"));
    }

    #[test]
    fn test_resolve_boolean() {
        let sst = SharedStrings::new();
        let styles = Styles::new();

        let val = resolve_cell_value("1", Some("b"), None, &sst, &styles, DateSystem::Base1900);
        assert_eq!(val.as_bool(), Some(true));

        let val = resolve_cell_value("0", Some("b"), None, &sst, &styles, DateSystem::Base1900);
        assert_eq!(val.as_bool(), Some(false));
    }

    #[test]
    fn test_resolve_number() {
        let sst = SharedStrings::new();
        let styles = Styles::new();

        let val = resolve_cell_value("42", None, None, &sst, &styles, DateSystem::Base1900);
        assert_eq!(val.as_int(), Some(42));

        let val = resolve_cell_value("3.15", None, None, &sst, &styles, DateSystem::Base1900);
        assert_eq!(val.as_float(), Some(3.15));
    }

    #[test]
    fn test_resolve_error() {
        let sst = SharedStrings::new();
        let styles = Styles::new();

        let val =
            resolve_cell_value("#DIV/0!", Some("e"), None, &sst, &styles, DateSystem::Base1900);
        assert!(matches!(val, CellValue::Error(crate::types::CellError::DivZero)));
    }

    #[test]
    fn test_select_sheet() {
        let sheets = vec!["Sheet1".to_string(), "Data".to_string(), "Summary".to_string()];

        let (name, idx) = select_sheet(&sheets, &SheetSelector::First).unwrap();
        assert_eq!(name, "Sheet1");
        assert_eq!(idx, 0);

        let (name, idx) =
            select_sheet(&sheets, &SheetSelector::ByName("Data".to_string())).unwrap();
        assert_eq!(name, "Data");
        assert_eq!(idx, 1);

        let (name, idx) = select_sheet(&sheets, &SheetSelector::ByIndex(2)).unwrap();
        assert_eq!(name, "Summary");
        assert_eq!(idx, 2);

        // Non-existent sheet
        assert!(select_sheet(&sheets, &SheetSelector::ByName("Missing".to_string())).is_err());
    }
}
