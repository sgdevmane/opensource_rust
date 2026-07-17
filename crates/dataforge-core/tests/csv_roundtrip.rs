use dataforge_core::config::{ReaderConfig, WriterConfig};
use dataforge_core::csv::{CsvReader, CsvWriter};
use dataforge_core::types::{CellValue, Row};
use tempfile::NamedTempFile;

#[test]
fn test_csv_roundtrip_sequential() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    // Write CSV data
    let headers = vec!["name".to_string(), "age".to_string(), "active".to_string()];
    let writer_config = WriterConfig::default().with_headers(headers.clone());
    let mut writer = CsvWriter::create(path, writer_config).unwrap();

    let mut row1 = Row::new(0);
    row1.push(CellValue::from("Alice"));
    row1.push(CellValue::from(30_i64));
    row1.push(CellValue::from(true));
    writer.write_row(&row1).unwrap();

    let mut row2 = Row::new(1);
    row2.push(CellValue::from("Bob"));
    row2.push(CellValue::from(25_i64));
    row2.push(CellValue::from(false));
    writer.write_row(&row2).unwrap();

    writer.finish().unwrap();

    // Read CSV data back
    let reader_config = ReaderConfig::default().with_parallel(false);
    let mut reader = CsvReader::open(path, reader_config).unwrap();

    assert_eq!(reader.headers().unwrap(), &headers);

    let batch = reader.next_batch().unwrap().unwrap();
    assert_eq!(batch.len(), 2);
    
    let r1 = &batch.rows[0];
    assert_eq!(r1.get_str(0), Some("Alice"));
    assert_eq!(r1.get_int(1), Some(30));
    assert_eq!(r1.get(2).unwrap().as_bool(), Some(true));

    let r2 = &batch.rows[1];
    assert_eq!(r2.get_str(0), Some("Bob"));
    assert_eq!(r2.get_int(1), Some(25));
    assert_eq!(r2.get(2).unwrap().as_bool(), Some(false));
}

#[test]
fn test_csv_auto_detect_dialect() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    // Write semicolon separated CSV data
    let headers = vec!["name".to_string(), "age".to_string()];
    let writer_config = WriterConfig::default()
        .with_headers(headers.clone())
        .with_delimiter(b';');
    let mut writer = CsvWriter::create(path, writer_config).unwrap();

    let mut row1 = Row::new(0);
    row1.push(CellValue::from("Alice"));
    row1.push(CellValue::from(30_i64));
    writer.write_row(&row1).unwrap();
    writer.finish().unwrap();

    // Read CSV data back with auto detect
    let reader_config = ReaderConfig::default()
        .with_parallel(false)
        .with_auto_detect_dialect(true);
    let mut reader = CsvReader::open(path, reader_config).unwrap();

    assert_eq!(reader.headers().unwrap(), &headers);

    let batch = reader.next_batch().unwrap().unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch.rows[0].get_str(0), Some("Alice"));
    assert_eq!(batch.rows[0].get_int(1), Some(30));
}
