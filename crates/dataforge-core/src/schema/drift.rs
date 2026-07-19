// =============================================================================
// DataForge Core — Dynamic Schema Drift Handler
// =============================================================================
// Aligns incoming RowBatches to a target ColumnSchema, resolving missing,
// extra, or out-of-order fields dynamically.
// =============================================================================

use crate::types::{CellValue, Row, RowBatch, ColumnSchema};
use crate::error::{DataForgeError, Result};

/// Configuration options for handling schema drift.
#[derive(Debug, Clone)]
pub struct SchemaDriftConfig {
    /// True = error out on extra columns. False = drop/ignore extra columns.
    pub strict_extra_columns: bool,
    /// True = error out on missing columns. False = fill missing columns with Null.
    pub strict_missing_columns: bool,
}

impl Default for SchemaDriftConfig {
    fn default() -> Self {
        SchemaDriftConfig {
            strict_extra_columns: false,
            strict_missing_columns: false,
        }
    }
}

/// Handler for dynamic schema drift adjustments.
pub struct SchemaDriftHandler {
    target_schema: Vec<ColumnSchema>,
    config: SchemaDriftConfig,
}

impl SchemaDriftHandler {
    /// Create a new SchemaDriftHandler.
    pub fn new(target_schema: Vec<ColumnSchema>, config: SchemaDriftConfig) -> Self {
        SchemaDriftHandler {
            target_schema,
            config,
        }
    }

    /// Align an incoming RowBatch with the target schema.
    pub fn align_batch(&self, batch: &RowBatch) -> Result<RowBatch> {
        let headers = batch.headers.as_ref().ok_or_else(|| {
            DataForgeError::config("Cannot handle schema drift on RowBatch without headers")
        })?;

        // 1. Determine index mappings
        let mut target_indices = Vec::new();
        let mut missing_cols = Vec::new();

        for target_col in &self.target_schema {
            if let Some(idx) = headers.iter().position(|h| h.eq_ignore_ascii_case(&target_col.name)) {
                target_indices.push(Some(idx));
            } else {
                if self.config.strict_missing_columns {
                    return Err(DataForgeError::config(format!(
                        "Schema drift error: missing expected target column '{}'",
                        target_col.name
                    )));
                }
                target_indices.push(None);
                missing_cols.push(target_col.name.clone());
            }
        }

        // 2. Check for extra columns in strict mode
        if self.config.strict_extra_columns {
            for h in headers {
                let is_expected = self.target_schema.iter().any(|c| c.name.eq_ignore_ascii_case(h));
                if !is_expected {
                    return Err(DataForgeError::config(format!(
                        "Schema drift error: unexpected extra column '{}'",
                        h
                    )));
                }
            }
        }

        // 3. Rebuild headers and rows matching target schema
        let mut aligned_batch = RowBatch::new(batch.start_index);
        aligned_batch.headers = Some(self.target_schema.iter().map(|c| c.name.clone()).collect());
        aligned_batch.is_last = batch.is_last;

        for row in &batch.rows {
            let mut aligned_row = Row::new(row.index);
            for &idx_opt in &target_indices {
                match idx_opt {
                    Some(idx) => {
                        aligned_row.push(row.get(idx).cloned().unwrap_or(CellValue::Null));
                    }
                    None => {
                        aligned_row.push(CellValue::Null);
                    }
                }
            }
            aligned_batch.push(aligned_row);
        }

        Ok(aligned_batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DataType;

    #[test]
    fn test_schema_drift_alignment() {
        let target_schema = vec![
            ColumnSchema::new("name", DataType::String, 0),
            ColumnSchema::new("age", DataType::Int, 1),
            ColumnSchema::new("country", DataType::String, 2),
        ];

        let handler = SchemaDriftHandler::new(target_schema, SchemaDriftConfig::default());

        // Batch with out-of-order headers and extra column "salary", missing "country"
        let mut input_batch = RowBatch::new(0);
        input_batch.headers = Some(vec!["age".to_string(), "salary".to_string(), "name".to_string()]);

        let mut row = Row::new(0);
        row.push(CellValue::from(30_i64));
        row.push(CellValue::from(80000_i64));
        row.push(CellValue::from("Alice"));
        input_batch.push(row);

        let aligned = handler.align_batch(&input_batch).unwrap();

        assert_eq!(aligned.headers.unwrap(), vec!["name", "age", "country"]);
        assert_eq!(aligned.rows[0].get_str(0), Some("Alice"));
        assert_eq!(aligned.rows[0].get_int(1), Some(30));
        assert_eq!(aligned.rows[0].get(2), Some(&CellValue::Null)); // filled missing column with Null
    }
}
