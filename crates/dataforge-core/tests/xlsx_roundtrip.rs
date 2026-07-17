use dataforge_core::config::{ReaderConfig, WriterConfig};
use dataforge_core::xlsx::{XlsxReader, XlsxWriter};
use dataforge_core::types::{CellValue, Row};
use tempfile::NamedTempFile;

#[test]
fn test_xlsx_roundtrip() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    // Write XLSX data
    let headers = vec!["name".to_string(), "age".to_string()];
    let writer_config = WriterConfig::default().with_headers(headers.clone());
    let mut writer = XlsxWriter::create(path, writer_config).unwrap();

    let mut row1 = Row::new(0);
    row1.push(CellValue::from("Alice"));
    row1.push(CellValue::from(30_i64));
    writer.write_row(&row1).unwrap();

    let mut row2 = Row::new(1);
    row2.push(CellValue::from("Bob"));
    row2.push(CellValue::from(25_i64));
    writer.write_row(&row2).unwrap();

    writer.finish().unwrap();

    // Read XLSX data back
    let reader_config = ReaderConfig::default();
    let mut reader = XlsxReader::open(path, reader_config).unwrap();

    let batch = reader.next_batch().unwrap().unwrap();
    assert_eq!(reader.headers().unwrap(), &headers);
    assert_eq!(batch.len(), 2);
    
    let r1 = &batch.rows[0];
    assert_eq!(r1.get_str(0), Some("Alice"));
    assert_eq!(r1.get_int(1), Some(30));

    let r2 = &batch.rows[1];
    assert_eq!(r2.get_str(0), Some("Bob"));
    assert_eq!(r2.get_int(1), Some(25));
}
