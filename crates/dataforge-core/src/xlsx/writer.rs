// =============================================================================
// DataForge Core — Streaming XLSX Writer
// =============================================================================
// Builds an XLSX file incrementally by writing worksheet XML in a streaming
// fashion and constructing the ZIP archive on-the-fly.
//
// The writer never holds a complete DOM — it writes XML elements directly
// to the ZIP entry as rows are added, keeping memory usage proportional
// to the buffer size, not the total row count.
// =============================================================================

use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::Path;

use chrono::Timelike;
use tracing::{debug, info};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::config::WriterConfig;
use crate::error::{DataForgeError, Result};
use crate::types::{CellValue, Row, RowBatch};

/// Streaming XLSX writer that constructs an Excel file incrementally.
///
/// # Architecture
/// 1. Create a ZIP archive writer
/// 2. Write initial XML boilerplate ([Content_Types].xml, rels, workbook.xml)
/// 3. Begin the worksheet XML entry in the ZIP
/// 4. For each row/batch, write `<row>` elements directly to the ZIP stream
/// 5. On `finish()`, close the worksheet XML and finalize the ZIP
///
/// # Example
/// ```no_run
/// use dataforge_core::xlsx::XlsxWriter;
/// use dataforge_core::config::WriterConfig;
/// use dataforge_core::types::{Row, CellValue};
///
/// let config = WriterConfig::default()
///     .with_headers(vec!["Name".into(), "Age".into()]);
///
/// let mut writer = XlsxWriter::create("output.xlsx", config).unwrap();
///
/// let mut row = Row::new(0);
/// row.push(CellValue::from("Alice"));
/// row.push(CellValue::from(30_i64));
/// writer.write_row(&row).unwrap();
///
/// writer.finish().unwrap();
/// ```
pub struct XlsxWriter<W: Write + Seek> {
    /// ZIP archive writer
    zip: ZipWriter<W>,

    /// Configuration
    config: WriterConfig,

    /// Shared strings table (built as rows are written)
    shared_strings: Vec<String>,

    /// Map from string → index for deduplication
    string_index: std::collections::HashMap<String, usize>,

    /// Number of data rows written
    rows_written: u64,

    /// Current row number in the worksheet (1-based for XLSX)
    current_row_num: u32,

    /// Buffer for accumulating the worksheet XML
    worksheet_xml: Vec<u8>,

    /// Whether the worksheet has been started
    worksheet_started: bool,
}

impl XlsxWriter<BufWriter<File>> {
    /// Create a new XLSX writer that writes to a file.
    pub fn create(path: impl AsRef<Path>, config: WriterConfig) -> Result<Self> {
        let path = path.as_ref();
        let file = File::create(path).map_err(|e| {
            DataForgeError::io(e, format!("Failed to create XLSX file '{}'", path.display()))
        })?;
        let buf_writer = BufWriter::with_capacity(64 * 1024, file);

        info!(path = %path.display(), "Creating XLSX output file");

        Self::new(buf_writer, config)
    }
}

impl<W: Write + Seek> XlsxWriter<W> {
    /// Create a new XLSX writer wrapping any Write + Seek implementation.
    pub fn new(inner: W, config: WriterConfig) -> Result<Self> {
        let zip = ZipWriter::new(inner);

        let mut writer = XlsxWriter {
            zip,
            config,
            shared_strings: Vec::new(),
            string_index: std::collections::HashMap::new(),
            rows_written: 0,
            current_row_num: 0,
            worksheet_xml: Vec::with_capacity(1024 * 1024), // 1MB initial buffer
            worksheet_started: false,
        };

        // Write the worksheet XML header
        writer.start_worksheet()?;

        // Write headers if provided
        if writer.config.headers.is_some() {
            writer.write_header_row()?;
        }

        Ok(writer)
    }

