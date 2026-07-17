// =============================================================================
// DataForge Core — Transform Pipeline
// =============================================================================
// Composable data transformation pipeline for streaming row processing.
// All transformations are lazy and operate on batches, maintaining the
// streaming property — no full-file materialization needed.
// =============================================================================

pub mod aggregate;
pub mod filter;
pub mod map;
pub mod pipeline;
pub mod sort;
pub mod mask;
pub mod join;
pub mod dedup;
pub mod pivot;

pub use pipeline::Pipeline;
pub use sort::{external_sort, ExternalSortIterator, SortKey, SortOrder};
pub use mask::{MaskingStrategy, mask_column};
pub use join::{FuzzyJoiner, FuzzyJoinMetric, levenshtein_similarity};
pub use dedup::{BloomFilter, Deduplicator};
pub use pivot::{PivotTable, PivotAggregate};
