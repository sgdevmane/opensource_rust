// =============================================================================
// DataForge Core — Apache Arrow Exporter
// =============================================================================
// Exposes streaming conversion and export to Apache Arrow IPC formats.
// =============================================================================

use std::io::Write;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanBuilder, Date32Builder, Float64Builder, Int64Builder,
    StringBuilder, TimestampSecondBuilder,
};
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema, SchemaRef};
use arrow::ipc::writer::FileWriter;
use arrow::record_batch::RecordBatch;

use crate::error::{DataForgeError, Result};
use crate::types::{CellValue, DataType, RowBatch};

/// Streaming Apache Arrow IPC Writer.
pub struct ArrowIpcWriter<W: Write> {
    writer: Option<FileWriter<W>>,
    schema: Option<SchemaRef>,
}

impl<W: Write> Default for ArrowIpcWriter<W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: Write> ArrowIpcWriter<W> {
    /// Create a new ArrowIpcWriter.
    pub fn new() -> Self {
        ArrowIpcWriter {
            writer: None,
            schema: None,
        }
    }

    /// Write a RowBatch to the Arrow IPC stream.
    pub fn write_batch(&mut self, inner_writer: W, batch: &RowBatch) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }

        // Initialize schema if not already done
        if self.schema.is_none() {
            let schema = infer_arrow_schema(batch)?;
            self.schema = Some(Arc::new(schema));
        }

        let schema_ref = self.schema.as_ref().unwrap();

        // Convert RowBatch cells to Arrow columns
        let columns = row_batch_to_arrow_columns(batch, schema_ref)?;

        let record_batch = RecordBatch::try_new(Arc::clone(schema_ref), columns)
            .map_err(|e| DataForgeError::internal(format!("Arrow RecordBatch creation failed: {}", e)))?;

        // Initialize writer if not done
        if self.writer.is_none() {
            let w = FileWriter::try_new(inner_writer, schema_ref)
                .map_err(|e| DataForgeError::internal(format!("Arrow IPC FileWriter initialization failed: {}", e)))?;
            self.writer = Some(w);
        }

        let writer = self.writer.as_mut().unwrap();
        writer.write(&record_batch)
            .map_err(|e| DataForgeError::io(std::io::Error::new(std::io::ErrorKind::Other, e), "Failed to write Arrow RecordBatch to IPC"))?;

        Ok(())
    }

    /// Finalize and close the Arrow IPC stream.
    pub fn finish(mut self) -> Result<()> {
        if let Some(mut writer) = self.writer.take() {
            writer.finish()
                .map_err(|e| DataForgeError::io(std::io::Error::new(std::io::ErrorKind::Other, e), "Failed to finalize Arrow IPC writer"))?;
        }
        Ok(())
    }
}

pub fn infer_arrow_schema(batch: &RowBatch) -> Result<Schema> {
    let headers = batch.headers.as_ref().ok_or_else(|| {
        DataForgeError::config("Cannot write to Arrow without column headers")
    })?;

    let mut fields = Vec::with_capacity(headers.len());
    
    for (col_idx, header) in headers.iter().enumerate() {
        let mut data_type = DataType::String;
        for row in &batch.rows {
            if let Some(cell) = row.get(col_idx) {
                if !cell.is_null() {
                    data_type = cell.data_type();
                    break;
                }
            }
        }

        let arrow_type = match data_type {
            DataType::Bool => ArrowDataType::Boolean,
            DataType::Int => ArrowDataType::Int64,
            DataType::Float => ArrowDataType::Float64,
            DataType::DateTime => ArrowDataType::Timestamp(arrow::datatypes::TimeUnit::Second, None),
            DataType::Date => ArrowDataType::Date32,
            _ => ArrowDataType::Utf8,
        };

        fields.push(Field::new(header, arrow_type, true));
    }

    Ok(Schema::new(fields))
}

