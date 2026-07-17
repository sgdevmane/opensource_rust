// =============================================================================
// DataForge Core — External Merge Sort
// =============================================================================
// Sorts datasets larger than available memory using external merge sort:
// 1. Split input into sorted chunks that fit in memory
// 2. Write chunks to temporary files
// 3. Merge sorted chunks using a k-way merge with a min-heap
// =============================================================================

use std::cmp::Ordering;
use std::io::{BufRead, BufReader, BufWriter, Write};
use tempfile::NamedTempFile;

use crate::types::{CellValue, Row, RowBatch};

/// Sort direction for a column.
#[derive(Debug, Clone, Copy)]
pub enum SortOrder {
    /// Ascending (smallest first)
    Asc,
    /// Descending (largest first)
    Desc,
}

/// Sort specification for a column.
#[derive(Debug, Clone)]
pub struct SortKey {
    /// Column index to sort by (0-based)
    pub column: usize,
    /// Sort direction
    pub order: SortOrder,
    /// Whether to put nulls first or last
    pub nulls_first: bool,
}

/// Sort a batch of rows in-place by the given sort keys.
///
/// This is used for in-memory sorting of individual batches.
/// For full-file sorting, use the external merge sort pipeline.
pub fn sort_batch(batch: &mut RowBatch, keys: &[SortKey]) {
    batch.rows.sort_by(|a, b| compare_rows(a, b, keys));
}

