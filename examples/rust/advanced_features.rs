// =============================================================================
// DataForge Examples — Advanced Features
// =============================================================================
// Demonstrates advanced features:
// - Password decryption of Agile-protected XLSX files
// - Applying custom XLSX styling templates
// - Exporting Prometheus metrics for observability
// =============================================================================

use dataforge_core::config::{ReaderConfig, WriterConfig};
use dataforge_core::xlsx::{XlsxWriter, StyleTemplate};
use dataforge_core::types::{Row, CellValue};
use std::io::Cursor;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== DataForge Advanced Features Example ===");

    // 1. Password Protected XLSX Decryption (ECMA-376 Standard Agile)
    // Create a reader configuration with a workbook password.
    let _reader_config = ReaderConfig::default()
        .with_password("my_secure_password");
    println!("Configured XLSX Reader with workbook decryption password.");

    // 2. Styling templates usage for XLSX output
    // Configure an XLSX writer with the 'Professional' styling template.
    let writer_config = WriterConfig::default()
        .with_headers(vec!["Task".to_string(), "Status".to_string(), "Progress".to_string()])
        .with_style_template(StyleTemplate::Professional);

    let output_buffer = Cursor::new(Vec::new());
    let mut writer = XlsxWriter::new(output_buffer, writer_config)?;

    // Write styled rows
    let mut row1 = Row::new(0);
    row1.push(CellValue::from("Download parser"));
    row1.push(CellValue::from("Completed"));
    row1.push(CellValue::from(1.0));
    writer.write_row(&row1)?;

    let mut row2 = Row::new(1);
    row2.push(CellValue::from("Run validation tests"));
    row2.push(CellValue::from("In Progress"));
    row2.push(CellValue::from(0.42));
    writer.write_row(&row2)?;

    let _total_rows = writer.finish()?;
    println!("Wrote styled workbook using 'Professional' StyleTemplate (freeze header, custom font, navy headers, alternating row colors).");

    // 3. Prometheus Metrics Export
    // Let's get the Prometheus metrics output from the system's memory tracker.
    let tracker = dataforge_core::memory::MemoryTracker::new(1024 * 1024, dataforge_core::config::BackpressurePolicy::Error);
    let _guard = tracker.try_allocate(512 * 1024)?;
    
    let prometheus_metrics = tracker.to_prometheus();
    println!("\n=== Exported Prometheus Metrics ===");
    println!("{}", prometheus_metrics);

    Ok(())
}
