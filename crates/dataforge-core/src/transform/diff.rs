// =============================================================================
// DataForge Core — Workbook & Dataset Diff Audit Engine
// =============================================================================
// Compares two tabular datasets and detects added, removed, and modified rows.
// =============================================================================

use crate::types::{CellValue, RowBatch};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DiffKind {
    Added,
    Deleted,
    Modified {
        cell_diffs: Vec<CellDiff>,
    },
    Unchanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CellDiff {
    pub column_index: usize,
    pub old_value: CellValue,
    pub new_value: CellValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RowDiff {
    pub row_index: usize,
    pub kind: DiffKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiffReport {
    pub total_left_rows: usize,
    pub total_right_rows: usize,
    pub added_count: usize,
    pub deleted_count: usize,
    pub modified_count: usize,
    pub unchanged_count: usize,
    pub row_diffs: Vec<RowDiff>,
}

pub struct WorkbookDiffEngine;

impl WorkbookDiffEngine {
    /// Compare two RowBatches by matching primary key column index or row index.
    pub fn diff_batches(left: &RowBatch, right: &RowBatch, key_col_idx: Option<usize>) -> DiffReport {
        let mut report = DiffReport {
            total_left_rows: left.rows.len(),
            total_right_rows: right.rows.len(),
            ..Default::default()
        };

        if let Some(key_idx) = key_col_idx {
            // Key-based diffing using map
            use std::collections::HashMap;
            let mut left_map = HashMap::new();
            for (idx, row) in left.rows.iter().enumerate() {
                if let Some(key) = row.cells.get(key_idx) {
                    left_map.insert(key.clone(), (idx, row));
                }
            }

            let mut right_matched_left_keys = std::collections::HashSet::new();

            for (r_idx, r_row) in right.rows.iter().enumerate() {
                if let Some(key) = r_row.cells.get(key_idx) {
                    if let Some(&(l_idx, l_row)) = left_map.get(key) {
                        right_matched_left_keys.insert(key.clone());
                        let cell_diffs = Self::diff_row_cells(l_row, r_row);
                        if cell_diffs.is_empty() {
                            report.unchanged_count += 1;
                        } else {
                            report.modified_count += 1;
                            report.row_diffs.push(RowDiff {
                                row_index: l_idx,
                                kind: DiffKind::Modified { cell_diffs },
                            });
                        }
                    } else {
                        report.added_count += 1;
                        report.row_diffs.push(RowDiff {
                            row_index: r_idx,
                            kind: DiffKind::Added,
                        });
                    }
                }
            }

            for (key, (l_idx, _)) in left_map {
                if !right_matched_left_keys.contains(&key) {
                    report.deleted_count += 1;
                    report.row_diffs.push(RowDiff {
                        row_index: l_idx,
                        kind: DiffKind::Deleted,
                    });
                }
            }
        } else {
            // Row index positional diffing
            let max_rows = left.rows.len().max(right.rows.len());
            for idx in 0..max_rows {
                match (left.rows.get(idx), right.rows.get(idx)) {
                    (Some(l_row), Some(r_row)) => {
                        let cell_diffs = Self::diff_row_cells(l_row, r_row);
                        if cell_diffs.is_empty() {
                            report.unchanged_count += 1;
                        } else {
                            report.modified_count += 1;
                            report.row_diffs.push(RowDiff {
                                row_index: idx,
                                kind: DiffKind::Modified { cell_diffs },
                            });
                        }
                    }
                    (Some(_), None) => {
                        report.deleted_count += 1;
                        report.row_diffs.push(RowDiff {
                            row_index: idx,
                            kind: DiffKind::Deleted,
                        });
                    }
                    (None, Some(_)) => {
                        report.added_count += 1;
                        report.row_diffs.push(RowDiff {
                            row_index: idx,
                            kind: DiffKind::Added,
                        });
                    }
                    (None, None) => {}
                }
            }
        }

        report
    }

    fn diff_row_cells(l_row: &crate::types::Row, r_row: &crate::types::Row) -> Vec<CellDiff> {
        let max_cols = l_row.cells.len().max(r_row.cells.len());
        let mut diffs = Vec::new();
        for col in 0..max_cols {
            let l_val = l_row.cells.get(col).cloned().unwrap_or(CellValue::Null);
            let r_val = r_row.cells.get(col).cloned().unwrap_or(CellValue::Null);
            if l_val != r_val {
                diffs.push(CellDiff {
                    column_index: col,
                    old_value: l_val,
                    new_value: r_val,
                });
            }
        }
        diffs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Row, Schema};
    use compact_str::CompactString;

    #[test]
    fn test_diff_engine() {
        let left = RowBatch {
            schema: Schema { fields: vec![] },
            rows: vec![
                Row { cells: vec![CellValue::Int(1), CellValue::String(CompactString::new("Alice"))] },
                Row { cells: vec![CellValue::Int(2), CellValue::String(CompactString::new("Bob"))] },
            ],
        };

        let right = RowBatch {
            schema: Schema { fields: vec![] },
            rows: vec![
                Row { cells: vec![CellValue::Int(1), CellValue::String(CompactString::new("Alice Smith"))] },
                Row { cells: vec![CellValue::Int(3), CellValue::String(CompactString::new("Charlie"))] },
            ],
        };

        let report = WorkbookDiffEngine::diff_batches(&left, &right, Some(0));
        assert_eq!(report.modified_count, 1);
        assert_eq!(report.added_count, 1);
        assert_eq!(report.deleted_count, 1);
    }
}
