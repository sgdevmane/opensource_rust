// =============================================================================
// DataForge Core — Composable Transform Pipeline
// =============================================================================
// High-level API for chaining transformations on streaming data.
// The pipeline applies filter → map → aggregate → sort stages
// to each batch as it flows through.
// =============================================================================

use crate::transform::filter::ColumnFilter;
use crate::transform::map;
use crate::transform::sort::{SortKey, sort_batch};
use crate::types::{CellValue, Row, RowBatch};

/// A composable transformation pipeline.
///
/// Stages are applied in order to each batch:
/// 1. Filters (remove rows)
/// 2. Column selection (remove columns)
/// 3. Column renames
/// 4. Computed columns (add columns)
/// 5. Sorting (within each batch)
///
/// Aggregation is handled separately since it reduces batches to a single row.
pub struct Pipeline {
    /// Row filter conditions
    filters: Vec<ColumnFilter>,

    /// Custom filter functions
    custom_filters: Vec<Box<dyn Fn(&Row) -> bool + Send + Sync>>,

    /// Column indices to select (None = all)
    select_columns: Option<Vec<usize>>,

    /// Column renames: (old_name, new_name)
    renames: Vec<(String, String)>,

    /// Computed columns: (name, compute_fn)
    computed_columns: Vec<(String, Box<dyn Fn(&Row) -> CellValue + Send + Sync>)>,

    /// Sort keys (applied per-batch)
    sort_keys: Vec<SortKey>,

    /// Type coercions: (column_index, target_type)
    coercions: Vec<(usize, crate::types::DataType)>,

    /// PII masking rules: (column_index, strategy)
    masks: Vec<(usize, crate::transform::mask::MaskingStrategy)>,
}

impl Pipeline {
    /// Create a new empty pipeline (pass-through).
    pub fn new() -> Self {
        Pipeline {
            filters: Vec::new(),
            custom_filters: Vec::new(),
            select_columns: None,
            renames: Vec::new(),
            computed_columns: Vec::new(),
            sort_keys: Vec::new(),
            coercions: Vec::new(),
            masks: Vec::new(),
        }
    }

    /// Add a column-based filter condition.
    pub fn filter(mut self, filter: ColumnFilter) -> Self {
        self.filters.push(filter);
        self
    }

    /// Add a custom filter function.
    pub fn filter_fn(mut self, f: impl Fn(&Row) -> bool + Send + Sync + 'static) -> Self {
        self.custom_filters.push(Box::new(f));
        self
    }

    /// Select specific columns by index.
    pub fn select(mut self, columns: Vec<usize>) -> Self {
        self.select_columns = Some(columns);
        self
    }

    /// Rename a column.
    pub fn rename(mut self, old: impl Into<String>, new: impl Into<String>) -> Self {
        self.renames.push((old.into(), new.into()));
        self
    }

    /// Add a computed column.
    pub fn add_column(
        mut self,
        name: impl Into<String>,
        compute: impl Fn(&Row) -> CellValue + Send + Sync + 'static,
    ) -> Self {
        self.computed_columns.push((name.into(), Box::new(compute)));
        self
    }

    /// Sort by a column.
    pub fn sort_by(mut self, key: SortKey) -> Self {
        self.sort_keys.push(key);
        self
    }

    /// Coerce a column to a target type.
    pub fn coerce(mut self, column: usize, target: crate::types::DataType) -> Self {
        self.coercions.push((column, target));
        self
    }

    /// Apply a PII masking strategy to a target column.
    pub fn mask(mut self, column: usize, strategy: crate::transform::mask::MaskingStrategy) -> Self {
        self.masks.push((column, strategy));
        self
    }

    /// Apply all pipeline stages to a batch, returning the transformed batch.
    ///
    /// Returns `None` if all rows were filtered out.
    pub fn apply(&self, mut batch: RowBatch) -> Option<RowBatch> {
        // Stage 1: Apply filters
        if !self.filters.is_empty() || !self.custom_filters.is_empty() {
            batch.rows.retain(|row| {
                // All column filters must match (AND logic)
                let col_filters_pass = self.filters.iter().all(|f| f.matches(row));
                // All custom filters must match
                let custom_filters_pass = self.custom_filters.iter().all(|f| f(row));
                col_filters_pass && custom_filters_pass
            });
        }

        if batch.is_empty() {
            return None;
        }

        // Stage 2: Type coercions
        for (col, target) in &self.coercions {
            map::coerce_column(&mut batch, *col, target);
        }

        // Stage 2.5: PII Masking
        for (col, strategy) in &self.masks {
            crate::transform::mask::mask_column(&mut batch, *col, strategy);
        }

        // Stage 3: Computed columns (before selection, so they can reference all columns)
        for (name, compute) in &self.computed_columns {
            map::add_computed_column(&mut batch, name, compute.as_ref());
        }

        // Stage 4: Column selection
        if let Some(ref columns) = self.select_columns {
            map::select_columns(&mut batch, columns);
        }

        // Stage 5: Column renames
        if !self.renames.is_empty() {
            let renames: Vec<(&str, &str)> = self
                .renames
                .iter()
                .map(|(old, new)| (old.as_str(), new.as_str()))
                .collect();
            map::rename_columns(&mut batch, &renames);
        }

        // Stage 6: Sorting (within batch)
        if !self.sort_keys.is_empty() {
            sort_batch(&mut batch, &self.sort_keys);
        }

        Some(batch)
    }

