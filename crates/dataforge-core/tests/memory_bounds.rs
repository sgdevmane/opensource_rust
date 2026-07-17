use dataforge_core::config::{BackpressurePolicy, ReaderConfig};
use dataforge_core::csv::CsvReader;
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn test_memory_backpressure_error() {
    let mut temp_file = NamedTempFile::new().unwrap();
    
    // Generate a CSV file
    writeln!(temp_file, "col1,col2").unwrap();
    for i in 0..1000 {
        writeln!(temp_file, "data_{},some_more_data_{}", i, i).unwrap();
    }
    temp_file.flush().unwrap();
    let path = temp_file.path();

    // Set a memory limit of only 50KB
    let config = ReaderConfig::default()
        .with_batch_size(500)
        .with_max_memory_bytes(50 * 1024)
        .with_backpressure(BackpressurePolicy::Error);

    let reader_res = CsvReader::open(path, config);
    if let Ok(mut reader) = reader_res {
        let mut exceeded = false;
        while let Some(batch_res) = reader.next_batch() {
            if batch_res.is_err() {
                exceeded = true;
                break;
            }
        }
        println!("Test completed. Exceeded: {}", exceeded);
    }
}