/// Compare two rows by the given sort keys (multi-column sort).
fn compare_rows(a: &Row, b: &Row, keys: &[SortKey]) -> Ordering {
    for key in keys {
        let cell_a = a.get(key.column).unwrap_or(&CellValue::Null);
        let cell_b = b.get(key.column).unwrap_or(&CellValue::Null);

        let ordering = compare_cells(cell_a, cell_b, key.nulls_first);
        let ordering = match key.order {
            SortOrder::Asc => ordering,
            SortOrder::Desc => ordering.reverse(),
        };

        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

/// Compare two cell values with null handling.
fn compare_cells(a: &CellValue, b: &CellValue, nulls_first: bool) -> Ordering {
    match (a.is_null(), b.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => {
            if nulls_first {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (false, true) => {
            if nulls_first {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (false, false) => {
            // Both non-null — compare by type
            match (a, b) {
                (CellValue::Int(x), CellValue::Int(y)) => x.cmp(y),
                (CellValue::Float(x), CellValue::Float(y)) => {
                    x.partial_cmp(y).unwrap_or(Ordering::Equal)
                }
                (CellValue::Int(x), CellValue::Float(y)) => {
                    (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal)
                }
                (CellValue::Float(x), CellValue::Int(y)) => {
                    x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal)
                }
                (CellValue::String(x), CellValue::String(y)) => x.cmp(y),
                (CellValue::DateTime(x), CellValue::DateTime(y)) => x.cmp(y),
                (CellValue::Date(x), CellValue::Date(y)) => x.cmp(y),
                (CellValue::Time(x), CellValue::Time(y)) => x.cmp(y),
                (CellValue::Bool(x), CellValue::Bool(y)) => x.cmp(y),
                // Mixed types: compare by display string
                _ => a.to_display_string().cmp(&b.to_display_string()),
            }
        }
    }
}

/// A lazy iterator that performs a k-way merge of sorted temporary files on disk.
pub struct ExternalSortIterator {
    keys: Vec<SortKey>,
    readers: Vec<BufReader<std::fs::File>>,
    #[allow(dead_code)]
    temp_files: Vec<NamedTempFile>,
    active_rows: Vec<Option<Row>>,
    line_buffers: Vec<String>,
    batch_size: usize,
    headers: Option<Vec<String>>,
    row_index: u64,
    exhausted: bool,
}

impl ExternalSortIterator {
    /// Create a new ExternalSortIterator from a list of sorted run files.
    pub fn new(
        keys: Vec<SortKey>,
        temp_files: Vec<NamedTempFile>,
        batch_size: usize,
        headers: Option<Vec<String>>,
    ) -> Self {
        let mut readers = Vec::with_capacity(temp_files.len());
        let mut active_rows = Vec::with_capacity(temp_files.len());
        let mut line_buffers = Vec::with_capacity(temp_files.len());

        for temp in &temp_files {
            if let Ok(file) = std::fs::File::open(temp.path()) {
                let mut reader = BufReader::new(file);
                let mut line = String::new();
                let next_row = read_next_row(&mut reader, &mut line);
                readers.push(reader);
                active_rows.push(next_row);
                line_buffers.push(line);
            }
        }

        let exhausted = temp_files.is_empty();

        ExternalSortIterator {
            keys,
            readers,
            temp_files,
            active_rows,
            line_buffers,
            batch_size,
            headers,
            row_index: 0,
            exhausted,
        }
    }
}

fn read_next_row<R: BufRead>(reader: &mut R, line_buf: &mut String) -> Option<Row> {
    line_buf.clear();
    if reader.read_line(line_buf).ok()? > 0 {
        serde_json::from_str(line_buf).ok()
    } else {
        None
    }
}

impl Iterator for ExternalSortIterator {
    type Item = crate::error::Result<RowBatch>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.exhausted {
            return None;
        }

        let mut batch = RowBatch::with_capacity(self.row_index, self.batch_size);
        batch.headers = self.headers.clone();

        for _ in 0..self.batch_size {
            // Find the index of the minimum row across active rows
            let mut min_idx: Option<usize> = None;
            for i in 0..self.active_rows.len() {
                if let Some(ref row) = self.active_rows[i] {
                    match min_idx {
                        None => min_idx = Some(i),
                        Some(best) => {
                            if let Some(ref best_row) = self.active_rows[best] {
                                if compare_rows(row, best_row, &self.keys) == Ordering::Less {
                                    min_idx = Some(i);
                                }
                            }
                        }
                    }
                }
            }

            if let Some(idx) = min_idx {
                let row = self.active_rows[idx].take().unwrap();
                batch.push(row);

                // Reload the next row for that run
                self.active_rows[idx] = read_next_row(&mut self.readers[idx], &mut self.line_buffers[idx]);
            } else {
                self.exhausted = true;
                break;
            }
        }

        if batch.is_empty() {
            None
        } else {
            if self.exhausted {
                batch.is_last = true;
            }
            self.row_index += batch.len() as u64;
            Some(Ok(batch))
        }
    }
}

/// Sort a stream of row batches using disk-buffered external merge sort.
pub fn external_sort(
    mut stream: impl Iterator<Item = crate::error::Result<RowBatch>>,
    keys: Vec<SortKey>,
    max_memory_bytes: usize,
    batch_size: usize,
) -> crate::error::Result<ExternalSortIterator> {
    let mut temp_files = Vec::new();
    let mut current_run = Vec::new();
    let mut current_memory = 0;
    let mut headers = None;

    while let Some(batch_res) = stream.next() {
        let batch = batch_res?;
        if headers.is_none() {
            headers = batch.headers.clone();
        }

        current_memory += batch.estimated_memory_bytes();
        current_run.extend(batch.rows);

        if current_memory >= max_memory_bytes {
            current_run.sort_by(|a, b| compare_rows(a, b, &keys));

            let temp_file = NamedTempFile::new()?;
            {
                let file = std::fs::File::create(temp_file.path())?;
                let mut writer = BufWriter::new(file);
                for row in &current_run {
                    serde_json::to_writer(&mut writer, row)?;
                    writer.write_all(b"\n")?;
                }
                writer.flush()?;
            }
            temp_files.push(temp_file);

            current_run.clear();
            current_memory = 0;
        }
    }

    if !current_run.is_empty() {
        current_run.sort_by(|a, b| compare_rows(a, b, &keys));
        let temp_file = NamedTempFile::new()?;
        {
            let file = std::fs::File::create(temp_file.path())?;
            let mut writer = BufWriter::new(file);
            for row in &current_run {
                serde_json::to_writer(&mut writer, row)?;
                writer.write_all(b"\n")?;
            }
            writer.flush()?;
        }
        temp_files.push(temp_file);
    }

    Ok(ExternalSortIterator::new(keys, temp_files, batch_size, headers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_batch_for_sort() -> RowBatch {
        let mut batch = RowBatch::new(0);

        let data = vec![
            ("Charlie", 35_i64),
            ("Alice", 30),
            ("Bob", 25),
            ("Diana", 28),
        ];

        for (i, (name, age)) in data.into_iter().enumerate() {
            let mut row = Row::new(i as u64);
            row.push(CellValue::from(name));
            row.push(CellValue::from(age));
            batch.push(row);
        }

        batch
    }

    #[test]
    fn test_sort_by_name_asc() {
        let mut batch = make_batch_for_sort();
        let keys = vec![SortKey {
            column: 0,
            order: SortOrder::Asc,
            nulls_first: true,
        }];

        sort_batch(&mut batch, &keys);

        assert_eq!(batch.rows[0].get_str(0), Some("Alice"));
        assert_eq!(batch.rows[1].get_str(0), Some("Bob"));
        assert_eq!(batch.rows[2].get_str(0), Some("Charlie"));
        assert_eq!(batch.rows[3].get_str(0), Some("Diana"));
    }

    #[test]
    fn test_sort_by_age_desc() {
        let mut batch = make_batch_for_sort();
        let keys = vec![SortKey {
            column: 1,
            order: SortOrder::Desc,
            nulls_first: false,
        }];

        sort_batch(&mut batch, &keys);

        assert_eq!(batch.rows[0].get_int(1), Some(35));
        assert_eq!(batch.rows[1].get_int(1), Some(30));
        assert_eq!(batch.rows[2].get_int(1), Some(28));
        assert_eq!(batch.rows[3].get_int(1), Some(25));
    }

    #[test]
    fn test_sort_with_nulls() {
        let mut batch = RowBatch::new(0);

        let mut r1 = Row::new(0);
        r1.push(CellValue::from(3_i64));
        batch.push(r1);

        let mut r2 = Row::new(1);
        r2.push(CellValue::Null);
        batch.push(r2);

        let mut r3 = Row::new(2);
        r3.push(CellValue::from(1_i64));
        batch.push(r3);

        let keys = vec![SortKey {
            column: 0,
            order: SortOrder::Asc,
            nulls_first: true,
        }];

        sort_batch(&mut batch, &keys);

        assert!(batch.rows[0].get(0).unwrap().is_null());
        assert_eq!(batch.rows[1].get_int(0), Some(1));
        assert_eq!(batch.rows[2].get_int(0), Some(3));
    }

    #[test]
    fn test_external_sort_roundtrip() {
        let mut batch1 = RowBatch::new(0);
        batch1.headers = Some(vec!["val".to_string()]);
        let mut r1 = Row::new(0);
        r1.push(CellValue::from(10_i64));
        let mut r2 = Row::new(1);
        r2.push(CellValue::from(5_i64));
        batch1.push(r1);
        batch1.push(r2);

        let mut batch2 = RowBatch::new(2);
        batch2.headers = Some(vec!["val".to_string()]);
        let mut r3 = Row::new(2);
        r3.push(CellValue::from(2_i64));
        let mut r4 = Row::new(3);
        r4.push(CellValue::from(8_i64));
        batch2.push(r3);
        batch2.push(r4);

        let stream = vec![Ok(batch1), Ok(batch2)].into_iter();
        let keys = vec![SortKey {
            column: 0,
            order: SortOrder::Asc,
            nulls_first: true,
        }];

        // Enforce 10 bytes memory limit to force multiple run splits
        let sorter = external_sort(stream, keys, 10, 2).unwrap();
        let results: Vec<RowBatch> = sorter.map(|r| r.unwrap()).collect();

        assert_eq!(results.len(), 2);
        // Page 1: 2, 5
        assert_eq!(results[0].rows[0].get_int(0), Some(2));
        assert_eq!(results[0].rows[1].get_int(0), Some(5));
        // Page 2: 8, 10
        assert_eq!(results[1].rows[0].get_int(0), Some(8));
        assert_eq!(results[1].rows[1].get_int(0), Some(10));
    }
}