    /// Check if this pipeline has any stages configured.
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
            && self.custom_filters.is_empty()
            && self.select_columns.is_none()
            && self.renames.is_empty()
            && self.computed_columns.is_empty()
            && self.sort_keys.is_empty()
            && self.coercions.is_empty()
            && self.masks.is_empty()
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::filter::CompareOp;

    fn make_test_batch() -> RowBatch {
        let mut batch = RowBatch::new(0);
        batch.headers = Some(vec!["name".into(), "age".into(), "score".into()]);

        let data = vec![
            ("Alice", 30_i64, 85.0_f64),
            ("Bob", 25, 92.0),
            ("Charlie", 35, 78.0),
            ("Diana", 28, 95.0),
        ];

        for (i, (name, age, score)) in data.into_iter().enumerate() {
            let mut row = Row::new(i as u64);
            row.push(CellValue::from(name));
            row.push(CellValue::from(age));
            row.push(CellValue::from(score));
            batch.push(row);
        }

        batch
    }

    #[test]
    fn test_empty_pipeline() {
        let pipeline = Pipeline::new();
        let batch = make_test_batch();
        let result = pipeline.apply(batch).unwrap();
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn test_filter_pipeline() {
        let pipeline = Pipeline::new()
            .filter(ColumnFilter::new(1, CompareOp::Gte, CellValue::from(30_i64)));

        let batch = make_test_batch();
        let result = pipeline.apply(batch).unwrap();
        assert_eq!(result.len(), 2); // Alice (30) and Charlie (35)
    }

    #[test]
    fn test_filter_all_removed() {
        let pipeline = Pipeline::new()
            .filter(ColumnFilter::new(1, CompareOp::Gt, CellValue::from(100_i64)));

        let batch = make_test_batch();
        assert!(pipeline.apply(batch).is_none());
    }

    #[test]
    fn test_pipeline_with_computed_column() {
        let pipeline = Pipeline::new().add_column("passed", |row| {
            let score = row.get_float(2).unwrap_or(0.0);
            CellValue::Bool(score >= 80.0)
        });

        let batch = make_test_batch();
        let result = pipeline.apply(batch).unwrap();
        assert_eq!(result.rows[0].len(), 4); // Original 3 + computed
        assert_eq!(result.rows[0].get(3).unwrap().as_bool(), Some(true)); // 85 >= 80
        assert_eq!(result.rows[2].get(3).unwrap().as_bool(), Some(false)); // 78 < 80
    }

    #[test]
    fn test_combined_pipeline() {
        use crate::transform::sort::SortOrder;

        let pipeline = Pipeline::new()
            .filter(ColumnFilter::new(2, CompareOp::Gte, CellValue::from(80.0_f64)))
            .sort_by(SortKey {
                column: 2,
                order: SortOrder::Desc,
                nulls_first: false,
            });

        let batch = make_test_batch();
        let result = pipeline.apply(batch).unwrap();

        assert_eq!(result.len(), 3); // Charlie (78) filtered out
        // Sorted descending by score: Diana (95), Bob (92), Alice (85)
        assert_eq!(result.rows[0].get_str(0), Some("Diana"));
        assert_eq!(result.rows[1].get_str(0), Some("Bob"));
        assert_eq!(result.rows[2].get_str(0), Some("Alice"));
    }

    #[test]
    fn test_pipeline_masking() {
        use crate::transform::mask::MaskingStrategy;

        let pipeline = Pipeline::new()
            .mask(0, MaskingStrategy::Redact);

        let batch = make_test_batch();
        let result = pipeline.apply(batch).unwrap();

        // The first column ("name") should be redacted: "Alice" -> "*****"
        assert_eq!(result.rows[0].get_str(0), Some("*****"));
        assert_eq!(result.rows[1].get_str(0), Some("***"));
    }
}
