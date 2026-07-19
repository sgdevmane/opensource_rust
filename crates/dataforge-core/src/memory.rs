// =============================================================================
// DataForge Core — Memory Management & Backpressure
// =============================================================================
// Memory tracking and backpressure system to prevent OOM crashes.
//
// The key insight: When processing massive files (100k+ rows), unchecked
// buffering will eventually exhaust available memory. This module provides:
//
// 1. **MemoryTracker**: Thread-safe atomic counter of current memory usage
// 2. **MemoryGuard**: RAII guard that releases memory when dropped
// 3. **Backpressure**: Blocks/errors/drops when memory limit is reached
//
// This ensures DataForge maintains constant memory usage regardless of
// input file size — the defining characteristic of this library.
// =============================================================================

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{debug, warn};

use crate::config::BackpressurePolicy;
use crate::error::{DataForgeError, Result};

/// Thread-safe memory usage tracker with backpressure support.
///
/// This is the central memory management component. It tracks the total
/// bytes currently allocated by DataForge operations and enforces a
/// configurable ceiling.
///
/// # How it works
/// 1. Before allocating a batch, call `try_allocate(bytes)` or `allocate(bytes)`
/// 2. If current + requested > limit, the backpressure policy is applied:
///    - `Block`: Spins (with backoff) until memory is freed by another thread
///    - `Error`: Returns `MemoryLimitExceeded` immediately
///    - `DropOldest`: Signals that the oldest batch should be dropped
/// 3. When a batch is consumed/freed, call `release(bytes)` or drop the `MemoryGuard`
///
/// # Thread Safety
/// Uses `AtomicUsize` for lock-free memory tracking. Multiple threads can
/// allocate and release concurrently without contention.
///
/// # Example
/// ```
/// use dataforge_core::memory::MemoryTracker;
/// use dataforge_core::config::BackpressurePolicy;
///
/// let tracker = MemoryTracker::new(1024 * 1024, BackpressurePolicy::Error); // 1MB limit
///
/// // Allocate 512KB — succeeds
/// let guard = tracker.try_allocate(512 * 1024).unwrap();
///
/// // Try to allocate another 768KB — fails (would exceed 1MB limit)
/// assert!(tracker.try_allocate(768 * 1024).is_err());
///
/// // Release the first allocation
/// drop(guard);
///
/// // Now 768KB fits
/// let guard2 = tracker.try_allocate(768 * 1024).unwrap();
/// ```
#[derive(Debug)]
pub struct MemoryTracker {
    /// Current memory usage in bytes (atomic for lock-free access)
    current_bytes: AtomicUsize,

    /// Maximum allowed memory in bytes
    limit_bytes: usize,

    /// What to do when the limit is exceeded
    policy: BackpressurePolicy,

    /// Peak memory usage observed (for diagnostics)
    peak_bytes: AtomicUsize,

    /// Total bytes allocated over the lifetime (for diagnostics)
    total_allocated: AtomicUsize,

    /// Total bytes released over the lifetime (for diagnostics)
    total_released: AtomicUsize,

    /// Whether the tracker has been shut down (signals producers to stop)
    shutdown: AtomicBool,
}

impl MemoryTracker {
    /// Create a new memory tracker with the given limit and policy.
    ///
    /// # Arguments
    /// * `limit_bytes` - Maximum allowed memory usage in bytes
    /// * `policy` - What to do when the limit is exceeded
    pub fn new(limit_bytes: usize, policy: BackpressurePolicy) -> Arc<Self> {
        Arc::new(MemoryTracker {
            current_bytes: AtomicUsize::new(0),
            limit_bytes,
            policy,
            peak_bytes: AtomicUsize::new(0),
            total_allocated: AtomicUsize::new(0),
            total_released: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
        })
    }

    /// Generate an ASCII progress bar and stats showing current and peak memory usage.
    pub fn generate_telemetry_ascii_chart(&self) -> String {
        let current = self.current_bytes.load(Ordering::Relaxed);
        let peak = self.peak_bytes.load(Ordering::Relaxed);
        let limit = self.limit_bytes;

        let pct = if limit > 0 {
            (current as f64 / limit as f64 * 100.0).min(100.0)
        } else {
            0.0
        };

        let width = 30;
        let filled = ((pct / 100.0) * width as f64) as usize;
        let empty = width - filled;

        let bar = format!(
            "[{}{}] {:.1}%",
            "#".repeat(filled),
            ".".repeat(empty),
            pct
        );

        format!(
            "--- Memory Telemetry ---\n\
             Usage Limit:  {} bytes\n\
             Current Usg:  {} bytes {}\n\
             Peak Usage:   {} bytes\n\
             Allocated:    {} bytes\n\
             Released:     {} bytes\n\
             ------------------------",
            limit,
            current,
            bar,
            peak,
            self.total_allocated.load(Ordering::Relaxed),
            self.total_released.load(Ordering::Relaxed)
        )
    }