    /// Start the worksheet XML.
    fn start_worksheet(&mut self) -> Result<()> {
        // Write the worksheet XML preamble
        self.worksheet_xml
            .extend_from_slice(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
        self.worksheet_xml.extend_from_slice(
            b"<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\n",
        );
        self.worksheet_xml
            .extend_from_slice(b"<sheetData>\n");
        self.worksheet_started = true;
        Ok(())
    }

    /// Write the header row.
    fn write_header_row(&mut self) -> Result<()> {
        if let Some(headers) = self.config.headers.clone() {
            self.current_row_num += 1;
            let row_num = self.current_row_num;

            self.worksheet_xml
                .extend_from_slice(format!("<row r=\"{row_num}\">\n").as_bytes());

            for (col_idx, header) in headers.iter().enumerate() {
                let col_letter = column_index_to_letter(col_idx);
                let cell_ref = format!("{col_letter}{row_num}");
                let string_idx = self.add_shared_string(header);

                // Write cell with shared string type
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" t=\"s\"><v>{string_idx}</v></c>\n").as_bytes(),
                );
            }

            self.worksheet_xml.extend_from_slice(b"</row>\n");

            debug!(num_columns = headers.len(), "XLSX header row written");
        }
        Ok(())
    }

    /// Write a single row to the XLSX output.
    pub fn write_row(&mut self, row: &Row) -> Result<()> {
        self.current_row_num += 1;
        let row_num = self.current_row_num;

        self.worksheet_xml
            .extend_from_slice(format!("<row r=\"{row_num}\">\n").as_bytes());

        for (col_idx, cell) in row.cells.iter().enumerate() {
            let col_letter = column_index_to_letter(col_idx);
            let cell_ref = format!("{col_letter}{row_num}");

            self.write_cell(&cell_ref, cell)?;
        }

        self.worksheet_xml.extend_from_slice(b"</row>\n");
        self.rows_written += 1;

        Ok(())
    }

    /// Write an entire batch of rows.
    pub fn write_batch(&mut self, batch: &RowBatch) -> Result<()> {
        // Write headers from batch if we haven't written any yet
        if self.current_row_num == 0 {
            if let Some(headers) = &batch.headers {
                self.config.headers = Some(headers.clone());
                self.write_header_row()?;
            }
        }

        for row in &batch.rows {
            self.write_row(row)?;
        }

        Ok(())
    }

    /// Write a single cell element to the worksheet XML.
    fn write_cell(&mut self, cell_ref: &str, value: &CellValue) -> Result<()> {
        match value {
            CellValue::Null => {
                // Skip null cells (sparse representation)
            }
            CellValue::Bool(v) => {
                let val = if *v { "1" } else { "0" };
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" t=\"b\"><v>{val}</v></c>\n").as_bytes(),
                );
            }
            CellValue::Int(v) => {
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\"><v>{v}</v></c>\n").as_bytes(),
                );
            }
            CellValue::Float(v) => {
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\"><v>{v}</v></c>\n").as_bytes(),
                );
            }
            CellValue::String(s) => {
                let idx = self.add_shared_string(s.as_str());
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" t=\"s\"><v>{idx}</v></c>\n").as_bytes(),
                );
            }
            CellValue::DateTime(dt) => {
                // Convert to Excel serial number
                let serial = datetime_to_serial(dt);
                // Style index 1 = date format (we'll define this in styles.xml)
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" s=\"1\"><v>{serial}</v></c>\n").as_bytes(),
                );
            }
            CellValue::Date(d) => {
                let dt = d.and_hms_opt(0, 0, 0).unwrap();
                let serial = datetime_to_serial(&dt);
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" s=\"1\"><v>{serial}</v></c>\n").as_bytes(),
                );
            }
            CellValue::Time(t) => {
                // Time-only as fraction of a day
                let serial = t.num_seconds_from_midnight() as f64 / 86400.0;
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" s=\"2\"><v>{serial}</v></c>\n").as_bytes(),
                );
            }
            CellValue::Duration(secs) => {
                let idx = self.add_shared_string(&format!("{secs}s"));
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" t=\"s\"><v>{idx}</v></c>\n").as_bytes(),
                );
            }
            CellValue::Error(e) => {
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" t=\"e\"><v>{e}</v></c>\n").as_bytes(),
                );
            }
            CellValue::Bytes(b) => {
                let idx = self.add_shared_string(&format!("<{} bytes>", b.len()));
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" t=\"s\"><v>{idx}</v></c>\n").as_bytes(),
                );
            }
        }
        Ok(())
    }

    /// Add a string to the shared strings table, returning its index.
    /// Deduplicates strings to minimize file size.
    fn add_shared_string(&mut self, s: &str) -> usize {
        if let Some(&idx) = self.string_index.get(s) {
            return idx;
        }
        let idx = self.shared_strings.len();
        self.shared_strings.push(s.to_string());
        self.string_index.insert(s.to_string(), idx);
        idx
    }

    /// Finalize the XLSX file, writing all remaining XML and closing the ZIP.
    ///
    /// This MUST be called to produce a valid XLSX file. If not called,
    /// the output will be an incomplete/corrupt ZIP archive.
    ///
    /// # Returns
    /// The number of data rows written (excluding header).
    pub fn finish(mut self) -> Result<u64> {
        // Close the worksheet XML
        self.worksheet_xml.extend_from_slice(b"</sheetData>\n");

        // Add freeze pane for header row if configured
        if self.config.xlsx.freeze_header && self.config.headers.is_some() {
            self.worksheet_xml.extend_from_slice(
                b"<sheetViews><sheetView tabSelected=\"1\" workbookViewId=\"0\">\
                  <pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/>\
                  </sheetView></sheetViews>\n",
            );
        }

        self.worksheet_xml.extend_from_slice(b"</worksheet>\n");

        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(6));

        // Write [Content_Types].xml
        self.zip
            .start_file("[Content_Types].xml", options)?;
        self.zip.write_all(CONTENT_TYPES_XML)?;

        // Write _rels/.rels
        self.zip
            .start_file("_rels/.rels", options)?;
        self.zip.write_all(RELS_XML)?;

        // Write xl/_rels/workbook.xml.rels
        self.zip
            .start_file("xl/_rels/workbook.xml.rels", options)?;
        self.zip.write_all(WORKBOOK_RELS_XML)?;

        // Write xl/workbook.xml
        let sheet_name = &self.config.xlsx.sheet_name;
        let workbook_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="{sheet_name}" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#
        );
        self.zip
            .start_file("xl/workbook.xml", options)?;
        self.zip.write_all(workbook_xml.as_bytes())?;

        // Write xl/styles.xml (with date format)
        self.zip
            .start_file("xl/styles.xml", options)?;
        self.zip.write_all(STYLES_XML)?;

        // Write xl/sharedStrings.xml
        self.zip
            .start_file("xl/sharedStrings.xml", options)?;
        let sst_count = self.shared_strings.len();
        write!(
            self.zip,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="{sst_count}" uniqueCount="{sst_count}">"#
        )?;
        for s in &self.shared_strings {
            let escaped = xml_escape(s);
            write!(self.zip, "<si><t>{escaped}</t></si>")?;
        }
        write!(self.zip, "</sst>")?;

        // Write xl/worksheets/sheet1.xml
        self.zip
            .start_file("xl/worksheets/sheet1.xml", options)?;
        self.zip.write_all(&self.worksheet_xml)?;

        // Finalize the ZIP archive
        self.zip.finish()?;

        info!(rows_written = self.rows_written, shared_strings = sst_count, "XLSX writing complete");
        Ok(self.rows_written)
    }

    /// Get the number of data rows written so far.
    pub fn rows_written(&self) -> u64 {
        self.rows_written
    }
}

