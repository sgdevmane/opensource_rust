// =============================================================================
// DataForge Core — Row Filter
// =============================================================================
// Predicate-based row filtering for streaming pipelines.
// Filters are applied per-row within each batch, maintaining constant memory.
// =============================================================================

use crate::types::{CellValue, Row};

/// A predicate function that determines whether a row should be kept.
pub type FilterFn = Box<dyn Fn(&Row) -> bool + Send + Sync>;

/// Comparison operator for simple column-based filters.
#[derive(Debug, Clone)]
pub enum CompareOp {
    /// Equal to
    Eq,
    /// Not equal to
    NotEq,
    /// Greater than
    Gt,
    /// Greater than or equal
    Gte,
    /// Less than
    Lt,
    /// Less than or equal
    Lte,
    /// Contains substring (string columns only)
    Contains,
    /// Starts with prefix (string columns only)
    StartsWith,
    /// Ends with suffix (string columns only)
    EndsWith,
    /// Is null/empty
    IsNull,
    /// Is not null/empty
    IsNotNull,
}

/// A column-based filter condition.
///
/// Describes a comparison between a column's value and a reference value.
/// Used for declarative filter construction (useful for FFI/WASM where
/// closures can't cross language boundaries).
#[derive(Debug, Clone)]
pub struct ColumnFilter {
    /// Column index to filter on (0-based)
    pub column: usize,
    /// Comparison operator
    pub op: CompareOp,
    /// Reference value to compare against
    pub value: CellValue,
}

impl ColumnFilter {
    /// Create a new column filter.
    pub fn new(column: usize, op: CompareOp, value: CellValue) -> Self {
        ColumnFilter { column, op, value }
    }

    /// Evaluate this filter against a row.
    pub fn matches(&self, row: &Row) -> bool {
        let cell = row.get(self.column).unwrap_or(&CellValue::Null);

        match &self.op {
            CompareOp::IsNull => cell.is_null(),
            CompareOp::IsNotNull => !cell.is_null(),
            CompareOp::Eq => cell_equals(cell, &self.value),
            CompareOp::NotEq => !cell_equals(cell, &self.value),
            CompareOp::Gt => cell_compare(cell, &self.value).is_some_and(|o| o == std::cmp::Ordering::Greater),
            CompareOp::Gte => cell_compare(cell, &self.value).is_some_and(|o| o != std::cmp::Ordering::Less),
            CompareOp::Lt => cell_compare(cell, &self.value).is_some_and(|o| o == std::cmp::Ordering::Less),
            CompareOp::Lte => cell_compare(cell, &self.value).is_some_and(|o| o != std::cmp::Ordering::Greater),
            CompareOp::Contains => {
                if let (Some(a), Some(b)) = (cell.as_str(), self.value.as_str()) {
                    a.contains(b)
                } else {
                    false
                }
            }
            CompareOp::StartsWith => {
                if let (Some(a), Some(b)) = (cell.as_str(), self.value.as_str()) {
                    a.starts_with(b)
                } else {
                    false
                }
            }
            CompareOp::EndsWith => {
                if let (Some(a), Some(b)) = (cell.as_str(), self.value.as_str()) {
                    a.ends_with(b)
                } else {
                    false
                }
            }
        }
    }
}

/// Compare two CellValues for equality.
fn cell_equals(a: &CellValue, b: &CellValue) -> bool {
    match (a, b) {
        (CellValue::Null, CellValue::Null) => true,
        (CellValue::Bool(a), CellValue::Bool(b)) => a == b,
        (CellValue::Int(a), CellValue::Int(b)) => a == b,
        (CellValue::Float(a), CellValue::Float(b)) => (a - b).abs() < f64::EPSILON,
        (CellValue::Int(a), CellValue::Float(b)) | (CellValue::Float(b), CellValue::Int(a)) => {
            (*a as f64 - b).abs() < f64::EPSILON
        }
        (CellValue::String(a), CellValue::String(b)) => a == b,
        (CellValue::DateTime(a), CellValue::DateTime(b)) => a == b,
        (CellValue::Date(a), CellValue::Date(b)) => a == b,
        (CellValue::Time(a), CellValue::Time(b)) => a == b,
        _ => false,
    }
}

/// Compare two CellValues for ordering.
fn cell_compare(a: &CellValue, b: &CellValue) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (CellValue::Int(a), CellValue::Int(b)) => Some(a.cmp(b)),
        (CellValue::Float(a), CellValue::Float(b)) => a.partial_cmp(b),
        (CellValue::Int(a), CellValue::Float(b)) => (*a as f64).partial_cmp(b),
        (CellValue::Float(a), CellValue::Int(b)) => a.partial_cmp(&(*b as f64)),
        (CellValue::String(a), CellValue::String(b)) => Some(a.cmp(b)),
        (CellValue::DateTime(a), CellValue::DateTime(b)) => Some(a.cmp(b)),
        (CellValue::Date(a), CellValue::Date(b)) => Some(a.cmp(b)),
        (CellValue::Time(a), CellValue::Time(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_row(values: Vec<CellValue>) -> Row {
        let mut row = Row::new(0);
        for v in values {
            row.push(v);
        }
        row
    }

    #[test]
    fn test_column_filter_eq() {
        let filter = ColumnFilter::new(1, CompareOp::Eq, CellValue::from(30_i64));
        let row = make_row(vec![CellValue::from("Alice"), CellValue::from(30_i64)]);
        assert!(filter.matches(&row));

        let row2 = make_row(vec![CellValue::from("Bob"), CellValue::from(25_i64)]);
        assert!(!filter.matches(&row2));
    }

    #[test]
    fn test_column_filter_gt() {
        let filter = ColumnFilter::new(0, CompareOp::Gt, CellValue::from(50.0_f64));
        let row = make_row(vec![CellValue::from(100.0_f64)]);
        assert!(filter.matches(&row));

        let row2 = make_row(vec![CellValue::from(30.0_f64)]);
        assert!(!filter.matches(&row2));
    }

    #[test]
    fn test_column_filter_contains() {
        let filter = ColumnFilter::new(0, CompareOp::Contains, CellValue::from("lic"));
        let row = make_row(vec![CellValue::from("Alice")]);
        assert!(filter.matches(&row));
    }

    #[test]
    fn test_column_filter_is_null() {
        let filter = ColumnFilter::new(0, CompareOp::IsNull, CellValue::Null);
        let row = make_row(vec![CellValue::Null]);
        assert!(filter.matches(&row));

        let row2 = make_row(vec![CellValue::from("value")]);
        assert!(!filter.matches(&row2));
    }
}
