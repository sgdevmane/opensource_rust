// =============================================================================
// DataForge Core — CSV to XLSX Conversion
// =============================================================================
// Streaming conversion from CSV to XLSX format.
// Reads CSV batches and writes them to XLSX without materializing full data.
// =============================================================================

use std::path::Path;

use tracing::info;

use crate::config::{ReaderConfig, WriterConfig};
use crate::csv::CsvReader;
use crate::error::Result;
use crate::xlsx::XlsxWriter;

/// Convert a CSV file to XLSX format.
///
/// This is a streaming operation — data flows from the CSV reader
/// through to the XLSX writer batch-by-batch without loading the
/// entire file into memory.
///
/// # Arguments
/// * `input_path` - Path to the input CSV file
/// * `output_path` - Path for the output XLSX file
/// * `reader_config` - CSV reader configuration
/// * `writer_config` - XLSX writer configuration
///
/// # Returns
/// The number of data rows converted.
pub fn convert_csv_to_xlsx(
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    reader_config: ReaderConfig,
    writer_config: WriterConfig,
) -> Result<u64> {
    let input = input_path.as_ref();
    let output = output_path.as_ref();

    info!(
        input = %input.display(),
        output = %output.display(),
        "Starting CSV → XLSX conversion"
    );

    let reader = CsvReader::open(input, reader_config)?;

    // Get headers from reader to pass to writer
    let writer_config = if let Some(headers) = reader.headers() {
        writer_config.with_headers(headers.to_vec())
    } else {
        writer_config
    };

    let mut writer = XlsxWriter::create(output, writer_config)?;
    let mut total_rows = 0u64;

    for batch_result in reader {
        let batch = batch_result?;
        total_rows += batch.len() as u64;
        writer.write_batch(&batch)?;
    }

    writer.finish()?;

    info!(rows = total_rows, "CSV → XLSX conversion complete");
    Ok(total_rows)
}

#[cfg(test)]
mod tests {
    // Integration tests in tests/ directory
}
