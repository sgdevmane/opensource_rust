// =============================================================================
// DataForge Core — Fuzzy Joins Transformation Stage
// =============================================================================
// Joins two streaming RowBatch collections using fuzzy string comparison.
// =============================================================================

use crate::types::{CellValue, Row, RowBatch};

/// Fuzzy Similarity Metric to use for key matches.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum FuzzyJoinMetric {
    /// Levenshtein edit distance metric.
    Levenshtein,
}

/// Fuzzy Join engine.
pub struct FuzzyJoiner {
    right_rows: Vec<Row>,
    right_headers: Option<Vec<String>>,
    left_key_idx: usize,
    right_key_idx: usize,
    metric: FuzzyJoinMetric,
    threshold: f64,
}

impl FuzzyJoiner {
    /// Create a new FuzzyJoiner using right reference RowBatch.
    pub fn new(
        right_batch: RowBatch,
        left_key_idx: usize,
        right_key_idx: usize,
        metric: FuzzyJoinMetric,
        threshold: f64,
    ) -> Self {
        FuzzyJoiner {
            right_rows: right_batch.rows,
            right_headers: right_batch.headers,
            left_key_idx,
            right_key_idx,
            metric,
            threshold,
        }
    }

    /// Perform a fuzzy left-join on a left RowBatch.
    pub fn join_batch(&self, left_batch: &RowBatch) -> RowBatch {
        let mut joined_batch = RowBatch::new(left_batch.start_index);
        
        // Build joined headers
        if let (Some(ref lh), Some(ref rh)) = (&left_batch.headers, &self.right_headers) {
            let mut headers = lh.clone();
            for (idx, header) in rh.iter().enumerate() {
                if idx != self.right_key_idx {
                    headers.push(header.clone());
                }
            }
            joined_batch.headers = Some(headers);
        }

        for left_row in &left_batch.rows {
            let mut matched = false;
            if let Some(left_key) = left_row.get_str(self.left_key_idx) {
                let mut best_match: Option<(&Row, f64)> = None;

                for right_row in &self.right_rows {
                    if let Some(right_key) = right_row.get_str(self.right_key_idx) {
                        let similarity = match self.metric {
                            FuzzyJoinMetric::Levenshtein => levenshtein_similarity(left_key, right_key),
                        };
                        if similarity >= self.threshold {
                            if let Some((_, best_sim)) = best_match {
                                if similarity > best_sim {
                                    best_match = Some((right_row, similarity));
                                }
                            } else {
                                best_match = Some((right_row, similarity));
                            }
                        }
                    }
                }

                if let Some((right_row, _)) = best_match {
                    let mut joined_row = left_row.clone();
                    for (idx, cell) in right_row.cells.iter().enumerate() {
                        if idx != self.right_key_idx {
                            joined_row.push(cell.clone());
                        }
                    }
                    joined_batch.push(joined_row);
                    matched = true;
                }
            }

            if !matched {
                let mut joined_row = left_row.clone();
                if let Some(ref rh) = self.right_headers {
                    for idx in 0..rh.len() {
                        if idx != self.right_key_idx {
                            joined_row.push(CellValue::Null);
                        }
                    }
                }
                joined_batch.push(joined_row);
            }
        }

        joined_batch
    }
}

fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let len1 = s1.chars().count();
    let len2 = s2.chars().count();
    if len1 == 0 { return len2; }
    if len2 == 0 { return len1; }
    
    let mut dp = vec![vec![0; len2 + 1]; len1 + 1];
    for i in 0..=len1 { dp[i][0] = i; }
    for j in 0..=len2 { dp[0][j] = j; }
    
    for (i, c1) in s1.chars().enumerate() {
        for (j, c2) in s2.chars().enumerate() {
            let cost = if c1 == c2 { 0 } else { 1 };
            dp[i+1][j+1] = std::cmp::min(
                dp[i][j+1] + 1,
                std::cmp::min(dp[i+1][j] + 1, dp[i][j] + cost)
            );
        }
    }
    dp[len1][len2]
}

/// Compute Levenshtein-based similarity percentage (0.0 to 1.0).
pub fn levenshtein_similarity(s1: &str, s2: &str) -> f64 {
    let max_len = std::cmp::max(s1.chars().count(), s2.chars().count());
    if max_len == 0 { return 1.0; }
    let dist = levenshtein_distance(s1, s2);
    1.0 - (dist as f64 / max_len as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_similarity() {
        assert_eq!(levenshtein_similarity("Alice", "Alice"), 1.0);
        assert!((levenshtein_similarity("Alice", "Alic") - 0.8).abs() < 1e-9);
        assert!((levenshtein_similarity("Alice", "Bob") - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_fuzzy_join() {
        let mut right_batch = RowBatch::new(0);
        right_batch.headers = Some(vec!["name_ref".to_string(), "department".to_string()]);
        let mut r1 = Row::new(0);
        r1.push(CellValue::from("Engineering Dept"));
        r1.push(CellValue::from("R&D"));
        right_batch.push(r1);

        let mut left_batch = RowBatch::new(0);
        left_batch.headers = Some(vec!["employee".to_string(), "dept_label".to_string()]);
        let mut l1 = Row::new(0);
        l1.push(CellValue::from("Bob"));
        l1.push(CellValue::from("Engineering")); // fuzzy match to "Engineering Dept"
        left_batch.push(l1);

        let joiner = FuzzyJoiner::new(right_batch, 1, 0, FuzzyJoinMetric::Levenshtein, 0.6);
        let joined = joiner.join_batch(&left_batch);

        assert_eq!(joined.len(), 1);
        assert_eq!(joined.rows[0].get_str(2), Some("R&D"));
    }
}
