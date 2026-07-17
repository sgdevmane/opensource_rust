// =============================================================================
// DataForge Core — Thread Pool Management
// =============================================================================
// Manages the Rayon thread pool configuration for parallel operations.
// =============================================================================

use rayon::ThreadPoolBuilder;
use tracing::info;

use crate::error::{DataForgeError, Result};

/// Initialize a custom Rayon thread pool with the given number of threads.
///
/// If not called, Rayon uses its default global pool (num_cpus threads).
/// This function allows limiting parallelism to avoid resource contention.
pub fn init_thread_pool(num_threads: usize) -> Result<rayon::ThreadPool> {
    let pool = ThreadPoolBuilder::new()
        .num_threads(num_threads.max(1))
        .thread_name(|idx| format!("dataforge-worker-{idx}"))
        .build()
        .map_err(|e| DataForgeError::internal(format!("Failed to create thread pool: {e}")))?;

    info!(num_threads = pool.current_num_threads(), "Thread pool initialized");

    Ok(pool)
}

/// Get the number of available CPU cores.
pub fn available_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_thread_pool() {
        let pool = init_thread_pool(2).unwrap();
        assert_eq!(pool.current_num_threads(), 2);
    }

    #[test]
    fn test_available_parallelism() {
        let cores = available_parallelism();
        assert!(cores >= 1);
    }
}
