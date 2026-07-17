// =============================================================================
// DataForge Core — File Chunking for Parallel Processing
// =============================================================================
// Re-exports the chunking logic from csv::reader for general use.
// The chunk splitting algorithm works for any newline-delimited format.
// =============================================================================

/// A range describing a chunk of a file.
#[derive(Debug, Clone, Copy)]
pub struct ChunkRange {
    /// Byte offset from the start of the file
    pub offset: usize,
    /// Number of bytes in this chunk
    pub length: usize,
}
