// =============================================================================
// DataForge Core — ODS Writer (Placeholder)
// =============================================================================
// Streaming writer for OpenDocument Spreadsheet (.ods) files.
// ODS files are ZIP archives with content.xml containing all data.
// =============================================================================

use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::Path;

use tracing::info;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::config::WriterConfig;
use crate::error::{DataForgeError, Result};
use crate::types::{CellValue, Row, RowBatch};

/// Streaming ODS writer that constructs an ODS file incrementally.
pub struct OdsWriter<W: Write + Seek> {
    /// ZIP archive writer
    zip: ZipWriter<W>,
    /// Configuration
    config: WriterConfig,
    /// Content XML buffer
    content_xml: Vec<u8>,
    /// Rows written
    rows_written: u64,
    /// Current row number
    current_row: u32,
}

impl OdsWriter<BufWriter<File>> {
    /// Create a new ODS writer that writes to a file.
    pub fn create(path: impl AsRef<Path>, config: WriterConfig) -> Result<Self> {
        let path = path.as_ref();
        let file = File::create(path).map_err(|e| {
            DataForgeError::io(e, format!("Failed to create ODS file '{}'", path.display()))
        })?;
        let buf_writer = BufWriter::with_capacity(64 * 1024, file);
        info!(path = %path.display(), "Creating ODS output file");
        Self::new(buf_writer, config)
    }
}

impl<W: Write + Seek> OdsWriter<W> {
    /// Create a new ODS writer wrapping any Write + Seek implementation.
    pub fn new(inner: W, config: WriterConfig) -> Result<Self> {
        let zip = ZipWriter::new(inner);
        let mut writer = OdsWriter {
            zip,
            config,
            content_xml: Vec::with_capacity(1024 * 1024),
            rows_written: 0,
            current_row: 0,
        };

        writer.start_content()?;
        if writer.config.headers.is_some() {
            writer.write_header_row()?;
        }
        Ok(writer)
    }

    /// Start the content XML.
    fn start_content(&mut self) -> Result<()> {
        self.content_xml.extend_from_slice(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
            <office:document-content \
              xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
              xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
              xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" \
              xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
              xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
              office:version=\"1.3\">\n\
            <office:body>\n\
            <office:spreadsheet>\n",
        );

        let sheet_name = xml_escape(&self.config.ods.sheet_name);
        self.content_xml.extend_from_slice(
            format!("<table:table table:name=\"{sheet_name}\">\n").as_bytes(),
        );

        Ok(())
    }

    /// Write header row.
    fn write_header_row(&mut self) -> Result<()> {
        if let Some(headers) = self.config.headers.clone() {
            self.content_xml
                .extend_from_slice(b"<table:table-row>\n");
            for header in &headers {
                let escaped = xml_escape(header);
                self.content_xml.extend_from_slice(
                    format!(
                        "<table:table-cell office:value-type=\"string\">\
                        <text:p>{escaped}</text:p></table:table-cell>\n"
                    )
                    .as_bytes(),
                );
            }
            self.content_xml
                .extend_from_slice(b"</table:table-row>\n");
        }
        Ok(())
    }

    /// Write a single row.
    pub fn write_row(&mut self, row: &Row) -> Result<()> {
        self.content_xml
            .extend_from_slice(b"<table:table-row>\n");

        for cell in &row.cells {
            self.write_cell(cell)?;
        }

        self.content_xml
            .extend_from_slice(b"</table:table-row>\n");
        self.rows_written += 1;
        Ok(())
    }

