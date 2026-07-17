// =============================================================================
// DataForge Core — ODS Reader
// =============================================================================
// Streaming reader for OpenDocument Spreadsheet (.ods) files.
// ODS is the native format of LibreOffice Calc and Apache OpenOffice Calc.
//
// ODS structure (ZIP archive):
//   content.xml — Contains all sheet data, styles, and metadata
//   meta.xml — Document metadata
//   styles.xml — Named styles
//   META-INF/manifest.xml — Package manifest
//
// Unlike XLSX which has separate files per sheet, ODS stores everything
// in a single content.xml. We parse it with SAX-style streaming.
// =============================================================================

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;

use compact_str::CompactString;
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use tracing::info;
use zip::ZipArchive;

use crate::config::{ReaderConfig, SheetSelector};
use crate::error::{DataForgeError, Result};
use crate::memory::MemoryTracker;
use crate::types::{CellValue, Row, RowBatch, SheetMetadata};

/// Streaming ODS reader that processes worksheets row-by-row.
///
/// # Example
/// ```no_run
/// use dataforge_core::ods::OdsReader;
/// use dataforge_core::config::ReaderConfig;
///
/// let reader = OdsReader::open("data.ods", ReaderConfig::default()).unwrap();
/// for batch in reader {
///     let batch = batch.unwrap();
///     println!("Batch with {} rows", batch.len());
/// }
/// ```
pub struct OdsReader {
    /// The content.xml data
    content_data: Vec<u8>,

    /// Current byte offset in content_data
    byte_offset: usize,

    /// Column headers
    headers: Option<Vec<String>>,

    /// Memory tracker
    memory_tracker: Arc<MemoryTracker>,

    /// Configuration
    config: ReaderConfig,

    /// Sheet metadata
    sheet_metadata: SheetMetadata,

    /// Current row index
    current_row: u64,

    /// Rows emitted
    rows_emitted: u64,

    /// Headers read flag
    headers_read: bool,

    /// Whether reader is exhausted
    exhausted: bool,

    /// Target sheet name
    target_sheet: Option<String>,
}

impl OdsReader {
    /// Open an ODS file from a filesystem path.
    pub fn open(path: impl AsRef<Path>, config: ReaderConfig) -> Result<Self> {
        let path = path.as_ref();
        config.validate()?;

        info!(path = %path.display(), "Opening ODS file");

        let file = File::open(path).map_err(|e| {
            DataForgeError::io(e, format!("Failed to open ODS file '{}'", path.display()))
        })?;
        let buf_reader = BufReader::with_capacity(64 * 1024, file);

        Self::from_reader(buf_reader, config)
    }

    /// Open from in-memory bytes.
    pub fn from_bytes(data: Vec<u8>, config: ReaderConfig) -> Result<Self> {
        let cursor = std::io::Cursor::new(data);
        Self::from_reader(cursor, config)
    }

    /// Internal constructor from any Read + Seek source.
    fn from_reader<R: Read + std::io::Seek>(reader: R, config: ReaderConfig) -> Result<Self> {
        let memory_tracker = MemoryTracker::new(config.max_memory_bytes, config.backpressure);

        let mut archive = ZipArchive::new(reader)?;

        // Read content.xml
        let mut content_data = {
            let mut file = archive.by_name("content.xml").map_err(|e| {
                DataForgeError::OdsParse {
                    component: "archive".to_string(),
                    message: format!("content.xml not found: {e}"),
                }
            })?;
            let mut data = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut data)?;
            data
        };

        // Decrypt content.xml if password protected in META-INF/manifest.xml
        if let Ok(mut manifest_file) = archive.by_name("META-INF/manifest.xml") {
            let mut manifest_data = Vec::with_capacity(manifest_file.size() as usize);
            if manifest_file.read_to_end(&mut manifest_data).is_ok() {
                if let Ok(Some(enc_info)) = super::decrypt::parse_manifest_encryption(&manifest_data, "content.xml") {
                    let password = config.ods.password.as_deref().ok_or_else(|| {
                        DataForgeError::config("ODS workbook is encrypted but no password was provided")
                    })?;
                    content_data = super::decrypt::decrypt_ods_entry(&content_data, password, &enc_info)?;
                }
            }
        }