    /// Try to allocate `bytes` of memory, applying backpressure if needed.
    ///
    /// Returns a `MemoryGuard` that automatically releases the memory
    /// when dropped. This RAII pattern ensures memory is always freed,
    /// even if the consumer panics.
    ///
    /// # Behavior by Policy
    /// - `Block`: Waits up to 30 seconds for memory to become available
    /// - `Error`: Returns error immediately if limit would be exceeded
    /// - `DropOldest`: Returns error (caller must handle dropping)
    pub fn try_allocate(self: &Arc<Self>, bytes: usize) -> Result<MemoryGuard> {
        // Check shutdown flag first
        if self.shutdown.load(Ordering::Relaxed) {
            return Err(DataForgeError::internal("Memory tracker has been shut down"));
        }

        match self.policy {
            BackpressurePolicy::Block => self.allocate_with_blocking(bytes),
            BackpressurePolicy::Error => self.allocate_or_error(bytes),
            BackpressurePolicy::DropOldest => self.allocate_or_error(bytes),
        }
    }

    /// Allocate memory, blocking until space is available.
    ///
    /// Uses exponential backoff to avoid busy-waiting:
    /// - Starts with 1μs sleeps
    /// - Doubles up to 10ms max sleep
    /// - Times out after 30 seconds
    fn allocate_with_blocking(self: &Arc<Self>, bytes: usize) -> Result<MemoryGuard> {
        let start = Instant::now();
        let timeout = Duration::from_secs(30);
        let mut backoff = Duration::from_micros(1);
        let max_backoff = Duration::from_millis(10);

        loop {
            // Try a lock-free compare-and-swap allocation
            let current = self.current_bytes.load(Ordering::Relaxed);
            let new_total = current + bytes;

            if new_total <= self.limit_bytes {
                // Attempt to atomically claim the memory
                if self
                    .current_bytes
                    .compare_exchange_weak(current, new_total, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
                {
                    self.record_allocation(bytes, new_total);
                    return Ok(MemoryGuard {
                        tracker: Arc::clone(self),
                        bytes,
                    });
                }
                // CAS failed — another thread beat us, retry immediately
                continue;
            }

            // Memory limit would be exceeded — apply backoff
            if start.elapsed() > timeout {
                warn!(
                    current_bytes = current,
                    limit_bytes = self.limit_bytes,
                    requested_bytes = bytes,
                    "Memory allocation timed out after 30s"
                );
                return Err(DataForgeError::MemoryLimitExceeded {
                    current_bytes: current,
                    limit_bytes: self.limit_bytes,
                });
            }

            // Check shutdown during wait
            if self.shutdown.load(Ordering::Relaxed) {
                return Err(DataForgeError::internal("Memory tracker shut down during wait"));
            }

            std::thread::sleep(backoff);
            backoff = (backoff * 2).min(max_backoff);
        }
    }

    /// Try to allocate memory, returning an error immediately if the limit
    /// would be exceeded (no waiting).
    fn allocate_or_error(self: &Arc<Self>, bytes: usize) -> Result<MemoryGuard> {
        loop {
            let current = self.current_bytes.load(Ordering::Relaxed);
            let new_total = current + bytes;

            if new_total > self.limit_bytes {
                return Err(DataForgeError::MemoryLimitExceeded {
                    current_bytes: current,
                    limit_bytes: self.limit_bytes,
                });
            }

            if self
                .current_bytes
                .compare_exchange_weak(current, new_total, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                self.record_allocation(bytes, new_total);
                return Ok(MemoryGuard {
                    tracker: Arc::clone(self),
                    bytes,
                });
            }
            // CAS failed — retry
        }
    }

    /// Record an allocation in diagnostic counters.
    fn record_allocation(&self, bytes: usize, new_total: usize) {
        self.total_allocated.fetch_add(bytes, Ordering::Relaxed);

        // Update peak using a lock-free max pattern
        let mut peak = self.peak_bytes.load(Ordering::Relaxed);
        while new_total > peak {
            match self.peak_bytes.compare_exchange_weak(
                peak,
                new_total,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => peak = actual,
            }
        }

        debug!(
            allocated_bytes = bytes,
            current_total = new_total,
            limit = self.limit_bytes,
            "Memory allocated"
        );
    }

    /// Release `bytes` of memory back to the tracker.
    ///
    /// Called automatically by `MemoryGuard::drop()`, but can also be
    /// called manually for fine-grained control.
    pub fn release(&self, bytes: usize) {
        let prev = self.current_bytes.fetch_sub(bytes, Ordering::AcqRel);
        self.total_released.fetch_add(bytes, Ordering::Relaxed);
        debug!(
            released_bytes = bytes,
            current_total = prev - bytes,
            "Memory released"
        );
    }

    /// Get current memory usage in bytes.
    pub fn current_usage(&self) -> usize {
        self.current_bytes.load(Ordering::Relaxed)
    }

    /// Get the configured memory limit in bytes.
    pub fn limit(&self) -> usize {
        self.limit_bytes
    }

    /// Get the peak memory usage observed.
    pub fn peak_usage(&self) -> usize {
        self.peak_bytes.load(Ordering::Relaxed)
    }

    /// Get the memory utilization as a percentage (0.0 to 100.0).
    pub fn utilization_percent(&self) -> f64 {
        let current = self.current_usage() as f64;
        let limit = self.limit_bytes as f64;
        (current / limit) * 100.0
    }

    /// Signal all producers to stop. Waiting allocations will fail.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    /// Check if the tracker has been shut down.
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    /// Get a diagnostic snapshot of memory usage.
    pub fn stats(&self) -> MemoryStats {
        MemoryStats {
            current_bytes: self.current_bytes.load(Ordering::Relaxed),
            limit_bytes: self.limit_bytes,
            peak_bytes: self.peak_bytes.load(Ordering::Relaxed),
            total_allocated: self.total_allocated.load(Ordering::Relaxed),
            total_released: self.total_released.load(Ordering::Relaxed),
        }
    }
}

/// RAII guard that releases memory when dropped.
///
/// This ensures memory is always freed, even if the consumer panics
/// or an error causes early return. The guard tracks how many bytes
/// it is responsible for and releases them on drop.
#[derive(Debug)]
pub struct MemoryGuard {
    /// Reference to the tracker that issued this guard
    tracker: Arc<MemoryTracker>,
    /// Number of bytes this guard is holding
    bytes: usize,
}

impl MemoryGuard {
    /// Get the number of bytes this guard is holding.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for MemoryGuard {
    fn drop(&mut self) {
        if self.bytes > 0 {
            self.tracker.release(self.bytes);
        }
    }
}

/// Diagnostic snapshot of memory tracker state.
///
/// Useful for logging, monitoring, and debugging memory usage patterns.
#[derive(Debug, Clone)]
pub struct MemoryStats {
    /// Current memory in use (bytes)
    pub current_bytes: usize,
    /// Configured limit (bytes)
    pub limit_bytes: usize,
    /// Peak memory observed (bytes)
    pub peak_bytes: usize,
    /// Total memory allocated over lifetime (bytes)
    pub total_allocated: usize,
    /// Total memory released over lifetime (bytes)
    pub total_released: usize,
}

impl std::fmt::Display for MemoryStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Memory: {:.1}MB / {:.1}MB ({:.1}%), peak: {:.1}MB, total alloc: {:.1}MB",
            self.current_bytes as f64 / 1_048_576.0,
            self.limit_bytes as f64 / 1_048_576.0,
            (self.current_bytes as f64 / self.limit_bytes as f64) * 100.0,
            self.peak_bytes as f64 / 1_048_576.0,
            self.total_allocated as f64 / 1_048_576.0,
        )
    }
}

