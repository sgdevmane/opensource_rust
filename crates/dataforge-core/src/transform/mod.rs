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
pub mod encrypt;
pub mod clean;
pub mod diff;

pub use pipeline::Pipeline;
pub use sort::{external_sort, ExternalSortIterator, SortKey, SortOrder};
pub use mask::{MaskingStrategy, mask_column};
pub use join::{FuzzyJoiner, FuzzyJoinMetric, levenshtein_similarity, disk_buffered_fuzzy_join};
pub use dedup::{BloomFilter, Deduplicator};
pub use pivot::{PivotTable, PivotAggregate};
pub use encrypt::{encrypt_columns, encrypt_aes_256_cbc};
pub use clean::{DataCleaner, CleanStrategy};
pub use diff::{WorkbookDiffEngine, DiffReport, DiffKind, CellDiff, RowDiff};
