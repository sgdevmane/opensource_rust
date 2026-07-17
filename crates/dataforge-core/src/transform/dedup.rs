// =============================================================================
// DataForge Core — Deduplication Bloom Filter Transformation Stage
// =============================================================================
// Implements a space-efficient probabilistic deduplication filter on row streams.
// =============================================================================

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::types::{CellValue, RowBatch};

/// Space-efficient probabilistic Bloom Filter.
pub struct BloomFilter {
    bitset: Vec<bool>,
    num_hashes: usize,
}

impl BloomFilter {
    /// Create a new BloomFilter with a target capacity and false-positive rate.
    pub fn new(capacity: usize, false_positive_rate: f64) -> Self {
        let n = capacity as f64;
        let p = false_positive_rate;
        let ln2 = std::f64::consts::LN_2;
        let m = (-(n * p.ln()) / (ln2 * ln2)).ceil() as usize;
        let k = (((m as f64) / n) * ln2).round() as usize;
        let k = std::cmp::max(1, k);

        BloomFilter {
            bitset: vec![false; m],
            num_hashes: k,
        }
    }

    fn hashes(&self, item: &str) -> Vec<usize> {
        let mut result = Vec::with_capacity(self.num_hashes);
        for i in 0..self.num_hashes {
            let mut hasher = DefaultHasher::new();
            item.hash(&mut hasher);
            i.hash(&mut hasher); // salt the hash
            let hash_val = hasher.finish() as usize;
            result.push(hash_val % self.bitset.len());
        }
        result
    }

    /// Insert a value into the filter. Returns true if it was NOT present.
    pub fn insert(&mut self, item: &str) -> bool {
        let idxs = self.hashes(item);
        let mut is_new = false;
        for idx in idxs {
            if !self.bitset[idx] {
                self.bitset[idx] = true;
                is_new = true;
            }
        }
        is_new
    }

    /// Check if a value is likely present in the filter.
    pub fn contains(&self, item: &str) -> bool {
        let idxs = self.hashes(item);
        for idx in idxs {
            if !self.bitset[idx] {
                return false;
            }
        }
        true
    }
}

/// Deduplicator filter stage to retain only unique rows on a specific key column.
pub struct Deduplicator {
    filter: BloomFilter,
    col_idx: usize,
}

impl Deduplicator {
    /// Create a new Deduplicator on the target column index.
    pub fn new(col_idx: usize, capacity: usize) -> Self {
        Deduplicator {
            filter: BloomFilter::new(capacity, 0.01),
            col_idx,
        }
    }

    /// Filter duplicates out of the RowBatch.
    pub fn filter_batch(&mut self, batch: &mut RowBatch) {
        batch.rows.retain(|row| {
            if let Some(cell) = row.get(self.col_idx) {
                let cell_str = match cell {
                    CellValue::String(s) => s.to_string(),
                    CellValue::Int(v) => v.to_string(),
                    CellValue::Float(v) => v.to_string(),
                    _ => return true, // keep nulls or other types
                };
                self.filter.insert(&cell_str)
            } else {
                true
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Row;

    #[test]
    fn test_bloom_filter_insert_contains() {
        let mut bf = BloomFilter::new(1000, 0.01);
        assert!(bf.insert("apple"));
        assert!(bf.contains("apple"));
        assert!(!bf.contains("banana"));
        assert!(!bf.insert("apple")); // already seen
    }

    #[test]
    fn test_deduplicator() {
        let mut batch = RowBatch::new(0);
        batch.headers = Some(vec!["id".to_string()]);
        
        let mut r1 = Row::new(0);
        r1.push(CellValue::from("ID1"));
        batch.push(r1);

        let mut r2 = Row::new(1);
        r2.push(CellValue::from("ID2"));
        batch.push(r2);

        let mut r3 = Row::new(2);
        r3.push(CellValue::from("ID1")); // duplicate
        batch.push(r3);

        let mut dedup = Deduplicator::new(0, 100);
        dedup.filter_batch(&mut batch);

        assert_eq!(batch.len(), 2);
        assert_eq!(batch.rows[0].get_str(0), Some("ID1"));
        assert_eq!(batch.rows[1].get_str(0), Some("ID2"));
    }
}
