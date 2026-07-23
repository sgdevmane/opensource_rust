// =============================================================================
// DataForge Core — Fuzz Testing Suite
// =============================================================================
// Fuzzes CSV and XLSX parsers with arbitrary byte streams to verify panic safety.
// =============================================================================

use dataforge_core::csv::CsvReader;
use dataforge_core::config::ReaderConfig;

pub fn fuzz_csv_parser(data: &[u8]) {
    let config = ReaderConfig::default().with_parallel(false);
    if let Ok(mut reader) = CsvReader::from_bytes(data.to_vec(), config) {
        while let Some(Ok(_batch)) = reader.next_batch() {
            // Drain batches to exercise parsing path
        }
    }
}
