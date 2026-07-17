// =============================================================================
// DataForge Core — Schema Inference
// =============================================================================
// Automatically detect column data types by sampling rows.
// Uses a voting/confidence system to handle ambiguous columns.
// =============================================================================

use std::collections::HashMap;

use crate::types::{ColumnSchema, DataType, RowBatch};

/// Infer column schemas from a collection of batches.
///
/// Examines up to `sample_size` rows and determines the most likely
/// data type for each column using a voting system.
///
/// # Arguments
/// * `batches` - Batches of rows to sample from
/// * `headers` - Optional column headers
/// * `sample_size` - Maximum number of rows to examine
///
/// # Returns
/// A vector of `ColumnSchema` with inferred types.
pub fn infer_schema(
    batches: &[RowBatch],
    headers: Option<&[String]>,
    sample_size: usize,
) -> Vec<ColumnSchema> {
    // Count type occurrences per column
    let mut type_counts: Vec<HashMap<DataType, u64>> = Vec::new();
    let mut null_counts: Vec<u64> = Vec::new();
    let mut max_columns = 0usize;
    let mut rows_sampled = 0usize;

    for batch in batches {
        for row in &batch.rows {
            if rows_sampled >= sample_size {
                break;
            }

            // Expand counters if this row has more columns
            while type_counts.len() < row.len() {
                type_counts.push(HashMap::new());
                null_counts.push(0);
            }
            max_columns = max_columns.max(row.len());

            for (col_idx, cell) in row.cells.iter().enumerate() {
                if cell.is_null() {
                    null_counts[col_idx] += 1;
                } else {
                    let dt = cell.data_type();
                    *type_counts[col_idx].entry(dt).or_insert(0) += 1;
                }
            }

            rows_sampled += 1;
        }

        if rows_sampled >= sample_size {
            break;
        }
    }

    // Determine the best type for each column
    let mut schemas = Vec::with_capacity(max_columns);

    for col_idx in 0..max_columns {
        let name = headers
            .and_then(|h| h.get(col_idx))
            .map(|s| s.clone())
            .unwrap_or_else(|| format!("column_{}", col_idx));

        let counts = &type_counts[col_idx];
        let null_count = null_counts[col_idx];
        let total_non_null: u64 = counts.values().sum();

        let data_type = if total_non_null == 0 {
            DataType::String // All nulls — default to string
        } else {
            // Find the most common type
            let (best_type, _) = counts
                .iter()
                .max_by_key(|(_, count)| *count)
                .unwrap();

            // Apply type promotion rules
            promote_type(*best_type, counts)
        };

        let nullable = null_count > 0;

        schemas.push(ColumnSchema::new(name, data_type, col_idx).with_nullable(nullable));
    }

    schemas
}

/// Promote types based on the observed type distribution.
///
/// Rules:
/// - If a column has both Int and Float, promote to Float
/// - If a column has numeric and string, promote to String
/// - Single type: use that type
fn promote_type(primary: DataType, counts: &HashMap<DataType, u64>) -> DataType {
    let types: Vec<&DataType> = counts.keys().collect();

    if types.len() == 1 {
        return primary;
    }

    // Int + Float → Float
    if types.contains(&&DataType::Int) && types.contains(&&DataType::Float) {
        return DataType::Float;
    }

    // Any numeric + String → String
    if types.contains(&&DataType::String) {
        return DataType::String;
    }

    // DateTime + Date → DateTime
    if types.contains(&&DataType::DateTime) && types.contains(&&DataType::Date) {
        return DataType::DateTime;
    }

    primary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CellValue, Row, RowBatch};

    #[test]
    fn test_infer_basic_types() {
        let mut batch = RowBatch::new(0);

        let data = vec![
            vec![CellValue::from("Alice"), CellValue::from(30_i64), CellValue::from(3.14_f64)],
            vec![CellValue::from("Bob"), CellValue::from(25_i64), CellValue::from(2.71_f64)],
            vec![CellValue::from("Charlie"), CellValue::from(35_i64), CellValue::from(1.41_f64)],
        ];

        for (i, cells) in data.into_iter().enumerate() {
            let mut row = Row::new(i as u64);
            for c in cells {
                row.push(c);
            }
            batch.push(row);
        }

        let headers = vec!["name".into(), "age".into(), "value".into()];
        let schema = infer_schema(&[batch], Some(&headers), 100);

        assert_eq!(schema.len(), 3);
        assert_eq!(schema[0].data_type, DataType::String);
        assert_eq!(schema[1].data_type, DataType::Int);
        assert_eq!(schema[2].data_type, DataType::Float);
    }

    #[test]
    fn test_infer_with_nulls() {
        let mut batch = RowBatch::new(0);

        let mut r1 = Row::new(0);
        r1.push(CellValue::from(42_i64));
        batch.push(r1);

        let mut r2 = Row::new(1);
        r2.push(CellValue::Null);
        batch.push(r2);

        let schema = infer_schema(&[batch], None, 100);
        assert_eq!(schema[0].data_type, DataType::Int);
        assert!(schema[0].nullable);
    }

    #[test]
    fn test_infer_mixed_int_float() {
        let mut batch = RowBatch::new(0);

        let mut r1 = Row::new(0);
        r1.push(CellValue::from(42_i64));
        batch.push(r1);

        let mut r2 = Row::new(1);
        r2.push(CellValue::from(3.14_f64));
        batch.push(r2);

        let schema = infer_schema(&[batch], None, 100);
        assert_eq!(schema[0].data_type, DataType::Float); // Promoted
    }
}
