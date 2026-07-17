// =============================================================================
// DataForge Core — External Merge Sort
// =============================================================================
// Sorts datasets larger than available memory using external merge sort:
// 1. Split input into sorted chunks that fit in memory
// 2. Write chunks to temporary files
// 3. Merge sorted chunks using a k-way merge with a min-heap
// =============================================================================

use std::cmp::Ordering;

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
}
