// =============================================================================
// DataForge Core — Column Mapping / Transformation
// =============================================================================
// Functions for transforming cell values within rows:
// - Select specific columns
// - Rename columns
// - Add computed columns
// - Type coercion
// =============================================================================

use compact_str::CompactString;

use crate::types::{CellValue, Row, RowBatch};

/// A column transformation function type.
pub type MapFn = Box<dyn Fn(&Row) -> CellValue + Send + Sync>;

/// Select specific columns from a batch, discarding the rest.
///
/// This is applied at the batch level for efficiency (avoids
/// per-row function call overhead for simple projection).
pub fn select_columns(batch: &mut RowBatch, indices: &[usize]) {
    for row in &mut batch.rows {
        let selected: Vec<CellValue> = indices
            .iter()
            .map(|&i| row.get(i).cloned().unwrap_or(CellValue::Null))
            .collect();
        row.cells = selected.into();
    }

    // Update headers
    if let Some(headers) = &batch.headers {
        let selected_headers: Vec<String> = indices
            .iter()
            .map(|&i| headers.get(i).cloned().unwrap_or_default())
            .collect();
        batch.headers = Some(selected_headers);
    }
}

/// Reorder columns in a batch to match a new index sequence.
pub fn reorder_columns(batch: &mut RowBatch, new_order: &[usize]) {
    select_columns(batch, new_order);
}

/// Rename columns in a batch by applying a name mapping.
///
/// # Arguments
/// * `batch` - The batch to modify
/// * `renames` - Pairs of (old_name, new_name)
pub fn rename_columns(batch: &mut RowBatch, renames: &[(&str, &str)]) {
    if let Some(headers) = &mut batch.headers {
        for header in headers.iter_mut() {
            for (old, new) in renames {
                if header == old {
                    *header = new.to_string();
                    break;
                }
            }
        }
    }
}

/// Add a computed column to each row in a batch.
///
/// The computation function receives the original row and returns
/// the new cell value. The new column is appended to the end.
///
/// # Arguments
/// * `batch` - The batch to modify
/// * `name` - Name for the new column
/// * `compute` - Function to compute the new column value from each row
pub fn add_computed_column(
    batch: &mut RowBatch,
    name: &str,
    compute: &(dyn Fn(&Row) -> CellValue + Send + Sync),
) {
    // Compute values first, then modify rows
    let new_values: Vec<CellValue> = batch.rows.iter().map(compute).collect();

    for (row, value) in batch.rows.iter_mut().zip(new_values) {
        row.push(value);
    }

    if let Some(headers) = &mut batch.headers {
        headers.push(name.to_string());
    }
}

