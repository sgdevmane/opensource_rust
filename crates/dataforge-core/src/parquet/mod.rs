// =============================================================================
// DataForge Core — Streaming Parquet Integration
// =============================================================================
// Provides memory-bounded streaming reading and writing of Apache Parquet files.
// =============================================================================

use std::io::Write;
use std::sync::Arc;

use arrow::array::{
    Array, BooleanArray, Date32Array, Float32Array, Float64Array,
    Int16Array, Int32Array, Int64Array, Int8Array, LargeStringArray, StringArray,
    TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow::datatypes::DataType as ArrowDataType;
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;

use crate::arrow::{infer_arrow_schema, row_batch_to_arrow_columns};
use crate::error::{DataForgeError, Result};
use crate::types::{CellValue, Row, RowBatch};

/// Streaming Parquet Writer.
pub struct ParquetWriter<W: Write + Send> {
    writer: Option<ArrowWriter<W>>,
}

impl<W: Write + Send> Default for ParquetWriter<W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: Write + Send> ParquetWriter<W> {
    /// Create a new ParquetWriter.
    pub fn new() -> Self {
        ParquetWriter { writer: None }
    }

    /// Write a RowBatch to the Parquet stream.
    pub fn write_batch(&mut self, inner_writer: W, batch: &RowBatch) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }

        let schema = Arc::new(infer_arrow_schema(batch)?);
        let columns = row_batch_to_arrow_columns(batch, &schema)?;
        let record_batch = RecordBatch::try_new(schema.clone(), columns)
            .map_err(|e| DataForgeError::internal(format!("Arrow RecordBatch creation failed: {e}")))?;

        if self.writer.is_none() {
            let props = WriterProperties::builder().build();
            let w = ArrowWriter::try_new(inner_writer, schema, Some(props))
                .map_err(|e| DataForgeError::internal(format!("Parquet ArrowWriter init failed: {e}")))?;
            self.writer = Some(w);
        }

        let writer = self.writer.as_mut().unwrap();
        writer.write(&record_batch)
            .map_err(|e| DataForgeError::internal(format!("Parquet write failed: {e}")))?;

        Ok(())
    }

    /// Close and finalize the Parquet writer.
    pub fn finish(mut self) -> Result<()> {
        if let Some(writer) = self.writer.take() {
            writer.close()
                .map_err(|e| DataForgeError::internal(format!("Parquet close failed: {e}")))?;
        }
        Ok(())
    }
}

/// Streaming Parquet Reader.
pub struct ParquetReader {
    reader: parquet::arrow::arrow_reader::ParquetRecordBatchReader,
    headers: Vec<String>,
}

/// Metadata information for a Parquet column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParquetColumnMetadata {
    pub name: String,
    pub data_type: crate::types::DataType,
}

/// Scan only the metadata of a Parquet file to extract the schema columns and types.
pub fn scan_parquet_metadata<R: parquet::file::reader::ChunkReader + 'static>(reader: R) -> Result<Vec<ParquetColumnMetadata>> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(reader)
        .map_err(|e| DataForgeError::internal(format!("Parquet reader builder failed: {e}")))?;
    
    let schema_arrow = builder.schema();
    let mut columns = Vec::new();
    
    for field in schema_arrow.fields() {
        let name = field.name().clone();
        let data_type = match field.data_type() {
            ArrowDataType::Boolean => crate::types::DataType::Bool,
            ArrowDataType::Int8 | ArrowDataType::Int16 | ArrowDataType::Int32 | ArrowDataType::Int64 |
            ArrowDataType::UInt8 | ArrowDataType::UInt16 | ArrowDataType::UInt32 | ArrowDataType::UInt64 => crate::types::DataType::Int,
            ArrowDataType::Float32 | ArrowDataType::Float64 => crate::types::DataType::Float,
            ArrowDataType::Date32 | ArrowDataType::Date64 => crate::types::DataType::Date,
            ArrowDataType::Timestamp(_, _) => crate::types::DataType::DateTime,
            _ => crate::types::DataType::String,
        };
        columns.push(ParquetColumnMetadata { name, data_type });
    }
    
    Ok(columns)
}