impl MemoryStats {
    /// Export metrics in Prometheus text exposition format.
    ///
    /// This produces a multi-line string compatible with Prometheus scrapers
    /// and Grafana dashboards. Each metric includes HELP and TYPE annotations.
    ///
    /// # Example output
    /// ```text
    /// # HELP dataforge_memory_current_bytes Current memory usage in bytes.
    /// # TYPE dataforge_memory_current_bytes gauge
    /// dataforge_memory_current_bytes 524288
    /// ```
    pub fn to_prometheus_metrics(&self) -> String {
        format!(
            "# HELP dataforge_memory_current_bytes Current memory usage in bytes.\n\
             # TYPE dataforge_memory_current_bytes gauge\n\
             dataforge_memory_current_bytes {current}\n\
             # HELP dataforge_memory_limit_bytes Configured memory limit in bytes.\n\
             # TYPE dataforge_memory_limit_bytes gauge\n\
             dataforge_memory_limit_bytes {limit}\n\
             # HELP dataforge_memory_peak_bytes Peak memory usage observed in bytes.\n\
             # TYPE dataforge_memory_peak_bytes gauge\n\
             dataforge_memory_peak_bytes {peak}\n\
             # HELP dataforge_memory_total_allocated_bytes Total bytes allocated over the lifetime.\n\
             # TYPE dataforge_memory_total_allocated_bytes counter\n\
             dataforge_memory_total_allocated_bytes {total_alloc}\n\
             # HELP dataforge_memory_total_released_bytes Total bytes released over the lifetime.\n\
             # TYPE dataforge_memory_total_released_bytes counter\n\
             dataforge_memory_total_released_bytes {total_release}\n\
             # HELP dataforge_memory_utilization_ratio Memory utilization ratio (0.0 to 1.0).\n\
             # TYPE dataforge_memory_utilization_ratio gauge\n\
             dataforge_memory_utilization_ratio {utilization:.6}\n",
            current = self.current_bytes,
            limit = self.limit_bytes,
            peak = self.peak_bytes,
            total_alloc = self.total_allocated,
            total_release = self.total_released,
            utilization = if self.limit_bytes > 0 {
                self.current_bytes as f64 / self.limit_bytes as f64
            } else {
                0.0
            },
        )
    }
}

