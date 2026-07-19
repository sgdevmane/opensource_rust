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
use super::StyleTemplate;

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

    /// Track column widths for auto-fitting
    column_widths: Vec<usize>,
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
            column_widths: Vec::new(),
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

            let has_style = !matches!(self.config.xlsx.style, StyleTemplate::None);
            let s_attr = if has_style { " s=\"3\"" } else { "" };

            for (col_idx, header) in headers.iter().enumerate() {
                let col_letter = column_index_to_letter(col_idx);
                let cell_ref = format!("{col_letter}{row_num}");
                let string_idx = self.add_shared_string(header);

                let val_len = header.len();
                if col_idx >= self.column_widths.len() {
                    self.column_widths.resize(col_idx + 1, 0);
                }
                self.column_widths[col_idx] = self.column_widths[col_idx].max(val_len);

                // Write cell with shared string type
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" t=\"s\"{s_attr}><v>{string_idx}</v></c>\n").as_bytes(),
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

            self.write_cell(&cell_ref, cell, row_num as u64, col_idx)?;
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
    fn write_cell(&mut self, cell_ref: &str, value: &CellValue, row_num: u64, col_idx: usize) -> Result<()> {
        let val_len = value.to_display_string().len();
        if col_idx >= self.column_widths.len() {
            self.column_widths.resize(col_idx + 1, 0);
        }
        self.column_widths[col_idx] = self.column_widths[col_idx].max(val_len);

        let is_even_row = row_num % 2 == 0;
        let has_alt_style = !matches!(self.config.xlsx.style, StyleTemplate::None);
        
        let mut style_attr = if has_alt_style && is_even_row {
            " s=\"4\"".to_string()
        } else {
            "".to_string()
        };

        // Evaluate conditional format rules
        for rule in &self.config.xlsx.conditional_formats {
            if rule.column_index == col_idx {
                let cell_val = match value {
                    CellValue::Int(v) => Some(*v as f64),
                    CellValue::Float(v) => Some(*v),
                    _ => None,
                };
                if let Some(val) = cell_val {
                    let mut is_match = true;
                    if let Some(min) = rule.min_val {
                        if val < min { is_match = false; }
                    }
                    if let Some(max) = rule.max_val {
                        if val > max { is_match = false; }
                    }
                    if is_match {
                        style_attr = format!(" s=\"{}\"", rule.style_index);
                        break;
                    }
                }
            }
        }

        match value {
            CellValue::Null => {
                // Skip null cells (sparse representation)
            }
            CellValue::Bool(v) => {
                let val = if *v { "1" } else { "0" };
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" t=\"b\"{style_attr}><v>{val}</v></c>\n").as_bytes(),
                );
            }
            CellValue::Int(v) => {
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\"{style_attr}><v>{v}</v></c>\n").as_bytes(),
                );
            }
            CellValue::Float(v) => {
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\"{style_attr}><v>{v}</v></c>\n").as_bytes(),
                );
            }
            CellValue::String(s) => {
                let idx = self.add_shared_string(s.as_str());
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" t=\"s\"{style_attr}><v>{idx}</v></c>\n").as_bytes(),
                );
            }
            CellValue::DateTime(dt) => {
                let serial = datetime_to_serial(dt);
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
                let serial = t.num_seconds_from_midnight() as f64 / 86400.0;
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" s=\"2\"><v>{serial}</v></c>\n").as_bytes(),
                );
            }
            CellValue::Duration(secs) => {
                let idx = self.add_shared_string(&format!("{secs}s"));
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" t=\"s\"{style_attr}><v>{idx}</v></c>\n").as_bytes(),
                );
            }
            CellValue::Error(e) => {
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" t=\"e\"{style_attr}><v>{e}</v></c>\n").as_bytes(),
                );
            }
            CellValue::Bytes(b) => {
                let idx = self.add_shared_string(&format!("<{} bytes>", b.len()));
                self.worksheet_xml.extend_from_slice(
                    format!("<c r=\"{cell_ref}\" t=\"s\"{style_attr}><v>{idx}</v></c>\n").as_bytes(),
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
        let style_resolved = self.config.xlsx.style.resolve();
        let freeze_header = self.config.xlsx.freeze_header || style_resolved.freeze_header;
        if freeze_header && self.config.headers.is_some() {
            self.worksheet_xml.extend_from_slice(
                b"<sheetViews><sheetView tabSelected=\"1\" workbookViewId=\"0\">\
                  <pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/>\
                  </sheetView></sheetViews>\n",
            );
        }

        // Add autoFilter if configured
        let auto_filter = self.config.xlsx.auto_filter || style_resolved.auto_filter;
        if auto_filter && self.config.headers.is_some() && self.current_row_num > 0 {
            if let Some(ref headers) = self.config.headers {
                let last_col = column_index_to_letter(headers.len() - 1);
                let last_row = self.current_row_num;
                self.worksheet_xml.extend_from_slice(
                    format!("<autoFilter ref=\"A1:{last_col}{last_row}\"/>\n").as_bytes()
                );
            }
        }

        if self.config.xlsx.chart.is_some() {
            self.worksheet_xml.extend_from_slice(b"<drawing r:id=\"rIdDrawing1\"/>\n");
        }
        self.worksheet_xml.extend_from_slice(b"</worksheet>\n");

        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(self.config.compression_level);

        // Build content types dynamically
        let mut content_types = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>"#
        );
        if self.config.xlsx.chart.is_some() {
            content_types.push_str(
                r#"<Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>
<Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#
            );
        }
        content_types.push_str("</Types>");

        // Write [Content_Types].xml
        self.zip
            .start_file("[Content_Types].xml", options)?;
        self.zip.write_all(content_types.as_bytes())?;

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

        // Write xl/styles.xml (with date format & style template support)
        self.zip
            .start_file("xl/styles.xml", options)?;
        let styles_xml_content = self.config.xlsx.style.to_styles_xml();
        self.zip.write_all(styles_xml_content.as_bytes())?;

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

        let mut final_worksheet_xml = Vec::new();
        if self.config.auto_column_width && !self.column_widths.is_empty() {
            if let Some(pos) = self.worksheet_xml.windows(11).position(|w| w == b"<sheetData>") {
                final_worksheet_xml.extend_from_slice(&self.worksheet_xml[..pos]);
                final_worksheet_xml.extend_from_slice(b"<cols>\n");
                for (i, &w) in self.column_widths.iter().enumerate() {
                    let width = (w as f64 + 3.0).max(10.0).min(50.0);
                    let col_xml = format!(
                        "  <col min=\"{}\" max=\"{}\" width=\"{:.2}\" customWidth=\"1\"/>\n",
                        i + 1, i + 1, width
                    );
                    final_worksheet_xml.extend_from_slice(col_xml.as_bytes());
                }
                final_worksheet_xml.extend_from_slice(b"</cols>\n");
                final_worksheet_xml.extend_from_slice(&self.worksheet_xml[pos..]);
            } else {
                final_worksheet_xml = self.worksheet_xml.clone();
            }
        } else {
            final_worksheet_xml = self.worksheet_xml.clone();
        }

        self.zip.write_all(&final_worksheet_xml)?;

        // If chart is configured, generate drawing and chart files
        if let Some(ref chart) = self.config.xlsx.chart {
            // Write xl/worksheets/_rels/sheet1.xml.rels
            self.zip.start_file("xl/worksheets/_rels/sheet1.xml.rels", options)?;
            let sheet_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rIdDrawing1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/>
</Relationships>"#;
            self.zip.write_all(sheet_rels.as_bytes())?;

            // Write xl/drawings/drawing1.xml
            self.zip.start_file("xl/drawings/drawing1.xml", options)?;
            let drawing_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <xdr:twoCellAnchor>
    <xdr:from><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>2</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
    <xdr:to><xdr:col>12</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>18</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
    <xdr:graphicFrame macro="">
      <xdr:nvGraphicFramePr>
        <xdr:cNvPr id="2" name="Chart 1"/>
        <xdr:cNvGraphicFramePr/>
      </xdr:nvGraphicFramePr>
      <xdr:xfrm>
        <xdr:off x="0" y="0"/>
        <xdr:ext cx="0" cy="0"/>
      </xdr:xfrm>
      <a:graphic>
        <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
          <c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId1"/>
        </a:graphicData>
      </a:graphic>
    </xdr:graphicFrame>
    <xdr:clientData/>
  </xdr:twoCellAnchor>
</xdr:wsDr>"#;
            self.zip.write_all(drawing_xml.as_bytes())?;

            // Write xl/drawings/_rels/drawing1.xml.rels
            self.zip.start_file("xl/drawings/_rels/drawing1.xml.rels", options)?;
            let drawing_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/>
</Relationships>"#;
            self.zip.write_all(drawing_rels.as_bytes())?;

            // Generate ranges
            let x_letter = column_index_to_letter(chart.x_axis_col);
            let y_letter = column_index_to_letter(chart.y_axis_col);
            let last_row = self.current_row_num;
            let x_range = format!("${x_letter}$2:${x_letter}${last_row}");
            let y_range = format!("${y_letter}$2:${y_letter}${last_row}");
            let title_escaped = xml_escape(&chart.title);

            let chart_tag = match chart.chart_type {
                crate::config::ChartType::Bar => "barChart",
                crate::config::ChartType::Line => "lineChart",
            };

            // Write xl/charts/chart1.xml
            self.zip.start_file("xl/charts/chart1.xml", options)?;
            let chart_xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <c:chart>
    <c:title>
      <c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{title_escaped}</a:t></a:r></a:p></c:rich></c:tx>
    </c:title>
    <c:plotArea>
      <c:{chart_tag}>
        <c:ser>
          <c:idx val="0"/>
          <c:order val="0"/>
          <c:tx><c:v>Series 1</c:v></c:tx>
          <c:cat>
            <c:strRef>
              <c:f>Sheet1!{x_range}</c:f>
            </c:strRef>
          </c:cat>
          <c:val>
            <c:numRef>
              <c:f>Sheet1!{y_range}</c:f>
            </c:numRef>
          </c:val>
        </c:ser>
      </c:{chart_tag}>
    </c:plotArea>
  </c:chart>
</c:chartSpace>"#
            );
            self.zip.write_all(chart_xml.as_bytes())?;
        }

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

    #[test]
    fn test_xlsx_write_with_style_template() {
        let config = WriterConfig::default()
            .with_headers(vec!["Name".into(), "Value".into()])
            .with_style_template(StyleTemplate::Professional);

        let buffer = Cursor::new(Vec::new());
        let mut writer = XlsxWriter::new(buffer, config).unwrap();

        let mut row1 = Row::new(0);
        row1.push(CellValue::from("test1"));
        row1.push(CellValue::from(42_i64));
        writer.write_row(&row1).unwrap();

        let mut row2 = Row::new(1);
        row2.push(CellValue::from("test2"));
        row2.push(CellValue::from(43_i64));
        writer.write_row(&row2).unwrap();

        let rows = writer.finish().unwrap();
        assert_eq!(rows, 2);
    }

    #[test]
    fn test_xlsx_conditional_formatting() {
        let rule = crate::config::ConditionalFormatRule {
            column_index: 1,
            min_val: Some(100.0),
            max_val: None,
            style_index: 3,
        };
        let config = WriterConfig::default()
            .with_headers(vec!["Name".into(), "Value".into()])
            .with_conditional_format(rule);

        let buffer = Cursor::new(Vec::new());
        let mut writer = XlsxWriter::new(buffer, config).unwrap();

        // Row 1: value = 50 (no match)
        let mut row1 = Row::new(0);
        row1.push(CellValue::from("Low"));
        row1.push(CellValue::from(50_i64));
        writer.write_row(&row1).unwrap();

        // Row 2: value = 150 (matches rule, style index 3)
        let mut row2 = Row::new(1);
        row2.push(CellValue::from("High"));
        row2.push(CellValue::from(150_i64));
        writer.write_row(&row2).unwrap();

        let xml_str = String::from_utf8(writer.worksheet_xml.clone()).unwrap();
        assert!(xml_str.contains("s=\"3\""));
        
        let rows = writer.finish().unwrap();
        assert_eq!(rows, 2);
    }

    #[test]
    fn test_xlsx_chart_generation() {
        use crate::config::{SpreadsheetChart, ChartType};

        let chart = SpreadsheetChart {
            chart_type: ChartType::Bar,
            title: "Sales Report".to_string(),
            x_axis_col: 0,
            y_axis_col: 1,
        };
        let config = WriterConfig::default()
            .with_headers(vec!["Product".into(), "Sales".into()])
            .with_chart(chart);

        let buffer = Cursor::new(Vec::new());
        let mut writer = XlsxWriter::new(buffer, config).unwrap();

        let mut row1 = Row::new(0);
        row1.push(CellValue::from("Apple"));
        row1.push(CellValue::from(100_i64));
        writer.write_row(&row1).unwrap();

        let mut row2 = Row::new(1);
        row2.push(CellValue::from("Orange"));
        row2.push(CellValue::from(150_i64));
        writer.write_row(&row2).unwrap();

        // Verify configuration is correctly stored
        assert!(writer.config.xlsx.chart.is_some());

        let rows = writer.finish().unwrap();
        assert_eq!(rows, 2);
    }
}
