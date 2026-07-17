use dataforge_core::config::ReaderConfig;
use dataforge_core::csv::CsvReader;
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn test_parallel_correctness() {
    let mut temp_file = NamedTempFile::new().unwrap();
    
    // Generate a CSV file with 5,000 rows
    writeln!(temp_file, "index,value").unwrap();
    for i in 0..5000 {
        writeln!(temp_file, "{},value_{}", i, i).unwrap();
    }
    temp_file.flush().unwrap();
    let path = temp_file.path();

    // 1. Read sequentially
    let seq_config = ReaderConfig::default()
        .with_batch_size(500)
        .with_parallel(false);
    let seq_reader = CsvReader::open(path, seq_config).unwrap();
    let mut seq_rows = Vec::new();
    for batch in seq_reader {
        for row in batch.unwrap().iter() {
            seq_rows.push((row.get_int(0).unwrap(), row.get_str(1).unwrap().to_string()));
        }
    }

    // 2. Read in parallel
    let par_config = ReaderConfig::default()
        .with_batch_size(500)
        .with_parallel(true)
        .with_num_threads(4);
    let par_reader = CsvReader::open(path, par_config).unwrap();
    let mut par_rows = Vec::new();
    for batch in par_reader {
        for row in batch.unwrap().iter() {
            par_rows.push((row.get_int(0).unwrap(), row.get_str(1).unwrap().to_string()));
        }
    }

    // Verify they are identical
    assert_eq!(seq_rows.len(), par_rows.len());
    assert_eq!(seq_rows, par_rows);
}