    /// Write a batch of rows.
    pub fn write_batch(&mut self, batch: &RowBatch) -> Result<()> {
        if self.current_row == 0 {
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

    /// Write a single cell element.
    fn write_cell(&mut self, value: &CellValue) -> Result<()> {
        match value {
            CellValue::Null => {
                self.content_xml
                    .extend_from_slice(b"<table:table-cell/>\n");
            }
            CellValue::Bool(v) => {
                let val = if *v { "true" } else { "false" };
                let display = if *v { "TRUE" } else { "FALSE" };
                self.content_xml.extend_from_slice(
                    format!(
                        "<table:table-cell office:value-type=\"boolean\" office:boolean-value=\"{val}\">\
                        <text:p>{display}</text:p></table:table-cell>\n"
                    ).as_bytes(),
                );
            }
            CellValue::Int(v) => {
                self.content_xml.extend_from_slice(
                    format!(
                        "<table:table-cell office:value-type=\"float\" office:value=\"{v}\">\
                        <text:p>{v}</text:p></table:table-cell>\n"
                    ).as_bytes(),
                );
            }
            CellValue::Float(v) => {
                self.content_xml.extend_from_slice(
                    format!(
                        "<table:table-cell office:value-type=\"float\" office:value=\"{v}\">\
                        <text:p>{v}</text:p></table:table-cell>\n"
                    ).as_bytes(),
                );
            }
            CellValue::String(s) => {
                let escaped = xml_escape(s.as_str());
                self.content_xml.extend_from_slice(
                    format!(
                        "<table:table-cell office:value-type=\"string\">\
                        <text:p>{escaped}</text:p></table:table-cell>\n"
                    ).as_bytes(),
                );
            }
            CellValue::DateTime(dt) => {
                let iso = dt.format("%Y-%m-%dT%H:%M:%S");
                let display = dt.format("%Y-%m-%d %H:%M:%S");
                self.content_xml.extend_from_slice(
                    format!(
                        "<table:table-cell office:value-type=\"date\" office:date-value=\"{iso}\">\
                        <text:p>{display}</text:p></table:table-cell>\n"
                    ).as_bytes(),
                );
            }
            CellValue::Date(d) => {
                let iso = d.format("%Y-%m-%d");
                self.content_xml.extend_from_slice(
                    format!(
                        "<table:table-cell office:value-type=\"date\" office:date-value=\"{iso}\">\
                        <text:p>{iso}</text:p></table:table-cell>\n"
                    ).as_bytes(),
                );
            }
            _ => {
                let display = xml_escape(&value.to_display_string());
                self.content_xml.extend_from_slice(
                    format!(
                        "<table:table-cell office:value-type=\"string\">\
                        <text:p>{display}</text:p></table:table-cell>\n"
                    ).as_bytes(),
                );
            }
        }
        Ok(())
    }

    /// Finalize the ODS file.
    pub fn finish(mut self) -> Result<u64> {
        // Close content XML
        self.content_xml.extend_from_slice(
            b"</table:table>\n\
            </office:spreadsheet>\n\
            </office:body>\n\
            </office:document-content>",
        );

        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        // Write mimetype (must be first, uncompressed)
        let mime_options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        self.zip.start_file("mimetype", mime_options)?;
        self.zip
            .write_all(b"application/vnd.oasis.opendocument.spreadsheet")?;

        // Write META-INF/manifest.xml
        self.zip
            .start_file("META-INF/manifest.xml", options)?;
        self.zip.write_all(ODS_MANIFEST_XML)?;

        // Write content.xml
        self.zip.start_file("content.xml", options)?;
        self.zip.write_all(&self.content_xml)?;

        self.zip.finish()?;

        info!(rows_written = self.rows_written, "ODS writing complete");
        Ok(self.rows_written)
    }

    /// Get the number of data rows written.
    pub fn rows_written(&self) -> u64 {
        self.rows_written
    }
}

/// Escape XML special characters.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const ODS_MANIFEST_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3">
<manifest:file-entry manifest:media-type="application/vnd.oasis.opendocument.spreadsheet" manifest:full-path="/"/>
<manifest:file-entry manifest:media-type="text/xml" manifest:full-path="content.xml"/>
</manifest:manifest>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_ods_write_to_buffer() {
        let config = WriterConfig::default()
            .with_headers(vec!["Name".into(), "Value".into()]);

        let buffer = Cursor::new(Vec::new());
        let mut writer = OdsWriter::new(buffer, config).unwrap();

        let mut row = Row::new(0);
        row.push(CellValue::from("test"));
        row.push(CellValue::from(42_i64));
        writer.write_row(&row).unwrap();

        let rows = writer.finish().unwrap();
        assert_eq!(rows, 1);
    }
}
