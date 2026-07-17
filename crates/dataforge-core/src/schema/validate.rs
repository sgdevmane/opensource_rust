// =============================================================================
// DataForge Core — Schema Validation
// =============================================================================
// Validates row batches against a defined schema.
// Reports type mismatches, null violations, and length violations.
// =============================================================================

use crate::error::{DataForgeError, Result};
use crate::types::{CellValue, ColumnSchema, DataType, RowBatch};

/// Validation error for a single cell.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub row: u64,
    pub column: String,
    pub message: String,
}

/// Validate a batch against the provided schema.
///
/// Returns a list of validation errors (empty = valid).
/// If `strict` is true, returns an error on the first mismatch.
pub fn validate_batch(
    batch: &RowBatch,
    schema: &[ColumnSchema],
    strict: bool,
) -> Result<Vec<ValidationError>> {
    let mut errors = Vec::new();

    for row in &batch.rows {
        for col_schema in schema {
            let cell = row.get(col_schema.index).unwrap_or(&CellValue::Null);

            // Check nullability
            if cell.is_null() && !col_schema.nullable {
                let err = ValidationError {
                    row: row.index,
                    column: col_schema.name.clone(),
                    message: "null value in non-nullable column".to_string(),
                };

                if strict {
                    return Err(DataForgeError::Schema {
                        row: row.index,
                        column: col_schema.name.clone(),
                        message: err.message,
                    });
                }
                errors.push(err);
                continue;
            }

            if cell.is_null() {
                continue; // Null in nullable column is fine
            }

            // Check type match
            let actual_type = cell.data_type();
            if !types_compatible(actual_type, col_schema.data_type) {
                let err = ValidationError {
                    row: row.index,
                    column: col_schema.name.clone(),
                    message: format!(
                        "expected {}, got {}",
                        col_schema.data_type, actual_type
                    ),
                };

                if strict {
                    return Err(DataForgeError::Schema {
                        row: row.index,
                        column: col_schema.name.clone(),
                        message: err.message,
                    });
                }
                errors.push(err);
            }

            // Check max length for strings
            if let Some(max_len) = col_schema.max_length {
                if let Some(s) = cell.as_str() {
                    if s.len() > max_len {
                        let err = ValidationError {
                            row: row.index,
                            column: col_schema.name.clone(),
                            message: format!(
                                "string length {} exceeds max {}",
                                s.len(),
                                max_len
                            ),
                        };

                        if strict {
                            return Err(DataForgeError::Schema {
                                row: row.index,
                                column: col_schema.name.clone(),
                                message: err.message,
                            });
                        }
                        errors.push(err);
                    }
                }
            }
        }
    }

    Ok(errors)
}

/// Check if two data types are compatible (allows numeric widening).
fn types_compatible(actual: DataType, expected: DataType) -> bool {
    if actual == expected {
        return true;
    }

    // Allow numeric widening
    matches!(
        (actual, expected),
        (DataType::Int, DataType::Float)
            | (DataType::Date, DataType::DateTime)
            | (DataType::Null, _)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CellValue, Row, RowBatch};

    #[test]
    fn test_valid_batch() {
        let schema = vec![
            ColumnSchema::new("name", DataType::String, 0).with_nullable(false),
            ColumnSchema::new("age", DataType::Int, 1),
        ];

        let mut batch = RowBatch::new(0);
        let mut row = Row::new(0);
        row.push(CellValue::from("Alice"));
        row.push(CellValue::from(30_i64));
        batch.push(row);

        let errors = validate_batch(&batch, &schema, false).unwrap();
        assert!(errors.is_empty());
    }

    #[test]
    fn test_null_violation() {
        let schema = vec![
            ColumnSchema::new("name", DataType::String, 0).with_nullable(false),
        ];

        let mut batch = RowBatch::new(0);
        let mut row = Row::new(0);
        row.push(CellValue::Null);
        batch.push(row);

        let errors = validate_batch(&batch, &schema, false).unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("null"));
    }

    #[test]
    fn test_type_mismatch() {
        let schema = vec![ColumnSchema::new("age", DataType::Int, 0)];

        let mut batch = RowBatch::new(0);
        let mut row = Row::new(0);
        row.push(CellValue::from("not a number"));
        batch.push(row);

        let errors = validate_batch(&batch, &schema, false).unwrap();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_strict_mode() {
        let schema = vec![
            ColumnSchema::new("name", DataType::String, 0).with_nullable(false),
        ];

        let mut batch = RowBatch::new(0);
        let mut row = Row::new(0);
        row.push(CellValue::Null);
        batch.push(row);

        let result = validate_batch(&batch, &schema, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_numeric_widening() {
        let schema = vec![ColumnSchema::new("value", DataType::Float, 0)];

        let mut batch = RowBatch::new(0);
        let mut row = Row::new(0);
        row.push(CellValue::from(42_i64)); // Int in Float column
        batch.push(row);

        let errors = validate_batch(&batch, &schema, false).unwrap();
        assert!(errors.is_empty()); // Int → Float is compatible
    }
}
