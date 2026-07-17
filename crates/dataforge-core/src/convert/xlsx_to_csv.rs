// =============================================================================
// DataForge Core — XLSX to CSV Conversion
// =============================================================================
// Streaming conversion from XLSX to CSV format.
// =============================================================================

use std::path::Path;

use tracing::info;

use crate::config::{ReaderConfig, WriterConfig};
use crate::csv::CsvWriter;
use crate::error::Result;
use crate::xlsx::XlsxReader;

/// Convert an XLSX file to CSV format.
///
/// Streaming operation — data flows batch-by-batch.
///
/// # Arguments
/// * `input_path` - Path to the input XLSX file
/// * `output_path` - Path for the output CSV file
/// * `reader_config` - XLSX reader configuration
/// * `writer_config` - CSV writer configuration
///
/// # Returns
/// The number of data rows converted.
pub fn convert_xlsx_to_csv(
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
        "Starting XLSX → CSV conversion"
    );

    let mut reader = XlsxReader::open(input, reader_config)?;

    // Get headers from reader to pass to writer
    let first_batch = match reader.next_batch() {
        Some(Ok(batch)) => batch,
        Some(Err(e)) => return Err(e),
        None => return Ok(0),
    };

    let writer_config = if let Some(headers) = &first_batch.headers {
        writer_config.with_headers(headers.clone())
    } else {
        writer_config
    };

    let mut writer = CsvWriter::create(output, writer_config)?;
    let mut total_rows = first_batch.len() as u64;

    writer.write_batch(&first_batch)?;

    for batch_result in reader {
        let batch = batch_result?;
        total_rows += batch.len() as u64;
        writer.write_batch(&batch)?;
    }

    writer.finish()?;

    info!(rows = total_rows, "XLSX → CSV conversion complete");
    Ok(total_rows)
}
