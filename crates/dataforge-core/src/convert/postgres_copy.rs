// =============================================================================
// DataForge Core — Postgres COPY Streaming Adapter
// =============================================================================
// Standard PostgreSQL COPY FROM STDIN protocol serializer.
// =============================================================================

use crate::types::{CellValue, Row, RowBatch};
use crate::error::{DataForgeError, Result};
use std::io::Write;
use std::fmt::Write as _;

/// A streaming writer that formats and writes RowBatches to a target output stream
/// using the PostgreSQL COPY FROM STDIN protocol.
pub struct PostgresCopyWriter<W: Write> {
    writer: W,
    table_name: String,
    headers: Option<Vec<String>>,
    header_written: bool,
}

impl<W: Write> PostgresCopyWriter<W> {
    /// Create a new PostgresCopyWriter wrapping a target output stream.
    pub fn new(writer: W, table_name: &str) -> Self {
        PostgresCopyWriter {
            writer,
            table_name: table_name.to_string(),
            headers: None,
            header_written: false,
        }
    }

    /// Pre-configure headers. If not set, headers will be inferred from the first RowBatch.
    pub fn with_headers(mut self, headers: Vec<String>) -> Self {
        self.headers = Some(headers);
        self
    }

    fn write_init(&mut self, headers: &[String]) -> Result<()> {
        if headers.is_empty() {
            return Err(DataForgeError::config("Postgres COPY requires at least one column header"));
        }
        let mut copy_stmt = String::new();
        write!(copy_stmt, "COPY {} (", self.table_name).unwrap();
        for (i, h) in headers.iter().enumerate() {
            if i > 0 {
                copy_stmt.push_str(", ");
            }
            copy_stmt.push_str(h);
        }
        copy_stmt.push_str(") FROM STDIN WITH (FORMAT csv, HEADER false, NULL 'NULL');\n");
        self.writer.write_all(copy_stmt.as_bytes())?;
        self.header_written = true;
        Ok(())
    }

    /// Write a single row to the output stream.
    pub fn write_row(&mut self, row: &Row, headers_fallback: &[String]) -> Result<()> {
        if !self.header_written {
            let hdrs = if let Some(ref h) = self.headers {
                h.clone()
            } else {
                headers_fallback.to_vec()
            };
            self.write_init(&hdrs)?;
        }

        let mut line = String::new();
        for (c_idx, cell) in row.cells.iter().enumerate() {
            if c_idx > 0 {
                line.push(',');
            }
            match cell {
                CellValue::Null => line.push_str("NULL"),
                CellValue::Bool(b) => line.push_str(if *b { "true" } else { "false" }),
                CellValue::Int(i) => write!(line, "{}", i).unwrap(),
                CellValue::Float(f) => write!(line, "{}", f).unwrap(),
                CellValue::String(s) => {
                    let needs_quotes = s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r');
                    if needs_quotes {
                        line.push('"');
                        line.push_str(&s.replace('"', "\"\""));
                        line.push('"');
                    } else {
                        line.push_str(s);
                    }
                }
                CellValue::Bytes(b) => {
                    line.push_str("\\x");
                    for byte in b {
                        write!(line, "{:02x}", byte).unwrap();
                    }
                }
                CellValue::DateTime(dt) => {
                    write!(line, "{}", dt.format("%Y-%m-%d %H:%M:%S")).unwrap();
                }
                CellValue::Date(d) => {
                    write!(line, "{}", d.format("%Y-%m-%d")).unwrap();
                }
                CellValue::Time(t) => {
                    write!(line, "{}", t.format("%H:%M:%S")).unwrap();
                }
                _ => line.push_str("NULL"),
            }
        }
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        Ok(())
    }

    /// Write a full RowBatch to the output stream.
    pub fn write_batch(&mut self, batch: &RowBatch) -> Result<()> {
        if batch.rows.is_empty() {
            return Ok(());
        }

        let fallback = batch.headers.clone().unwrap_or_default();
        for row in &batch.rows {
            self.write_row(row, &fallback)?;
        }
        Ok(())
    }

    /// Finalize the copy payload by sending the standard termination sequence `\.` and flushing the stream.
    pub fn finish(mut self) -> Result<W> {
        self.writer.write_all(b"\\.\n")?;
        self.writer.flush()?;
        Ok(self.writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CellValue;

    #[test]
    fn test_postgres_copy_writer() {
        let mut buffer = Vec::new();
        let mut writer = PostgresCopyWriter::new(&mut buffer, "users")
            .with_headers(vec!["name".to_string(), "age".to_string()]);

        let mut row = Row::new(0);
        row.push(CellValue::from("Alice"));
        row.push(CellValue::from(30_i64));
        writer.write_row(&row, &[]).unwrap();

        let mut row2 = Row::new(1);
        row2.push(CellValue::from("Bob"));
        row2.push(CellValue::from(25_i64));
        writer.write_row(&row2, &[]).unwrap();

        let _ = writer.finish().unwrap();

        let result = String::from_utf8(buffer).unwrap();
        assert_eq!(
            result,
            "COPY users (name, age) FROM STDIN WITH (FORMAT csv, HEADER false, NULL 'NULL');\n\
             Alice,30\n\
             Bob,25\n\
             \\.\n"
        );
    }
}