// =============================================================================
// Helper functions
// =============================================================================

/// Convert a 0-based column index to an Excel column letter(s).
///
/// Examples: 0 → "A", 1 → "B", 25 → "Z", 26 → "AA", 27 → "AB"
fn column_index_to_letter(index: usize) -> String {
    let mut result = String::new();
    let mut n = index;

    loop {
        let remainder = n % 26;
        result.insert(0, (b'A' + remainder as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }

    result
}

/// Convert a NaiveDateTime to an Excel serial date number (1900 system).
fn datetime_to_serial(dt: &chrono::NaiveDateTime) -> f64 {
    let base = chrono::NaiveDate::from_ymd_opt(1899, 12, 30).unwrap();
    let days = dt.date().signed_duration_since(base).num_days() as f64;
    let time_frac = dt.time().num_seconds_from_midnight() as f64 / 86400.0;
    days + time_frac
}

/// Escape special XML characters in a string.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// =============================================================================
// Static XML templates for the XLSX package structure
// =============================================================================

const CONTENT_TYPES_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
</Types>"#;

const RELS_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK_RELS_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>
</Relationships>"#;

const STYLES_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<numFmts count="2">
<numFmt numFmtId="164" formatCode="yyyy-mm-dd hh:mm:ss"/>
<numFmt numFmtId="165" formatCode="hh:mm:ss"/>
</numFmts>
<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>
<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
<cellXfs count="3">
<xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>
<xf numFmtId="164" fontId="0" fillId="0" borderId="0" applyNumberFormat="1"/>
<xf numFmtId="165" fontId="0" fillId="0" borderId="0" applyNumberFormat="1"/>
</cellXfs>
</styleSheet>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_column_index_to_letter() {
        assert_eq!(column_index_to_letter(0), "A");
        assert_eq!(column_index_to_letter(1), "B");
        assert_eq!(column_index_to_letter(25), "Z");
        assert_eq!(column_index_to_letter(26), "AA");
        assert_eq!(column_index_to_letter(27), "AB");
        assert_eq!(column_index_to_letter(51), "AZ");
        assert_eq!(column_index_to_letter(52), "BA");
        assert_eq!(column_index_to_letter(701), "ZZ");
    }

    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape("hello"), "hello");
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape("\"quoted\""), "&quot;quoted&quot;");
    }

    #[test]
    fn test_datetime_to_serial() {
        use chrono::NaiveDate;

        // January 1, 2024 at midnight
        let dt = NaiveDate::from_ymd_opt(2024, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let serial = datetime_to_serial(&dt);
        assert!((serial - 45292.0).abs() < 0.001);
    }

    #[test]
    fn test_xlsx_write_to_buffer() {
        let config = WriterConfig::default()
            .with_headers(vec!["Name".into(), "Value".into()]);

        let buffer = Cursor::new(Vec::new());
        let mut writer = XlsxWriter::new(buffer, config).unwrap();

        let mut row = Row::new(0);
        row.push(CellValue::from("test"));
        row.push(CellValue::from(42_i64));
        writer.write_row(&row).unwrap();

        let rows = writer.finish().unwrap();
        assert_eq!(rows, 1);
    }
}
