use dataforge_core::config::ReaderConfig;
use dataforge_core::csv::CsvReader;
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn test_large_csv_memory_bounded() {
    let mut temp_file = NamedTempFile::new().unwrap();
    
    // Generate a CSV file with 5,000 rows
    writeln!(temp_file, "col1,col2,col3").unwrap();
    for i in 0..5000 {
        writeln!(temp_file, "row_{},{},{}", i, i * 2, i % 2 == 0).unwrap();
    }
    temp_file.flush().unwrap();

    let path = temp_file.path();

    // Set a memory limit of 10MB
    let config = ReaderConfig::default()
        .with_batch_size(1000)
        .with_max_memory_mb(10)
        .with_parallel(false);

    let mut reader = CsvReader::open(path, config).unwrap();
    
    let mut total_rows = 0;
    while let Some(batch_res) = reader.next_batch() {
        let batch = batch_res.unwrap();
        total_rows += batch.len();
        
        // Assert memory usage is within limit
        let stats = reader.memory_stats();
        assert!(stats.current_bytes <= stats.limit_bytes);
    }

    assert_eq!(total_rows, 5000);
}
