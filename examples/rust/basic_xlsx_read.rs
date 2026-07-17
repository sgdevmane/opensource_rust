// =============================================================================
// DataForge Example — Basic XLSX Streaming Read
// =============================================================================
// Demonstrates how to stream and process an Excel spreadsheet.
// =============================================================================

use dataforge_core::config::{ReaderConfig, WriterConfig};
use dataforge_core::xlsx::{XlsxReader, XlsxWriter};
use dataforge_core::types::{CellValue, Row};

fn main() {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("dataforge_demo.xlsx");

    // Write a demo Excel file first
    let writer_config = WriterConfig::default()
        .with_headers(vec!["Item".to_string(), "Quantity".to_string()]);
    let mut writer = XlsxWriter::create(&file_path, writer_config).unwrap();

    let mut row1 = Row::new(0);
    row1.push(CellValue::from("Apples"));
    row1.push(CellValue::from(10_i64));
    writer.write_row(&row1).unwrap();

    let mut row2 = Row::new(1);
    row2.push(CellValue::from("Oranges"));
    row2.push(CellValue::from(15_i64));
    writer.write_row(&row2).unwrap();

    writer.finish().unwrap();
    println!("Created demo XLSX file: {}", file_path.display());

    // Stream the file back
    let config = ReaderConfig::default().with_batch_size(1);
    let mut reader = XlsxReader::open(&file_path, config).unwrap();

    println!("Headers: {:?}", reader.headers().unwrap());

    for (batch_idx, batch_res) in reader.enumerate() {
        let batch = batch_res.unwrap();
        println!("--- Batch {} ---", batch_idx + 1);
        for row in batch.iter() {
            let item = row.get_str(0).unwrap_or("Unknown");
            let qty = row.get_int(1).unwrap_or(0);
            println!("Item: {}, Qty: {}", item, qty);
        }
    }

    // Clean up
    std::fs::remove_file(file_path).unwrap();
}
