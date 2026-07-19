// =============================================================================
// DataForge Core — Parallel Multi-Sheet Reader
// =============================================================================
// Concurrently processes multiple sheets in an XLSX file in parallel using Rayon.
// =============================================================================

use crate::xlsx::reader::XlsxReader;
use crate::config::ReaderConfig;
use crate::types::RowBatch;
use crate::error::Result;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

/// Concurrently reads all sheets of an XLSX workbook using a Rayon thread pool.
pub struct ParallelMultiSheetReader {
    path: PathBuf,
    config: ReaderConfig,
}

impl ParallelMultiSheetReader {
    /// Create a new ParallelMultiSheetReader.
    pub fn new(path: impl AsRef<Path>, config: ReaderConfig) -> Self {
        ParallelMultiSheetReader {
            path: path.as_ref().to_path_buf(),
            config,
        }
    }

    /// Read all sheets in parallel, returning sheet names and their respective row batches.
    pub fn read_all_sheets(&self) -> Result<Vec<(String, Vec<RowBatch>)>> {
        // Step 1: List all sheet names in the workbook
        let sheet_names = XlsxReader::sheet_names(&self.path)?;

        // Step 2: Use rayon parallel iterator to read each sheet concurrently
        let results: Result<Vec<(String, Vec<RowBatch>)>> = sheet_names
            .into_par_iter()
            .map(|name| {
                let mut sheet_config = self.config.clone();
                // Select the sheet by name
                sheet_config.xlsx.sheet_selector = crate::config::SheetSelector::ByName(name.clone());

                let reader = XlsxReader::open(&self.path, sheet_config)?;
                let mut batches = Vec::new();
                for batch in reader {
                    batches.push(batch?);
                }
                Ok((name, batches))
            })
            .collect();

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xlsx::writer::XlsxWriter;
    use crate::types::Row;
    use crate::types::CellValue;

    #[test]
    fn test_parallel_multi_sheet_and_metadata() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("multi_sheet.xlsx");

        // Write a test XLSX workbook with a single sheet (Sheet1 by default)
        {
            let headers = vec!["col1".to_string()];
            let writer_config = crate::config::WriterConfig::default().with_headers(headers);
            let mut writer = XlsxWriter::create(&file_path, writer_config).unwrap();
            let mut r1 = Row::new(0);
            r1.push(CellValue::from("ValueA"));
            writer.write_row(&r1).unwrap();
            writer.finish().unwrap();
        }

        // Test scan_metadata
        let metadata = XlsxReader::scan_metadata(&file_path).unwrap();
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].name, "Sheet1");

        // Test ParallelMultiSheetReader
        let reader = ParallelMultiSheetReader::new(&file_path, ReaderConfig::default());
        let results = reader.read_all_sheets().unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "Sheet1");
        assert_eq!(results[0].1[0].rows[0].get_str(0), Some("ValueA"));
    }
}