pub fn row_batch_to_arrow_columns(batch: &RowBatch, schema: &Schema) -> Result<Vec<ArrayRef>> {
    let num_rows = batch.len();
    let mut builders: Vec<Box<dyn std::any::Any>> = Vec::with_capacity(schema.fields().len());

    for field in schema.fields() {
        match field.data_type() {
            ArrowDataType::Boolean => builders.push(Box::new(BooleanBuilder::with_capacity(num_rows))),
            ArrowDataType::Int64 => builders.push(Box::new(Int64Builder::with_capacity(num_rows))),
            ArrowDataType::Float64 => builders.push(Box::new(Float64Builder::with_capacity(num_rows))),
            ArrowDataType::Timestamp(_, _) => builders.push(Box::new(TimestampSecondBuilder::with_capacity(num_rows))),
            ArrowDataType::Date32 => builders.push(Box::new(Date32Builder::with_capacity(num_rows))),
            _ => builders.push(Box::new(StringBuilder::new())),
        }
    }

    for row in &batch.rows {
        for (col_idx, field) in schema.fields().iter().enumerate() {
            let cell = row.get(col_idx).unwrap_or(&CellValue::Null);
            match field.data_type() {
                ArrowDataType::Boolean => {
                    let b = builders[col_idx].downcast_mut::<BooleanBuilder>().unwrap();
                    if let Some(v) = cell.as_bool() {
                        b.append_value(v);
                    } else {
                        b.append_null();
                    }
                }
                ArrowDataType::Int64 => {
                    let b = builders[col_idx].downcast_mut::<Int64Builder>().unwrap();
                    if let Some(v) = cell.as_int() {
                        b.append_value(v);
                    } else {
                        b.append_null();
                    }
                }
                ArrowDataType::Float64 => {
                    let b = builders[col_idx].downcast_mut::<Float64Builder>().unwrap();
                    if let Some(v) = cell.as_float() {
                        b.append_value(v);
                    } else {
                        b.append_null();
                    }
                }
                ArrowDataType::Timestamp(_, _) => {
                    let b = builders[col_idx].downcast_mut::<TimestampSecondBuilder>().unwrap();
                    if let CellValue::DateTime(dt) = cell {
                        b.append_value(dt.and_utc().timestamp());
                    } else {
                        b.append_null();
                    }
                }
                ArrowDataType::Date32 => {
                    let b = builders[col_idx].downcast_mut::<Date32Builder>().unwrap();
                    if let CellValue::Date(d) = cell {
                        let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
                        let days = d.signed_duration_since(epoch).num_days() as i32;
                        b.append_value(days);
                    } else {
                        b.append_null();
                    }
                }
                _ => {
                    let b = builders[col_idx].downcast_mut::<StringBuilder>().unwrap();
                    if cell.is_null() {
                        b.append_null();
                    } else {
                        b.append_value(cell.to_display_string());
                    }
                }
            }
        }
    }

    let mut columns = Vec::with_capacity(schema.fields().len());
    for (col_idx, field) in schema.fields().iter().enumerate() {
        let col: ArrayRef = match field.data_type() {
            ArrowDataType::Boolean => {
                Arc::new(builders[col_idx].downcast_mut::<BooleanBuilder>().unwrap().finish())
            }
            ArrowDataType::Int64 => {
                Arc::new(builders[col_idx].downcast_mut::<Int64Builder>().unwrap().finish())
            }
            ArrowDataType::Float64 => {
                Arc::new(builders[col_idx].downcast_mut::<Float64Builder>().unwrap().finish())
            }
            ArrowDataType::Timestamp(_, _) => {
                Arc::new(builders[col_idx].downcast_mut::<TimestampSecondBuilder>().unwrap().finish())
            }
            ArrowDataType::Date32 => {
                Arc::new(builders[col_idx].downcast_mut::<Date32Builder>().unwrap().finish())
            }
            _ => {
                Arc::new(builders[col_idx].downcast_mut::<StringBuilder>().unwrap().finish())
            }
        };
        columns.push(col);
    }

    Ok(columns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Row;
    use std::io::Cursor;

    #[test]
    fn test_arrow_ipc_writer_roundtrip() {
        let mut batch = RowBatch::new(0);
        batch.headers = Some(vec!["name".to_string(), "age".to_string()]);
        
        let mut r1 = Row::new(0);
        r1.push(CellValue::from("Alice"));
        r1.push(CellValue::from(30_i64));
        batch.push(r1);

        let mut r2 = Row::new(1);
        r2.push(CellValue::from("Bob"));
        r2.push(CellValue::Null);
        batch.push(r2);

        let mut buffer = Cursor::new(Vec::new());
        let mut writer = ArrowIpcWriter::new();
        writer.write_batch(&mut buffer, &batch).unwrap();
        writer.finish().unwrap();

        let bytes = buffer.into_inner();
        assert!(!bytes.is_empty());
    }
}
