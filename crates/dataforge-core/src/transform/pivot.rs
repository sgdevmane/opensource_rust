// =============================================================================
// DataForge Core — Streaming Pivot Tables Stage
// =============================================================================
// Aggregates and pivots row streams into cross-tabulated RowBatches.
// =============================================================================

use std::collections::{BTreeMap, BTreeSet};
use crate::types::{CellValue, Row, RowBatch};

/// Aggregate operation to perform on the pivoted values.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum PivotAggregate {
    /// Sum numeric field values.
    Sum,
    /// Count occurrences of entries.
    Count,
}

/// Streaming Pivot Table generator.
pub struct PivotTable {
    row_key_idx: usize,
    col_key_idx: usize,
    val_idx: usize,
    agg: PivotAggregate,
    // Maps row_key -> (column_key -> accumulated_value)
    data: BTreeMap<String, BTreeMap<String, f64>>,
}

impl PivotTable {
    /// Create a new PivotTable configuration.
    pub fn new(
        row_key_idx: usize,
        col_key_idx: usize,
        val_idx: usize,
        agg: PivotAggregate,
    ) -> Self {
        PivotTable {
            row_key_idx,
            col_key_idx,
            val_idx,
            agg,
            data: BTreeMap::new(),
        }
    }

    /// Add a RowBatch to accumulate values in the pivot.
    pub fn add_batch(&mut self, batch: &RowBatch) {
        for row in &batch.rows {
            let row_key = match row.get(self.row_key_idx) {
                Some(cell) if !cell.is_null() => cell.to_display_string(),
                _ => continue,
            };
            let col_key = match row.get(self.col_key_idx) {
                Some(cell) if !cell.is_null() => cell.to_display_string(),
                _ => continue,
            };
            let val = match row.get(self.val_idx) {
                Some(CellValue::Int(v)) => *v as f64,
                Some(CellValue::Float(v)) => *v,
                _ => 0.0,
            };

            let entry = self.data.entry(row_key).or_default();
            let current_val = entry.entry(col_key).or_insert(0.0);
            match self.agg {
                PivotAggregate::Sum => {
                    *current_val += val;
                }
                PivotAggregate::Count => {
                    *current_val += 1.0;
                }
            }
        }
    }

    /// Finalize the accumulation and yield a pivoted RowBatch.
    pub fn finish(self, row_header_name: &str) -> RowBatch {
        let mut unique_cols = BTreeSet::new();
        for col_map in self.data.values() {
            for col_key in col_map.keys() {
                unique_cols.insert(col_key.clone());
            }
        }
        let cols_list: Vec<String> = unique_cols.into_iter().collect();

        // Build headers: [RowHeader, Col1, Col2, ...]
        let mut headers = vec![row_header_name.to_string()];
        headers.extend(cols_list.clone());

        let mut out_batch = RowBatch::new(0);
        out_batch.headers = Some(headers);

        for (row_idx, (row_key, col_map)) in self.data.into_iter().enumerate() {
            let mut row = Row::new(row_idx as u64);
            row.push(CellValue::from(row_key));
            for col_key in &cols_list {
                if let Some(&val) = col_map.get(col_key) {
                    row.push(CellValue::from(val));
                } else {
                    row.push(CellValue::Null);
                }
            }
            out_batch.push(row);
        }

        out_batch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pivot_table_sum() {
        let mut pivot = PivotTable::new(0, 1, 2, PivotAggregate::Sum);

        let mut batch = RowBatch::new(0);
        batch.headers = Some(vec!["Year".to_string(), "Product".to_string(), "Sales".to_string()]);
        
        let mut r1 = Row::new(0);
        r1.push(CellValue::from("2021"));
        r1.push(CellValue::from("Apple"));
        r1.push(CellValue::from(100_i64));
        batch.push(r1);

        let mut r2 = Row::new(1);
        r2.push(CellValue::from("2021"));
        r2.push(CellValue::from("Orange"));
        r2.push(CellValue::from(150_i64));
        batch.push(r2);

        let mut r3 = Row::new(2);
        r3.push(CellValue::from("2022"));
        r3.push(CellValue::from("Apple"));
        r3.push(CellValue::from(200_i64));
        batch.push(r3);

        pivot.add_batch(&batch);
        let result = pivot.finish("Year");

        assert_eq!(result.len(), 2);
        assert_eq!(result.headers.as_ref().unwrap(), &["Year", "Apple", "Orange"]);
        
        // Year 2021 Apple should be 100, Orange should be 150
        assert_eq!(result.rows[0].get_str(0), Some("2021"));
        assert_eq!(result.rows[0].get_float(1), Some(100.0));
        assert_eq!(result.rows[0].get_float(2), Some(150.0));

        // Year 2022 Apple should be 200, Orange should be Null (sparse)
        assert_eq!(result.rows[1].get_str(0), Some("2022"));
        assert_eq!(result.rows[1].get_float(1), Some(200.0));
        assert!(result.rows[1].get(2).unwrap().is_null());
    }
}