impl ParquetReader {
    /// Create a new ParquetReader from a seekable reader source.
    pub fn new<R: parquet::file::reader::ChunkReader + 'static>(reader: R) -> Result<Self> {
        let builder = ParquetRecordBatchReaderBuilder::try_new(reader)
            .map_err(|e| DataForgeError::internal(format!("Parquet reader builder failed: {e}")))?;
        
        let file_metadata = builder.metadata().file_metadata();
        let schema = file_metadata.schema_descr();
        let headers: Vec<String> = schema.columns().iter().map(|c| c.name().to_string()).collect();

        // Stream in chunks of 2048 rows
        let reader = builder.with_batch_size(2048).build()
            .map_err(|e| DataForgeError::internal(format!("Parquet reader build failed: {e}")))?;

        Ok(ParquetReader { reader, headers })
    }

    /// Get column headers.
    pub fn headers(&self) -> &[String] {
        &self.headers
    }

    /// Read the next RowBatch from the Parquet stream.
    pub fn next_batch(&mut self) -> Option<Result<RowBatch>> {
        match self.reader.next() {
            Some(Ok(record_batch)) => {
                let batch = record_batch_to_row_batch(&record_batch, &self.headers);
                Some(Ok(batch))
            }
            Some(Err(e)) => Some(Err(DataForgeError::internal(format!("Parquet batch read failed: {e}")))),
            None => None,
        }
    }
}

fn record_batch_to_row_batch(record_batch: &RecordBatch, headers: &[String]) -> RowBatch {
    let num_rows = record_batch.num_rows();
    let num_cols = record_batch.num_columns();
    let mut row_batch = RowBatch::new(0);
    row_batch.headers = Some(headers.to_vec());

    for row_idx in 0..num_rows {
        let mut row = Row::new(row_idx as u64);
        for col_idx in 0..num_cols {
            let column = record_batch.column(col_idx);
            let cell = arrow_column_to_cell_value(column.as_ref(), row_idx);
            row.push(cell);
        }
        row_batch.push(row);
    }

    row_batch
}

