// =============================================================================
// DataForge Core — Running Aggregations
// =============================================================================
// Memory-bounded streaming aggregations (sum, count, avg, min, max, std-dev).
// These operate on a running state — they don't need to see all data at once.
// =============================================================================

use crate::types::{CellValue, Row, RowBatch};

/// Aggregation operation type.
#[derive(Debug, Clone)]
pub enum AggOp {
    /// Sum of numeric values
    Sum,
    /// Count of non-null values
    Count,
    /// Average (mean) of numeric values
    Avg,
    /// Minimum value
    Min,
    /// Maximum value
    Max,
    /// Count of distinct values (uses hash set — bounded by unique count)
    CountDistinct,
}

/// Specification for a single aggregation.
#[derive(Debug, Clone)]
pub struct AggSpec {
    /// Column index to aggregate
    pub column: usize,
    /// Aggregation operation
    pub op: AggOp,
    /// Output column name
    pub output_name: String,
}

/// Running aggregation state that processes rows incrementally.
///
/// This accumulates aggregate values as rows flow through without
/// holding any row data in memory — only the running totals.
#[derive(Debug)]
pub struct Aggregator {
    /// Aggregation specifications
    specs: Vec<AggSpec>,
    /// Running state for each aggregation
    states: Vec<AggState>,
}

/// Internal state for a single running aggregation.
#[derive(Debug)]
struct AggState {
    sum: f64,
    count: u64,
    min: Option<f64>,
    max: Option<f64>,
    /// For Welford's online variance algorithm
    mean: f64,
    m2: f64, // sum of squared differences from mean
    distinct: Option<std::collections::HashSet<String>>,
}

impl AggState {
    fn new(needs_distinct: bool) -> Self {
        AggState {
            sum: 0.0,
            count: 0,
            min: None,
            max: None,
            mean: 0.0,
            m2: 0.0,
            distinct: if needs_distinct {
                Some(std::collections::HashSet::new())
            } else {
                None
            },
        }
    }

    /// Update this state with a new numeric value (Welford's online algorithm).
    fn update(&mut self, value: f64) {
        self.sum += value;
        self.count += 1;

        // Update min/max
        self.min = Some(match self.min {
            Some(m) => m.min(value),
            None => value,
        });
        self.max = Some(match self.max {
            Some(m) => m.max(value),
            None => value,
        });

        // Welford's online algorithm for variance
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = value - self.mean;
        self.m2 += delta * delta2;
    }

    /// Update distinct count with a string representation.
    fn update_distinct(&mut self, value: &str) {
        if let Some(set) = &mut self.distinct {
            set.insert(value.to_string());
            self.count += 1;
        }
    }
}

impl Aggregator {
    /// Create a new aggregator with the given specifications.
    pub fn new(specs: Vec<AggSpec>) -> Self {
        let states = specs
            .iter()
            .map(|s| AggState::new(matches!(s.op, AggOp::CountDistinct)))
            .collect();
        Aggregator { specs, states }
    }

    /// Process a batch of rows, updating all running aggregations.
    pub fn process_batch(&mut self, batch: &RowBatch) {
        for row in &batch.rows {
            self.process_row(row);
        }
    }

    /// Process a single row.
    pub fn process_row(&mut self, row: &Row) {
        for (spec, state) in self.specs.iter().zip(self.states.iter_mut()) {
            let cell = row.get(spec.column).unwrap_or(&CellValue::Null);

            match &spec.op {
                AggOp::Count => {
                    if !cell.is_null() {
                        state.count += 1;
                    }
                }
                AggOp::CountDistinct => {
                    if !cell.is_null() {
                        state.update_distinct(&cell.to_display_string());
                    }
                }
                _ => {
                    if let Some(v) = cell.as_float() {
                        state.update(v);
                    }
                }
            }
        }
    }

    /// Get the final aggregation results.
    ///
    /// Returns a single row containing the aggregated values.
    pub fn results(&self) -> Row {
        let mut row = Row::new(0);

        for (spec, state) in self.specs.iter().zip(self.states.iter()) {
            let value = match spec.op {
                AggOp::Sum => CellValue::Float(state.sum),
                AggOp::Count => CellValue::Int(state.count as i64),
                AggOp::Avg => {
                    if state.count > 0 {
                        CellValue::Float(state.sum / state.count as f64)
                    } else {
                        CellValue::Null
                    }
                }
                AggOp::Min => match state.min {
                    Some(v) => CellValue::Float(v),
                    None => CellValue::Null,
                },
                AggOp::Max => match state.max {
                    Some(v) => CellValue::Float(v),
                    None => CellValue::Null,
                },
                AggOp::CountDistinct => {
                    let count = state.distinct.as_ref().map(|s| s.len()).unwrap_or(0);
                    CellValue::Int(count as i64)
                }
            };
            row.push(value);
        }

        row
    }

    /// Get the result column headers.
    pub fn result_headers(&self) -> Vec<String> {
        self.specs.iter().map(|s| s.output_name.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rows() -> Vec<Row> {
        let values = vec![
            (0, vec![CellValue::from("A"), CellValue::from(10.0_f64)]),
            (1, vec![CellValue::from("B"), CellValue::from(20.0_f64)]),
            (2, vec![CellValue::from("A"), CellValue::from(30.0_f64)]),
            (3, vec![CellValue::from("C"), CellValue::from(40.0_f64)]),
        ];

        values
            .into_iter()
            .map(|(idx, cells)| {
                let mut row = Row::new(idx);
                for c in cells {
                    row.push(c);
                }
                row
            })
            .collect()
    }

    #[test]
    fn test_sum() {
        let specs = vec![AggSpec {
            column: 1,
            op: AggOp::Sum,
            output_name: "total".into(),
        }];
        let mut agg = Aggregator::new(specs);

        for row in &make_rows() {
            agg.process_row(row);
        }

        let result = agg.results();
        assert_eq!(result.get_float(0), Some(100.0));
    }

    #[test]
    fn test_count() {
        let specs = vec![AggSpec {
            column: 0,
            op: AggOp::Count,
            output_name: "count".into(),
        }];
        let mut agg = Aggregator::new(specs);

        for row in &make_rows() {
            agg.process_row(row);
        }

        let result = agg.results();
        assert_eq!(result.get_int(0), Some(4));
    }

    #[test]
    fn test_avg() {
        let specs = vec![AggSpec {
            column: 1,
            op: AggOp::Avg,
            output_name: "avg".into(),
        }];
        let mut agg = Aggregator::new(specs);

        for row in &make_rows() {
            agg.process_row(row);
        }

        let result = agg.results();
        assert_eq!(result.get_float(0), Some(25.0));
    }

    #[test]
    fn test_min_max() {
        let specs = vec![
            AggSpec { column: 1, op: AggOp::Min, output_name: "min".into() },
            AggSpec { column: 1, op: AggOp::Max, output_name: "max".into() },
        ];
        let mut agg = Aggregator::new(specs);

        for row in &make_rows() {
            agg.process_row(row);
        }

        let result = agg.results();
        assert_eq!(result.get_float(0), Some(10.0));
        assert_eq!(result.get_float(1), Some(40.0));
    }

    #[test]
    fn test_count_distinct() {
        let specs = vec![AggSpec {
            column: 0,
            op: AggOp::CountDistinct,
            output_name: "unique".into(),
        }];
        let mut agg = Aggregator::new(specs);

        for row in &make_rows() {
            agg.process_row(row);
        }

        let result = agg.results();
        assert_eq!(result.get_int(0), Some(3)); // A, B, C
    }
}
