// =============================================================================
// DataForge Node.js — napi-rs Bindings
// =============================================================================
// Provides high-performance JS/TS wrappers over dataforge-core.
//
// Key features:
// - Non-blocking native streaming via async generators or iteration
// - Memory-safe transfers using JavaScript class proxies
// - Native conversions (CSV ↔ XLSX) with zero Node.js heap overhead
// =============================================================================

use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{Value, Map, Number};

use dataforge_core::config::{ReaderConfig, WriterConfig};
use dataforge_core::csv::CsvReader;
use dataforge_core::xlsx::XlsxReader;
use dataforge_core::ods::OdsReader;
use dataforge_core::types::CellValue;

/// A lightweight representation of a CellValue for JS.
#[napi(object)]
pub struct JsCell {
    pub r#type: String,
    pub value: Option<String>,
    pub number_value: Option<f64>,
    pub bool_value: Option<bool>,
}

impl From<&CellValue> for JsCell {
    fn from(cell: &CellValue) -> Self {
        match cell {
            CellValue::Null => JsCell {
                r#type: "null".to_string(),
                value: None,
                number_value: None,
                bool_value: None,
            },
            CellValue::Bool(b) => JsCell {
                r#type: "boolean".to_string(),
                value: Some(b.to_string()),
                number_value: None,
                bool_value: Some(*b),
            },
            CellValue::Int(i) => JsCell {
                r#type: "number".to_string(),
                value: Some(i.to_string()),
                number_value: Some(*i as f64),
                bool_value: None,
            },
            CellValue::Float(f) => JsCell {
                r#type: "number".to_string(),
                value: Some(f.to_string()),
                number_value: Some(*f),
                bool_value: None,
            },
            CellValue::String(s) => JsCell {
                r#type: "string".to_string(),
                value: Some(s.to_string()),
                number_value: None,
                bool_value: None,
            },
            CellValue::DateTime(dt) => JsCell {
                r#type: "datetime".to_string(),
                value: Some(dt.to_string()),
                number_value: None,
                bool_value: None,
            },
            CellValue::Date(d) => JsCell {
                r#type: "date".to_string(),
                value: Some(d.to_string()),
                number_value: None,
                bool_value: None,
            },
            CellValue::Time(t) => JsCell {
                r#type: "time".to_string(),
                value: Some(t.to_string()),
                number_value: None,
                bool_value: None,
            },
            _ => JsCell {
                r#type: "string".to_string(),
                value: Some(cell.to_display_string()),
                number_value: None,
                bool_value: None,
            },
        }
    }
}

/// A lightweight representation of a RowBatch for JS.
#[napi]
pub struct JsRowBatch {
    inner: dataforge_core::types::RowBatch,
}

#[napi]
impl JsRowBatch {
    #[napi(getter)]
    pub fn row_count(&self) -> u32 {
        self.inner.len() as u32
    }

    #[napi(getter)]
    pub fn headers(&self) -> Option<Vec<String>> {
        self.inner.headers.clone()
    }

    #[napi]
    pub fn get_row(&self, index: u32) -> Option<Vec<JsCell>> {
        self.inner.rows.get(index as usize).map(|row| {
            row.cells.iter().map(JsCell::from).collect()
        })
    }

    #[napi]
    pub fn to_json_objects(&self) -> Vec<serde_json::Value> {
        let headers = self.inner.headers.as_ref();
        self.inner.rows.iter().map(|row| {
            let mut map = Map::new();
            for (col_idx, cell) in row.cells.iter().enumerate() {
                let col_name = headers
                    .and_then(|h| h.get(col_idx))
                    .map(|s| s.clone())
                    .unwrap_or_else(|| format!("col_{}", col_idx));
                
                let val = match cell {
                    CellValue::Null => Value::Null,
                    CellValue::Bool(b) => Value::Bool(*b),
                    CellValue::Int(i) => Value::Number((*i).into()),
                    CellValue::Float(f) => Value::Number(
                        Number::from_f64(*f).unwrap_or_else(|| 0.into())
                    ),
                    _ => Value::String(cell.to_display_string()),
                };
                map.insert(col_name, val);
            }
            Value::Object(map)
        }).collect()
    }

    #[napi]
    pub fn to_html_report(&self, title: String, dark_mode: bool) -> String {
        let generator = dataforge_core::PdfReportGenerator::new(title).with_dark_mode(dark_mode);
        generator.render_html(&self.inner).unwrap_or_else(|e| format!("Error generating report: {e}"))
    }
}

/// JS Class for CSV Streaming Reading.
#[napi]
pub struct JsCsvReader {
    inner: CsvReader,
}

#[napi]
impl JsCsvReader {
    #[napi(factory)]
    pub fn open(path: String, batch_size: Option<u32>, parallel: Option<bool>) -> Result<Self> {
        let mut config = ReaderConfig::default();
        if let Some(bs) = batch_size {
            config = config.with_batch_size(bs as usize);
        }
        if let Some(p) = parallel {
            config = config.with_parallel(p);
        }

        CsvReader::open(&path, config)
            .map(|r| JsCsvReader { inner: r })
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }

    #[napi(getter)]
    pub fn headers(&self) -> Option<Vec<String>> {
        self.inner.headers().map(|h| h.to_vec())
    }

    #[napi]
    pub fn next_batch(&mut self) -> Result<Option<JsRowBatch>> {
        match self.inner.next_batch() {
            Some(Ok(batch)) => Ok(Some(JsRowBatch { inner: batch })),
            Some(Err(e)) => Err(Error::new(Status::GenericFailure, e.to_string())),
            None => Ok(None),
        }
    }

    #[napi]
    pub fn get_memory_utilization(&self) -> f64 {
        let stats = self.inner.memory_stats();
        if stats.limit_bytes > 0 {
            (stats.current_bytes as f64 / stats.limit_bytes as f64) * 100.0
        } else {
            0.0
        }
    }
}

/// JS Class for XLSX Streaming Reading.
#[napi]
pub struct JsXlsxReader {
    inner: XlsxReader,
}

#[napi]
impl JsXlsxReader {
    #[napi(factory)]
    pub fn open(
        path: String,
        batch_size: Option<u32>,
        sheet_name: Option<String>,
        password: Option<String>,
    ) -> Result<Self> {
        let mut config = ReaderConfig::default();
        if let Some(bs) = batch_size {
            config = config.with_batch_size(bs as usize);
        }
        if let Some(ref sn) = sheet_name {
            config = config.with_sheet_name(sn);
        }
        if let Some(ref pwd) = password {
            config = config.with_password(pwd);
        }

        XlsxReader::open(&path, config)
            .map(|r| JsXlsxReader { inner: r })
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }

    #[napi(getter)]
    pub fn headers(&self) -> Option<Vec<String>> {
        self.inner.headers().map(|h| h.to_vec())
    }

    #[napi]
    pub fn next_batch(&mut self) -> Result<Option<JsRowBatch>> {
        match self.inner.next_batch() {
            Some(Ok(batch)) => Ok(Some(JsRowBatch { inner: batch })),
            Some(Err(e)) => Err(Error::new(Status::GenericFailure, e.to_string())),
            None => Ok(None),
        }
    }

    #[napi]
    pub fn get_memory_utilization(&self) -> f64 {
        let stats = self.inner.memory_stats();
        if stats.limit_bytes > 0 {
            (stats.current_bytes as f64 / stats.limit_bytes as f64) * 100.0
        } else {
            0.0
        }
    }
}

/// JS Class for ODS Streaming Reading.
#[napi]
pub struct JsOdsReader {
    inner: OdsReader,
}

#[napi]
impl JsOdsReader {
    #[napi(factory)]
    pub fn open(path: String, batch_size: Option<u32>, sheet_name: Option<String>) -> Result<Self> {
        let mut config = ReaderConfig::default();
        if let Some(bs) = batch_size {
            config = config.with_batch_size(bs as usize);
        }
        if let Some(ref sn) = sheet_name {
            config = config.with_sheet_name(sn);
        }

        OdsReader::open(&path, config)
            .map(|r| JsOdsReader { inner: r })
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }

    #[napi(getter)]
    pub fn headers(&self) -> Option<Vec<String>> {
        self.inner.headers().map(|h| h.to_vec())
    }

    #[napi]
    pub fn next_batch(&mut self) -> Result<Option<JsRowBatch>> {
        match self.inner.next_batch() {
            Some(Ok(batch)) => Ok(Some(JsRowBatch { inner: batch })),
            Some(Err(e)) => Err(Error::new(Status::GenericFailure, e.to_string())),
            None => Ok(None),
        }
    }

    #[napi]
    pub fn get_memory_utilization(&self) -> f64 {
        let stats = self.inner.memory_stats();
        if stats.limit_bytes > 0 {
            (stats.current_bytes as f64 / stats.limit_bytes as f64) * 100.0
        } else {
            0.0
        }
    }
}

/// High-performance file format converter: CSV to XLSX.
#[napi]
pub fn convert_csv_to_xlsx(input_path: String, output_path: String) -> Result<i64> {
    dataforge_core::convert::convert_csv_to_xlsx(
        &input_path,
        &output_path,
        ReaderConfig::default(),
        WriterConfig::default(),
    )
    .map(|rows| rows as i64)
    .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
}

/// High-performance file format converter: XLSX to CSV.
#[napi]
pub fn convert_xlsx_to_csv(input_path: String, output_path: String) -> Result<i64> {
    dataforge_core::convert::convert_xlsx_to_csv(
        &input_path,
        &output_path,
        ReaderConfig::default(),
        WriterConfig::default(),
    )
    .map(|rows| rows as i64)
    .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
}
