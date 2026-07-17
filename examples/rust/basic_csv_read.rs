// =============================================================================
// DataForge Example — Basic CSV Streaming Read
// =============================================================================
// Demonstrates how to stream and process a CSV file in batches.
// =============================================================================

use dataforge_core::config::ReaderConfig;
use dataforge_core::csv::CsvReader;

fn main() {
    // Generate a temporary CSV file for demonstration
    let csv_content = "\
name,age,city
Alice,30,New York
Bob,25,Los Angeles
Charlie,35,San Francisco
";
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("dataforge_demo.csv");
    std::fs::write(&file_path, csv_content).unwrap();

    println!("Created demo CSV file: {}", file_path.display());

    // Configure the reader
    let config = ReaderConfig::default()
        .with_batch_size(2) // Small batch size for demonstration
        .with_parallel(false); // Run sequentially for small dataset

    // Open and stream
    let reader = CsvReader::open(&file_path, config).unwrap();
    
    println!("Headers: {:?}", reader.headers().unwrap());

    for (batch_idx, batch_res) in reader.enumerate() {
        let batch = batch_res.unwrap();
        println!("--- Batch {} ({} rows) ---", batch_idx + 1, batch.len());
        
        for row in batch.iter() {
            let name = row.get_str(0).unwrap_or("Unknown");
            let age = row.get_int(1).unwrap_or(0);
            let city = row.get_str(2).unwrap_or("Unknown");
            println!("Row {}: Name: {}, Age: {}, City: {}", row.index, name, age, city);
        }
    }

    // Clean up
    std::fs::remove_file(file_path).unwrap();
}