impl MemoryTracker {
    /// Export the current memory stats in Prometheus text exposition format.
    ///
    /// This is a convenience method that calls `stats().to_prometheus_metrics()`.
    /// Use this to expose metrics via an HTTP endpoint.
    pub fn to_prometheus(&self) -> String {
        self.stats().to_prometheus_metrics()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_allocation_and_release() {
        let tracker = MemoryTracker::new(1024, BackpressurePolicy::Error);

        // Allocate 512 bytes
        let guard = tracker.try_allocate(512).unwrap();
        assert_eq!(tracker.current_usage(), 512);

        // Release via drop
        drop(guard);
        assert_eq!(tracker.current_usage(), 0);
    }

    #[test]
    fn test_memory_limit_exceeded() {
        let tracker = MemoryTracker::new(1024, BackpressurePolicy::Error);

        // Allocate 512 bytes — OK
        let _guard = tracker.try_allocate(512).unwrap();

        // Try to allocate 768 more — exceeds 1024 limit
        let result = tracker.try_allocate(768);
        assert!(result.is_err());

        // Current usage should still be 512 (failed allocation doesn't change it)
        assert_eq!(tracker.current_usage(), 512);
    }

    #[test]
    fn test_raii_guard_on_drop() {
        let tracker = MemoryTracker::new(1024, BackpressurePolicy::Error);

        {
            let _g1 = tracker.try_allocate(256).unwrap();
            let _g2 = tracker.try_allocate(256).unwrap();
            assert_eq!(tracker.current_usage(), 512);
        }
        // Both guards dropped — memory should be fully released
        assert_eq!(tracker.current_usage(), 0);
    }

    #[test]
    fn test_peak_tracking() {
        let tracker = MemoryTracker::new(4096, BackpressurePolicy::Error);

        let g1 = tracker.try_allocate(1000).unwrap();
        let g2 = tracker.try_allocate(2000).unwrap();
        assert_eq!(tracker.peak_usage(), 3000);

        drop(g1);
        drop(g2);
        assert_eq!(tracker.current_usage(), 0);
        // Peak should still be 3000
        assert_eq!(tracker.peak_usage(), 3000);
    }

    #[test]
    fn test_stats_display() {
        let tracker = MemoryTracker::new(1024 * 1024, BackpressurePolicy::Error);
        let _guard = tracker.try_allocate(512 * 1024).unwrap();
        let stats = tracker.stats();
        let display = stats.to_string();
        assert!(display.contains("Memory:"));
        assert!(display.contains("0.5MB"));
    }

    #[test]
    fn test_shutdown() {
        let tracker = MemoryTracker::new(1024, BackpressurePolicy::Error);
        assert!(!tracker.is_shutdown());

        tracker.shutdown();
        assert!(tracker.is_shutdown());

        // Allocations should fail after shutdown
        let result = tracker.try_allocate(100);
        assert!(result.is_err());
    }

    #[test]
    fn test_blocking_policy_releases() {
        // Test that blocking policy works when memory is released by another thread
        let tracker = MemoryTracker::new(1024, BackpressurePolicy::Block);

        let guard = tracker.try_allocate(800).unwrap();
        let tracker_clone = Arc::clone(&tracker);

        // Spawn a thread that releases memory after a short delay
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            drop(guard);
        });

        // This should block briefly, then succeed after the other thread releases
        let result = tracker_clone.try_allocate(800);
        assert!(result.is_ok());

        handle.join().unwrap();
    }

    #[test]
    fn test_utilization_percent() {
        let tracker = MemoryTracker::new(1000, BackpressurePolicy::Error);
        let _g = tracker.try_allocate(500).unwrap();
        let pct = tracker.utilization_percent();
        assert!((pct - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_telemetry_ascii_chart() {
        let tracker = MemoryTracker::new(1000, BackpressurePolicy::Error);
        let _g = tracker.try_allocate(500).unwrap();
        let chart = tracker.generate_telemetry_ascii_chart();
        assert!(chart.contains("Memory Telemetry"));
        assert!(chart.contains("50.0%"));
    }
}