fn arrow_column_to_cell_value(column: &dyn Array, row_idx: usize) -> CellValue {
    if column.is_null(row_idx) {
        return CellValue::Null;
    }

    match column.data_type() {
        ArrowDataType::Boolean => {
            let array = column.as_any().downcast_ref::<BooleanArray>().unwrap();
            CellValue::Bool(array.value(row_idx))
        }
        ArrowDataType::Int8 => {
            let array = column.as_any().downcast_ref::<Int8Array>().unwrap();
            CellValue::Int(array.value(row_idx) as i64)
        }
        ArrowDataType::Int16 => {
            let array = column.as_any().downcast_ref::<Int16Array>().unwrap();
            CellValue::Int(array.value(row_idx) as i64)
        }
        ArrowDataType::Int32 => {
            let array = column.as_any().downcast_ref::<Int32Array>().unwrap();
            CellValue::Int(array.value(row_idx) as i64)
        }
        ArrowDataType::Int64 => {
            let array = column.as_any().downcast_ref::<Int64Array>().unwrap();
            CellValue::Int(array.value(row_idx))
        }
        ArrowDataType::UInt8 => {
            let array = column.as_any().downcast_ref::<UInt8Array>().unwrap();
            CellValue::Int(array.value(row_idx) as i64)
        }
        ArrowDataType::UInt16 => {
            let array = column.as_any().downcast_ref::<UInt16Array>().unwrap();
            CellValue::Int(array.value(row_idx) as i64)
        }
        ArrowDataType::UInt32 => {
            let array = column.as_any().downcast_ref::<UInt32Array>().unwrap();
            CellValue::Int(array.value(row_idx) as i64)
        }
        ArrowDataType::UInt64 => {
            let array = column.as_any().downcast_ref::<UInt64Array>().unwrap();
            CellValue::Int(array.value(row_idx) as i64)
        }
        ArrowDataType::Float32 => {
            let array = column.as_any().downcast_ref::<Float32Array>().unwrap();
            CellValue::Float(array.value(row_idx) as f64)
        }
        ArrowDataType::Float64 => {
            let array = column.as_any().downcast_ref::<Float64Array>().unwrap();
            CellValue::Float(array.value(row_idx))
        }
        ArrowDataType::Date32 => {
            let array = column.as_any().downcast_ref::<Date32Array>().unwrap();
            let days = array.value(row_idx);
            let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
            if let Some(date) = epoch.checked_add_signed(chrono::Duration::days(days as i64)) {
                CellValue::Date(date)
            } else {
                CellValue::Null
            }
        }
        ArrowDataType::Timestamp(_, _) => {
            if let Some(array) = column.as_any().downcast_ref::<TimestampSecondArray>() {
                if let Some(dt) = chrono::DateTime::from_timestamp(array.value(row_idx), 0) {
                    return CellValue::DateTime(dt.naive_utc());
                }
            } else if let Some(array) = column.as_any().downcast_ref::<TimestampMillisecondArray>() {
                if let Some(dt) = chrono::DateTime::from_timestamp(array.value(row_idx) / 1000, 0) {
                    return CellValue::DateTime(dt.naive_utc());
                }
            } else if let Some(array) = column.as_any().downcast_ref::<TimestampMicrosecondArray>() {
                if let Some(dt) = chrono::DateTime::from_timestamp(array.value(row_idx) / 1_000_000, 0) {
                    return CellValue::DateTime(dt.naive_utc());
                }
            } else if let Some(array) = column.as_any().downcast_ref::<TimestampNanosecondArray>() {
                if let Some(dt) = chrono::DateTime::from_timestamp(array.value(row_idx) / 1_000_000_000, 0) {
                    return CellValue::DateTime(dt.naive_utc());
                }
            }
            CellValue::Null
        }
        ArrowDataType::Utf8 => {
            let array = column.as_any().downcast_ref::<StringArray>().unwrap();
            CellValue::String(array.value(row_idx).into())
        }
        ArrowDataType::LargeUtf8 => {
            let array = column.as_any().downcast_ref::<LargeStringArray>().unwrap();
            CellValue::String(array.value(row_idx).into())
        }
        _ => CellValue::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parquet_roundtrip() {
        let mut batch = RowBatch::new(0);
        batch.headers = Some(vec!["name".to_string(), "age".to_string()]);
        
        let mut r1 = Row::new(0);
        r1.push(CellValue::from("Alice"));
        r1.push(CellValue::from(30_i64));
        batch.push(r1);

        let mut r2 = Row::new(1);
        r2.push(CellValue::from("Bob"));
        r2.push(CellValue::from(25_i64));
        batch.push(r2);

        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let file_write = temp_file.reopen().unwrap();

        let mut writer = ParquetWriter::new();
        writer.write_batch(file_write, &batch).unwrap();
        writer.finish().unwrap();

        let file_read = std::fs::File::open(temp_file.path()).unwrap();
        let mut reader = ParquetReader::new(file_read).unwrap();
        let read_batch = reader.next_batch().unwrap().unwrap();
        
        assert_eq!(read_batch.len(), 2);
        assert_eq!(read_batch.rows[0].get_str(0), Some("Alice"));
        assert_eq!(read_batch.rows[1].get_int(1), Some(25));
    }

    #[test]
    fn test_parquet_metadata_scanner() {
        let mut batch = RowBatch::new(0);
        batch.headers = Some(vec!["name".to_string(), "age".to_string(), "active".to_string()]);
        
        let mut r1 = Row::new(0);
        r1.push(CellValue::from("Alice"));
        r1.push(CellValue::from(30_i64));
        r1.push(CellValue::from(true));
        batch.push(r1);

        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let file_write = temp_file.reopen().unwrap();

        let mut writer = ParquetWriter::new();
        writer.write_batch(file_write, &batch).unwrap();
        writer.finish().unwrap();

        let file_read = std::fs::File::open(temp_file.path()).unwrap();
        let columns = scan_parquet_metadata(file_read).unwrap();

        assert_eq!(columns.len(), 3);
        assert_eq!(columns[0].name, "name");
        assert_eq!(columns[0].data_type, crate::types::DataType::String);
        assert_eq!(columns[1].name, "age");
        assert_eq!(columns[1].data_type, crate::types::DataType::Int);
        assert_eq!(columns[2].name, "active");
        assert_eq!(columns[2].data_type, crate::types::DataType::Bool);
    }
}
