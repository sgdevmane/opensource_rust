// =============================================================================
// DataForge Core — Prometheus & Telemetry Metrics Engine
// =============================================================================
// Tracks metrics for parsed rows, processing latency, active streaming jobs,
// memory footprint, and error counts for Grafana/Prometheus monitoring.
// =============================================================================

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub struct CoreMetricsRegistry {
    pub total_rows_parsed: AtomicU64,
    pub total_bytes_processed: AtomicU64,
    pub total_transformations_executed: AtomicU64,
    pub active_jobs_count: AtomicU64,
    pub error_count: AtomicU64,
}

impl CoreMetricsRegistry {
    pub fn new() -> Self {
        Self {
            total_rows_parsed: AtomicU64::new(0),
            total_bytes_processed: AtomicU64::new(0),
            total_transformations_executed: AtomicU64::new(0),
            active_jobs_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
        }
    }

    pub fn record_parsed_rows(&self, count: u64) {
        self.total_rows_parsed.fetch_add(count, Ordering::Relaxed);
    }

    pub fn record_bytes(&self, bytes: u64) {
        self.total_bytes_processed.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_transformation(&self) {
        self.total_transformations_executed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_active_jobs(&self) {
        self.active_jobs_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn decrement_active_jobs(&self) {
        self.active_jobs_count.fetch_sub(1, Ordering::Relaxed);
    }

    /// Render Prometheus exposition text format string.
    pub fn render_prometheus(&self) -> String {
        let mut prometheus = String::with_capacity(512);

        prometheus.push_str("# HELP dataforge_rows_parsed_total Total number of spreadsheet/tabular rows parsed\n");
        prometheus.push_str("# TYPE dataforge_rows_parsed_total counter\n");
        prometheus.push_str(&format!("dataforge_rows_parsed_total {}\n\n", self.total_rows_parsed.load(Ordering::Relaxed)));

        prometheus.push_str("# HELP dataforge_bytes_processed_total Total bytes processed across parsers\n");
        prometheus.push_str("# TYPE dataforge_bytes_processed_total counter\n");
        prometheus.push_str(&format!("dataforge_bytes_processed_total {}\n\n", self.total_bytes_processed.load(Ordering::Relaxed)));

        prometheus.push_str("# HELP dataforge_transformations_total Total pipeline transformations executed\n");
        prometheus.push_str("# TYPE dataforge_transformations_total counter\n");
        prometheus.push_str(&format!("dataforge_transformations_total {}\n\n", self.total_transformations_executed.load(Ordering::Relaxed)));

        prometheus.push_str("# HELP dataforge_active_jobs Currently active processing jobs\n");
        prometheus.push_str("# TYPE dataforge_active_jobs gauge\n");
        prometheus.push_str(&format!("dataforge_active_jobs {}\n\n", self.active_jobs_count.load(Ordering::Relaxed)));

        prometheus.push_str("# HELP dataforge_errors_total Total errors encountered\n");
        prometheus.push_str("# TYPE dataforge_errors_total counter\n");
        prometheus.push_str(&format!("dataforge_errors_total {}\n", self.error_count.load(Ordering::Relaxed)));

        prometheus
    }
}

impl Default for CoreMetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ExecutionTimer<'a> {
    registry: &'a CoreMetricsRegistry,
    start: Instant,
}

impl<'a> ExecutionTimer<'a> {
    pub fn start(registry: &'a CoreMetricsRegistry) -> Self {
        registry.increment_active_jobs();
        Self {
            registry,
            start: Instant::now(),
        }
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

impl<'a> Drop for ExecutionTimer<'a> {
    fn drop(&mut self) {
        self.registry.decrement_active_jobs();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_metrics() {
        let registry = CoreMetricsRegistry::new();
        registry.record_parsed_rows(150);
        registry.record_bytes(2048);
        registry.record_transformation();

        let output = registry.render_prometheus();
        assert!(output.contains("dataforge_rows_parsed_total 150"));
        assert!(output.contains("dataforge_bytes_processed_total 2048"));
    }
}