/// Coerce all values in a specific column to a target type.
///
/// Best-effort conversion:
/// - String "42" → Int(42)
/// - Int(42) → Float(42.0)
/// - Float(3.14) → String("3.14")
/// - Failed conversions → Null
pub fn coerce_column(batch: &mut RowBatch, column: usize, target: &crate::types::DataType) {
    use crate::types::DataType;

    for row in &mut batch.rows {
        if let Some(cell) = row.get_mut(column) {
            *cell = match target {
                DataType::String => CellValue::String(CompactString::new(&cell.to_display_string())),
                DataType::Int => match cell.as_int() {
                    Some(v) => CellValue::Int(v),
                    None => match cell.as_str().and_then(|s| s.parse::<i64>().ok()) {
                        Some(v) => CellValue::Int(v),
                        None => CellValue::Null,
                    },
                },
                DataType::Float => match cell.as_float() {
                    Some(v) => CellValue::Float(v),
                    None => match cell.as_str().and_then(|s| s.parse::<f64>().ok()) {
                        Some(v) => CellValue::Float(v),
                        None => CellValue::Null,
                    },
                },
                DataType::Bool => match cell {
                    CellValue::Bool(v) => CellValue::Bool(*v),
                    CellValue::Int(v) => CellValue::Bool(*v != 0),
                    CellValue::String(s) => {
                        CellValue::Bool(matches!(s.to_lowercase().as_str(), "true" | "yes" | "1"))
                    }
                    _ => CellValue::Null,
                },
                DataType::Date => match cell {
                    CellValue::Date(d) => CellValue::Date(*d),
                    CellValue::DateTime(dt) => CellValue::Date(dt.date()),
                    CellValue::String(s) => {
                        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                            CellValue::Date(d)
                        } else if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y/%m/%d") {
                            CellValue::Date(d)
                        } else if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%d-%m-%Y") {
                            CellValue::Date(d)
                        } else if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%d/%m/%Y") {
                            CellValue::Date(d)
                        } else {
                            CellValue::Null
                        }
                    }
                    _ => CellValue::Null,
                },
                DataType::DateTime => match cell {
                    CellValue::DateTime(dt) => CellValue::DateTime(*dt),
                    CellValue::Date(d) => CellValue::DateTime(d.and_hms_opt(0, 0, 0).unwrap_or_default()),
                    CellValue::String(s) => {
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                            CellValue::DateTime(dt.naive_utc())
                        } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
                            CellValue::DateTime(dt)
                        } else {
                            CellValue::Null
                        }
                    }
                    _ => CellValue::Null,
                },
                _ => cell.clone(),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_batch() -> RowBatch {
        let mut batch = RowBatch::new(0);
        batch.headers = Some(vec!["name".into(), "age".into(), "city".into()]);

        let mut r1 = Row::new(0);
        r1.push(CellValue::from("Alice"));
        r1.push(CellValue::from(30_i64));
        r1.push(CellValue::from("NYC"));
        batch.push(r1);

        let mut r2 = Row::new(1);
        r2.push(CellValue::from("Bob"));
        r2.push(CellValue::from(25_i64));
        r2.push(CellValue::from("LA"));
        batch.push(r2);

        batch
    }

    #[test]
    fn test_select_columns() {
        let mut batch = make_batch();
        select_columns(&mut batch, &[0, 2]); // name, city

        assert_eq!(batch.rows[0].len(), 2);
        assert_eq!(batch.rows[0].get_str(0), Some("Alice"));
        assert_eq!(batch.rows[0].get_str(1), Some("NYC"));
        assert_eq!(batch.headers.as_ref().unwrap(), &["name", "city"]);
    }

    #[test]
    fn test_rename_columns() {
        let mut batch = make_batch();
        rename_columns(&mut batch, &[("name", "full_name"), ("age", "years")]);

        let headers = batch.headers.as_ref().unwrap();
        assert_eq!(headers[0], "full_name");
        assert_eq!(headers[1], "years");
        assert_eq!(headers[2], "city");
    }

    #[test]
    fn test_add_computed_column() {
        let mut batch = make_batch();
        add_computed_column(&mut batch, "senior", &|row| {
            let age = row.get_int(1).unwrap_or(0);
            CellValue::Bool(age >= 30)
        });

        assert_eq!(batch.rows[0].len(), 4);
        assert_eq!(batch.rows[0].get(3).unwrap().as_bool(), Some(true));
        assert_eq!(batch.rows[1].get(3).unwrap().as_bool(), Some(false));
        assert_eq!(
            batch.headers.as_ref().unwrap().last().unwrap(),
            "senior"
        );
    }

    #[test]
    fn test_coerce_column_to_string() {
        let mut batch = make_batch();
        coerce_column(&mut batch, 1, &crate::types::DataType::String);

        assert_eq!(batch.rows[0].get_str(1), Some("30"));
    }

    #[test]
    fn test_coerce_column_to_date() {
        let mut batch = RowBatch::new(0);
        batch.headers = Some(vec!["date_str".into()]);
        let mut r1 = Row::new(0);
        r1.push(CellValue::from("2026-07-18"));
        batch.push(r1);

        coerce_column(&mut batch, 0, &crate::types::DataType::Date);
        let val = batch.rows[0].get(0).unwrap();
        assert!(val.as_date().is_some());
        assert_eq!(val.to_display_string(), "2026-07-18");
    }
}
