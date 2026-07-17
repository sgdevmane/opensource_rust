// =============================================================================
// DataForge Core — JSON / JSONL Reader
// =============================================================================
// Streaming reader for JSON Lines (JSONL) and line-separated JSON Arrays.
// Parses rows incrementally with constant memory footprint.
// =============================================================================

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use crate::config::ReaderConfig;
use crate::error::{DataForgeError, Result};
use crate::types::{CellValue, Row, RowBatch};

/// Streaming JSON/JSONL reader.
pub struct JsonReader<R: Read> {
    reader: BufReader<R>,
    config: ReaderConfig,
    headers: Option<Vec<String>>,
    row_index: u64,
    line_buf: String,
    exhausted: bool,
}

impl JsonReader<File> {
    /// Open a JSON/JSONL file for streaming.
    pub fn open(path: impl AsRef<Path>, config: ReaderConfig) -> Result<Self> {
        let file = File::open(path).map_err(|e| {
            DataForgeError::io(e, "Failed to open JSON/JSONL file")
        })?;
        Ok(Self::new(file, config))
    }
}

impl<R: Read> JsonReader<R> {
    /// Wrap any standard reader with a streaming JsonReader.
    pub fn new(reader: R, config: ReaderConfig) -> Self {
        JsonReader {
            reader: BufReader::new(reader),
            headers: config.column_names.clone(),
            config,
            row_index: 0,
            line_buf: String::new(),
            exhausted: false,
        }
    }

    /// Retrieve inferred or configured column headers.
    pub fn headers(&self) -> Option<&Vec<String>> {
        self.headers.as_ref()
    }

    fn value_to_row(&mut self, val: serde_json::Value) -> Result<Row> {
        let obj = match val {
            serde_json::Value::Object(map) => map,
            _ => return Err(DataForgeError::config("JSON line must be a JSON object")),
        };

        // Infer headers if not configured or set
        if self.headers.is_none() {
            let keys: Vec<String> = obj.keys().cloned().collect();
            self.headers = Some(keys);
        }

        let headers = self.headers.as_ref().unwrap();
        let mut row = Row::with_capacity(self.row_index, headers.len());

        for header in headers {
            if let Some(json_val) = obj.get(header) {
                let cell_val = match json_val {
                    serde_json::Value::Null => CellValue::Null,
                    serde_json::Value::Bool(b) => CellValue::Bool(*b),
                    serde_json::Value::Number(num) => {
                        if let Some(i) = num.as_i64() {
                            CellValue::Int(i)
                        } else if let Some(f) = num.as_f64() {
                            CellValue::Float(f)
                        } else {
                            CellValue::Null
                        }
                    }
                    serde_json::Value::String(s) => CellValue::String(s.clone().into()),
                    _ => CellValue::String(json_val.to_string().into()),
                };
                row.push(cell_val);
            } else {
                row.push(CellValue::Null);
            }
        }

        Ok(row)
    }
}

impl<R: Read> Iterator for JsonReader<R> {
    type Item = Result<RowBatch>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.exhausted {
            return None;
        }

        let batch_size = self.config.batch_size;
        let mut batch = RowBatch::with_capacity(self.row_index, batch_size);
        batch.headers = self.headers.clone();

        while batch.len() < batch_size {
            self.line_buf.clear();
            match self.reader.read_line(&mut self.line_buf) {
                Ok(0) => {
                    self.exhausted = true;
                    break;
                }
                Ok(_) => {
                    let trimmed = self.line_buf.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed == "[" || trimmed == "]" || trimmed == "]," {
                        continue;
                    }

                    // Handle array wrapping syntax and commas
                    let mut clean_line = trimmed;
                    if clean_line.starts_with('[') {
                        clean_line = &clean_line[1..];
                    }
                    if clean_line.ends_with(',') {
                        clean_line = &clean_line[..clean_line.len() - 1];
                    }
                    if clean_line.ends_with(']') {
                        clean_line = &clean_line[..clean_line.len() - 1];
                    }
                    let clean_line = clean_line.trim();

                    if clean_line.is_empty() {
                        continue;
                    }

                    match serde_json::from_str::<serde_json::Value>(clean_line) {
                        Ok(val) => {
                            match self.value_to_row(val) {
                                Ok(row) => {
                                    batch.push(row);
                                    self.row_index += 1;
                                }
                                Err(e) => return Some(Err(e)),
                            }
                        }
                        Err(e) => return Some(Err(e.into())),
                    }
                }
                Err(e) => return Some(Err(e.into())),
            }
        }

        if batch.headers.is_none() && self.headers.is_some() {
            batch.headers = self.headers.clone();
        }

        if batch.is_empty() {
            None
        } else {
            if self.exhausted {
                batch.is_last = true;
            }
            Some(Ok(batch))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_jsonl_streaming() {
        let jsonl = b"{\"name\": \"Alice\", \"age\": 30}\n{\"name\": \"Bob\", \"age\": 25}\n";
        let config = ReaderConfig::default()
            .with_batch_size(1)
            .with_column_names(vec!["name".to_string(), "age".to_string()]);
        let mut reader = JsonReader::new(Cursor::new(jsonl), config);

        let b1 = reader.next().unwrap().unwrap();
        assert_eq!(b1.len(), 1);
        assert_eq!(b1.rows[0].get_str(0), Some("Alice"));
        assert_eq!(b1.rows[0].get_int(1), Some(30));

        let b2 = reader.next().unwrap().unwrap();
        assert_eq!(b2.len(), 1);
        assert_eq!(b2.rows[0].get_str(0), Some("Bob"));
        assert_eq!(b2.rows[0].get_int(1), Some(25));
    }

    #[test]
    fn test_json_array_streaming() {
        let array = b"[\n  {\"name\": \"Alice\", \"age\": 30},\n  {\"name\": \"Bob\", \"age\": 25}\n]";
        let config = ReaderConfig::default()
            .with_batch_size(2)
            .with_column_names(vec!["name".to_string(), "age".to_string()]);
        let mut reader = JsonReader::new(Cursor::new(array), config);

        let batch = reader.next().unwrap().unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch.rows[0].get_str(0), Some("Alice"));
        assert_eq!(batch.rows[1].get_str(0), Some("Bob"));
    }
}
