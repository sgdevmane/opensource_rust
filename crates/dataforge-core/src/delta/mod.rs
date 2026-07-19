// =============================================================================
// DataForge Core — Delta Lake Exporter
// =============================================================================
// Exporters for Delta Lake storage format (Parquet files + Transaction log).
// =============================================================================

use std::fs::{create_dir_all, File};
use std::path::{Path, PathBuf};
use chrono::Utc;
use serde_json::json;

use crate::error::Result;
use crate::parquet::ParquetWriter;
use crate::types::{DataType, RowBatch};
use tracing::info;

/// Delta Lake Exporter to write data to Delta tables with transaction logs.
pub struct DeltaLakeExporter {
    output_dir: PathBuf,
    parquet_writer: Option<ParquetWriter<File>>,
    current_part_path: PathBuf,
    part_name: String,
    rows_written: u64,
}

impl DeltaLakeExporter {
    /// Create a new DeltaLakeExporter targeting a specific output directory.
    pub fn new(output_dir: impl AsRef<Path>) -> Result<Self> {
        let output_dir = output_dir.as_ref().to_path_buf();
        create_dir_all(&output_dir)?;
        create_dir_all(output_dir.join("_delta_log"))?;

        let part_name = "part-00000.parquet".to_string();
        let current_part_path = output_dir.join(&part_name);

        Ok(DeltaLakeExporter {
            output_dir,
            parquet_writer: None,
            current_part_path,
            part_name,
            rows_written: 0,
        })
    }

    /// Write a RowBatch into the Delta table.
    pub fn write_batch(&mut self, batch: &RowBatch) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }

        if self.parquet_writer.is_none() {
            let _file = File::create(&self.current_part_path)?;
            let writer = ParquetWriter::new();
            self.parquet_writer = Some(writer);
        }

        let writer = self.parquet_writer.as_mut().unwrap();
        // Subsequent calls pass a dummy file to satisfy the signature since the writer is already initialized.
        let file_handle = File::options().write(true).open(&self.current_part_path)?;
        writer.write_batch(file_handle, batch)?;
        self.rows_written += batch.len() as u64;

        Ok(())
    }

    /// Sync the Delta table schema metadata to a central catalog registry.
    pub fn sync_catalog(&self, catalog_url: &str, table_name: &str, _schema_string: &str) -> Result<()> {
        info!("Syncing Delta Table '{}' schema metadata to catalog endpoint: {}", table_name, catalog_url);
        // We simulate this for local testing by printing it to tracing logs
        Ok(())
    }

    /// Close the exporter and generate the Delta Lake transaction log.
    pub fn finish(mut self, schema_headers: &[String], schema_types: &[DataType]) -> Result<()> {
        if let Some(writer) = self.parquet_writer.take() {
            writer.finish()?;
        }

        // Get file size
        let file_metadata = std::fs::metadata(&self.current_part_path)?;
        let file_size = file_metadata.len();
        let modification_time = Utc::now().timestamp_millis();

        // 1. Build Delta Schema String
        let fields: Vec<serde_json::Value> = schema_headers.iter().zip(schema_types.iter())
            .map(|(header, dtype)| {
                let type_name = match dtype {
                    DataType::Bool => "boolean",
                    DataType::Int => "long",
                    DataType::Float => "double",
                    DataType::Date => "date",
                    DataType::DateTime => "timestamp",
                    _ => "string",
                };
                json!({
                    "name": header,
                    "type": type_name,
                    "nullable": true,
                    "metadata": {}
                })
            })
            .collect();

        let schema_json = json!({
            "type": "struct",
            "fields": fields
        });
        let schema_string = serde_json::to_string(&schema_json).unwrap_or_default();

        // 2. Generate Delta Log lines
        let commit_info = json!({
            "commitInfo": {
                "timestamp": modification_time,
                "operation": "WRITE",
                "operationParameters": {
                    "mode": "Append",
                    "partitionBy": "[]"
                },
                "isBlindAppend": true
            }
        });

        let protocol = json!({
            "protocol": {
                "minReaderVersion": 1,
                "minWriterVersion": 2
            }
        });

        let metadata = json!({
            "metaData": {
                "id": uuid::Uuid::new_v4().to_string(),
                "format": {
                    "provider": "parquet",
                    "options": {}
                },
                "schemaString": schema_string,
                "partitionColumns": [],
                "configuration": {},
                "createdTime": modification_time
            }
        });

        let add_file = json!({
            "add": {
                "path": self.part_name,
                "partitionValues": {},
                "size": file_size,
                "modificationTime": modification_time,
                "dataChange": true
            }
        });

        // 3. Write transaction log file: _delta_log/00000000000000000000.json
        let log_path = self.output_dir.join("_delta_log").join("00000000000000000000.json");
        let mut log_file = File::create(log_path)?;
        use std::io::Write as _;
        writeln!(log_file, "{}", commit_info)?;
        writeln!(log_file, "{}", protocol)?;
        writeln!(log_file, "{}", metadata)?;
        writeln!(log_file, "{}", add_file)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CellValue, Row};

    #[test]
    fn test_delta_export() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut exporter = DeltaLakeExporter::new(temp_dir.path()).unwrap();

        let mut batch = RowBatch::new(0);
        batch.headers = Some(vec!["name".to_string(), "age".to_string()]);
        
        let mut r1 = Row::new(0);
        r1.push(CellValue::from("Alice"));
        r1.push(CellValue::from(30_i64));
        batch.push(r1);

        exporter.write_batch(&batch).unwrap();

        exporter.finish(
            &["name".to_string(), "age".to_string()],
            &[DataType::String, DataType::Int]
        ).unwrap();

        let exporter_ref = DeltaLakeExporter::new(temp_dir.path()).unwrap();
        exporter_ref.sync_catalog("https://my-catalog.org", "employees", "{}").unwrap();

        // Verify files exist
        assert!(temp_dir.path().join("part-00000.parquet").exists());
        assert!(temp_dir.path().join("_delta_log").join("00000000000000000000.json").exists());
    }
}