        // Determine target sheet
        let target_sheet = match &config.ods.sheet_selector {
            SheetSelector::ByName(name) => Some(name.clone()),
            _ => None,
        };

        info!(
            content_size_mb = content_data.len() as f64 / 1_048_576.0,
            "ODS file loaded, starting stream"
        );

        Ok(OdsReader {
            content_data,
            byte_offset: 0,
            headers: None,
            memory_tracker,
            config,
            sheet_metadata: SheetMetadata {
                name: "Sheet1".to_string(),
                index: 0,
                row_count: None,
                column_count: 0,
                columns: Vec::new(),
                is_active: true,
            },
            current_row: 0,
            rows_emitted: 0,
            headers_read: false,
            exhausted: false,
            target_sheet,
        })
    }

    /// Get column headers.
    pub fn headers(&self) -> Option<&[String]> {
        self.headers.as_deref()
    }

    /// Get current memory stats.
    pub fn memory_stats(&self) -> crate::memory::MemoryStats {
        self.memory_tracker.stats()
    }

    /// Read the next batch of rows.
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

    /// Parse the next batch from content.xml.
    fn parse_next_batch(&mut self) -> Result<Option<RowBatch>> {
        let batch_size = self.config.batch_size;
        let data = &self.content_data[self.byte_offset..];

        if data.is_empty() {
            return Ok(None);
        }

        let mut reader = XmlReader::from_reader(data);
        reader.config_mut().trim_text(true);

        let mut batch = RowBatch::with_capacity(self.current_row, batch_size);
        let mut buf = Vec::with_capacity(1024);

        let mut current_cells: Vec<CellValue> = Vec::new();
        let mut in_table = false;
        let mut in_row = false;
        let mut in_cell = false;
        let mut in_text = false;
        let mut cell_text = String::new();
        let mut cell_repeat: u32 = 1;
        let mut row_repeat: u32 = 1;
        let mut in_target_sheet = self.target_sheet.is_none();
        let mut cell_value_type: Option<String> = None;
        let mut cell_value_attr: Option<String> = None;

        loop {
            if let Some(max) = self.config.max_rows {
                if self.rows_emitted >= max {
                    batch.is_last = true;
                    break;
                }
            }

            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    let local = e.local_name();
                    match local.as_ref() {
                        b"table" => {
                            // Check sheet name
                            let mut sheet_name = String::new();
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref().ends_with(b"name") {
                                    sheet_name =
                                        String::from_utf8_lossy(&attr.value).into_owned();
                                }
                            }

                            if let Some(ref target) = self.target_sheet {
                                in_target_sheet = &sheet_name == target;
                            } else {
                                in_target_sheet = true;
                            }

                            if in_target_sheet {
                                in_table = true;
                                self.sheet_metadata.name = sheet_name;
                            }
                        }
                        b"table-row" if in_table && in_target_sheet => {
                            in_row = true;
                            current_cells.clear();
                            row_repeat = 1;

                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref().ends_with(b"number-rows-repeated") {
                                    if let Ok(val) =
                                        String::from_utf8_lossy(&attr.value).parse::<u32>()
                                    {
                                        row_repeat = val;
                                    }
                                }
                            }
                        }
                        b"table-cell" if in_row => {
                            in_cell = true;
                            cell_text.clear();
                            cell_repeat = 1;
                            cell_value_type = None;
                            cell_value_attr = None;

                            for attr in e.attributes().flatten() {
                                let key = attr.key.as_ref();
                                if key.ends_with(b"number-columns-repeated") {
                                    if let Ok(val) =
                                        String::from_utf8_lossy(&attr.value).parse::<u32>()
                                    {
                                        cell_repeat = val;
                                    }
                                } else if key.ends_with(b"value-type") {
                                    cell_value_type =
                                        Some(String::from_utf8_lossy(&attr.value).into_owned());
                                } else if key.ends_with(b"value")
                                    && !key.ends_with(b"value-type")
                                {
                                    cell_value_attr =
                                        Some(String::from_utf8_lossy(&attr.value).into_owned());
                                } else if key.ends_with(b"date-value") {
                                    cell_value_attr =
                                        Some(String::from_utf8_lossy(&attr.value).into_owned());
                                } else if key.ends_with(b"boolean-value") {
                                    cell_value_attr =
                                        Some(String::from_utf8_lossy(&attr.value).into_owned());
                                }
                            }

                            // Handle empty cells (Empty event = self-closing tag)
                            if matches!(reader.read_event_into(&mut Vec::new()), Ok(Event::End(_)))
                            {
                                let value = resolve_ods_cell(
                                    &cell_text,
                                    cell_value_type.as_deref(),
                                    cell_value_attr.as_deref(),
                                );
                                for _ in 0..cell_repeat.min(1000) {
                                    current_cells.push(value.clone());
                                }
                                in_cell = false;
                            }
                        }
                        b"p" if in_cell => {
                            in_text = true;
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(ref e)) => {
                    let local = e.local_name();
                    match local.as_ref() {
                        b"table" => {
                            in_table = false;
                            if in_target_sheet {
                                batch.is_last = true;
                            }
                        }
                        b"table-row" if in_row => {
                            in_row = false;

                            // Only process if not an empty repeated row
                            let is_empty = current_cells.iter().all(|c| c.is_null());
                            if is_empty && row_repeat > 1 {
                                self.current_row += row_repeat as u64;
                                continue;
                            }

                            // Handle header row
                            if !self.headers_read && self.config.csv.has_header {
                                let headers: Vec<String> = current_cells
                                    .iter()
                                    .map(|c| c.to_display_string())
                                    .collect();
                                self.headers = Some(headers);
                                self.headers_read = true;
                                self.current_row += 1;
                                continue;
                            }

                            // Skip rows
                            if self.current_row < self.config.skip_rows {
                                self.current_row += 1;
                                continue;
                            }

                            let mut row = Row::new(self.current_row);
                            row.cells = std::mem::take(&mut current_cells).into();
                            batch.push(row);
                            self.rows_emitted += 1;
                            self.current_row += 1;

                            if batch.len() >= batch_size {
                                batch.headers = self.headers.clone();
                                self.byte_offset += reader.buffer_position() as usize;
                                return Ok(Some(batch));
                            }
                        }
                        b"table-cell" if in_cell => {
                            in_cell = false;
                            let value = resolve_ods_cell(
                                &cell_text,
                                cell_value_type.as_deref(),
                                cell_value_attr.as_deref(),
                            );
                            for _ in 0..cell_repeat.min(1000) {
                                current_cells.push(value.clone());
                            }
                        }
                        b"p" => {
                            in_text = false;
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(ref e)) if in_text => {
                    if let Ok(text) = e.unescape() {
                        cell_text.push_str(&text);
                    }
                }
                Ok(Event::Eof) => {
                    batch.is_last = true;
                    break;
                }
                Err(e) => {
                    return Err(DataForgeError::OdsParse {
                        component: "content".to_string(),
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
            self.byte_offset = self.content_data.len();
            Ok(Some(batch))
        }
    }
}

impl Iterator for OdsReader {
    type Item = Result<RowBatch>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_batch()
    }
}

/// Resolve an ODS cell value from its attributes.
fn resolve_ods_cell(
    text: &str,
    value_type: Option<&str>,
    value_attr: Option<&str>,
) -> CellValue {
    match value_type {
        Some("float") | Some("currency") | Some("percentage") => {
            if let Some(val) = value_attr {
                if let Ok(f) = val.parse::<f64>() {
                    if f == f.floor() && f.abs() < i64::MAX as f64 {
                        return CellValue::Int(f as i64);
                    }
                    return CellValue::Float(f);
                }
            }
            CellValue::Null
        }
        Some("string") => {
            if text.is_empty() {
                CellValue::Null
            } else {
                CellValue::String(CompactString::new(text))
            }
        }
        Some("boolean") => {
            let val = value_attr.unwrap_or("false");
            CellValue::Bool(val == "true")
        }
        Some("date") => {
            if let Some(val) = value_attr {
                // ODS dates are ISO 8601 format
                if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(val, "%Y-%m-%dT%H:%M:%S") {
                    return CellValue::DateTime(dt);
                }
                if let Ok(d) = chrono::NaiveDate::parse_from_str(val, "%Y-%m-%d") {
                    return CellValue::Date(d);
                }
            }
            CellValue::String(CompactString::new(text))
        }
        Some("time") => {
            // ODS time format: PT12H30M00S (ISO 8601 duration)
            if let Some(val) = value_attr {
                if let Some(time) = parse_ods_duration(val) {
                    return CellValue::Time(time);
                }
            }
            CellValue::String(CompactString::new(text))
        }
        None | Some("") => {
            if text.is_empty() {
                CellValue::Null
            } else {
                CellValue::String(CompactString::new(text))
            }
        }
        _ => CellValue::String(CompactString::new(text)),
    }
}

/// Parse an ODS duration string (ISO 8601 format: PT12H30M00S).
fn parse_ods_duration(s: &str) -> Option<chrono::NaiveTime> {
    let s = s.strip_prefix("PT")?;

    let mut hours: u32 = 0;
    let mut minutes: u32 = 0;
    let mut seconds: u32 = 0;
    let mut current_num = String::new();

    for ch in s.chars() {
        match ch {
            '0'..='9' | '.' => current_num.push(ch),
            'H' => {
                hours = current_num.parse().unwrap_or(0);
                current_num.clear();
            }
            'M' => {
                minutes = current_num.parse().unwrap_or(0);
                current_num.clear();
            }
            'S' => {
                seconds = current_num.parse::<f64>().unwrap_or(0.0) as u32;
                current_num.clear();
            }
            _ => {}
        }
    }

    chrono::NaiveTime::from_hms_opt(hours, minutes, seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_ods_float() {
        let val = resolve_ods_cell("42", Some("float"), Some("42"));
        assert_eq!(val.as_int(), Some(42));

        let val = resolve_ods_cell("3.14", Some("float"), Some("3.14"));
        assert_eq!(val.as_float(), Some(3.14));
    }

    #[test]
    fn test_resolve_ods_string() {
        let val = resolve_ods_cell("Hello", Some("string"), None);
        assert_eq!(val.as_str(), Some("Hello"));
    }

    #[test]
    fn test_resolve_ods_boolean() {
        let val = resolve_ods_cell("", Some("boolean"), Some("true"));
        assert_eq!(val.as_bool(), Some(true));
    }

    #[test]
    fn test_resolve_ods_date() {
        let val = resolve_ods_cell("2024-01-15", Some("date"), Some("2024-01-15"));
        assert!(matches!(val, CellValue::Date(_)));
    }

    #[test]
    fn test_parse_ods_duration() {
        let time = parse_ods_duration("PT12H30M00S").unwrap();
        assert_eq!(time, chrono::NaiveTime::from_hms_opt(12, 30, 0).unwrap());

        let time = parse_ods_duration("PT1H0M5S").unwrap();
        assert_eq!(time, chrono::NaiveTime::from_hms_opt(1, 0, 5).unwrap());
    }

    #[test]
    fn test_resolve_ods_null() {
        let val = resolve_ods_cell("", None, None);
        assert!(val.is_null());
    }
}
