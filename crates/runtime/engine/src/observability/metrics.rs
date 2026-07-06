//! Zero-overhead metrics collection for runtime observability.
//!
//! Compiles to nothing when the `metrics` feature is disabled.

#[cfg(feature = "metrics")]
mod enabled {
    use parking_lot::Mutex;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;

    // Flush the per-worker accumulators back to the shared collector every
    // this many recorded ops. Bounds staleness while keeping the shared
    // cache line cold during normal operation.
    pub const WORKER_METRICS_FLUSH_OPS: u32 = 64;

    #[derive(Debug)]
    pub struct MetricsCollector {
        pub ready_set_update_ns: AtomicU64,
        pub work_stealing_ns: AtomicU64,
        pub input_collection_ns: AtomicU64,
        pub operation_dispatch_ns: AtomicU64,
        pub token_routing_ns: AtomicU64,

        pub max_concurrent_ops: AtomicUsize,
        pub operations_in_flight: AtomicUsize,
        pub parallelism_samples: Mutex<Vec<usize>>,

        pub operations_executed: AtomicUsize,
        pub operations_failed: AtomicUsize,
        pub tokens_published: AtomicUsize,
        pub retries_attempted: AtomicUsize,
        pub total_execution_time_us: AtomicU64,

        pub ready_set_update_count: AtomicU64,
        pub work_stealing_count: AtomicU64,
        pub input_collection_count: AtomicU64,
        pub operation_dispatch_count: AtomicU64,
        pub token_routing_count: AtomicU64,
    }

    impl Default for MetricsCollector {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MetricsCollector {
        pub fn new() -> Self {
            Self {
                ready_set_update_ns: AtomicU64::new(0),
                work_stealing_ns: AtomicU64::new(0),
                input_collection_ns: AtomicU64::new(0),
                operation_dispatch_ns: AtomicU64::new(0),
                token_routing_ns: AtomicU64::new(0),
                max_concurrent_ops: AtomicUsize::new(0),
                operations_in_flight: AtomicUsize::new(0),
                parallelism_samples: Mutex::new(Vec::with_capacity(1024)),
                operations_executed: AtomicUsize::new(0),
                operations_failed: AtomicUsize::new(0),
                tokens_published: AtomicUsize::new(0),
                retries_attempted: AtomicUsize::new(0),
                total_execution_time_us: AtomicU64::new(0),
                ready_set_update_count: AtomicU64::new(0),
                work_stealing_count: AtomicU64::new(0),
                input_collection_count: AtomicU64::new(0),
                operation_dispatch_count: AtomicU64::new(0),
                token_routing_count: AtomicU64::new(0),
            }
        }

        #[inline]
        pub fn record_ready_set_update(&self, duration: Duration) {
            self.ready_set_update_ns
                .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
            self.ready_set_update_count.fetch_add(1, Ordering::Relaxed);
        }

        #[inline]
        pub fn record_work_stealing(&self, duration: Duration) {
            self.work_stealing_ns
                .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
            self.work_stealing_count.fetch_add(1, Ordering::Relaxed);
        }

        #[inline]
        pub fn record_input_collection(&self, duration: Duration) {
            self.input_collection_ns
                .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
            self.input_collection_count.fetch_add(1, Ordering::Relaxed);
        }

        #[inline]
        pub fn record_operation_dispatch(&self, duration: Duration) {
            self.operation_dispatch_ns
                .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
            self.operation_dispatch_count
                .fetch_add(1, Ordering::Relaxed);
        }

        #[inline]
        pub fn record_token_routing(&self, duration: Duration) {
            self.token_routing_ns
                .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
            self.token_routing_count.fetch_add(1, Ordering::Relaxed);
        }

        #[inline]
        pub fn record_schedule(&self) {
            let current = self.operations_in_flight.fetch_add(1, Ordering::Relaxed) + 1;

            let mut max = self.max_concurrent_ops.load(Ordering::Relaxed);
            while current > max {
                match self.max_concurrent_ops.compare_exchange_weak(
                    max,
                    current,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(actual) => max = actual,
                }
            }

            if let Some(mut samples) = self.parallelism_samples.try_lock() {
                samples.push(current);
            }
        }

        #[inline]
        pub fn record_completion(&self) {
            self.operations_executed.fetch_add(1, Ordering::Relaxed);
            self.operations_in_flight.fetch_sub(1, Ordering::Relaxed);
        }

        #[inline]
        pub fn record_failure(&self) {
            self.operations_failed.fetch_add(1, Ordering::Relaxed);
            self.operations_in_flight.fetch_sub(1, Ordering::Relaxed);
        }

        /// Decrement in-flight only — paired with a deferred local count so
        /// `max_concurrent_ops` stays live while executed/failed totals batch.
        #[inline]
        pub fn release_in_flight(&self) {
            self.operations_in_flight.fetch_sub(1, Ordering::Relaxed);
        }

        #[inline]
        pub fn record_execution_time(&self, duration: Duration) {
            self.total_execution_time_us
                .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
        }

        /// Apply a worker's accumulated additive counters in one batch — five
        /// `fetch_add`s per counter family instead of one per op, eliminating
        /// the per-op CAS traffic on the shared cache line.
        pub fn merge_from(&self, local: &WorkerLocalMetrics) {
            if local.is_empty() {
                return;
            }
            self.ready_set_update_ns
                .fetch_add(local.ready_set_update_ns, Ordering::Relaxed);
            self.work_stealing_ns
                .fetch_add(local.work_stealing_ns, Ordering::Relaxed);
            self.input_collection_ns
                .fetch_add(local.input_collection_ns, Ordering::Relaxed);
            self.operation_dispatch_ns
                .fetch_add(local.operation_dispatch_ns, Ordering::Relaxed);
            self.token_routing_ns
                .fetch_add(local.token_routing_ns, Ordering::Relaxed);
            self.total_execution_time_us
                .fetch_add(local.total_execution_time_us, Ordering::Relaxed);
            self.ready_set_update_count
                .fetch_add(local.ready_set_update_count, Ordering::Relaxed);
            self.work_stealing_count
                .fetch_add(local.work_stealing_count, Ordering::Relaxed);
            self.input_collection_count
                .fetch_add(local.input_collection_count, Ordering::Relaxed);
            self.operation_dispatch_count
                .fetch_add(local.operation_dispatch_count, Ordering::Relaxed);
            self.token_routing_count
                .fetch_add(local.token_routing_count, Ordering::Relaxed);
            self.operations_executed
                .fetch_add(local.operations_executed, Ordering::Relaxed);
            self.operations_failed
                .fetch_add(local.operations_failed, Ordering::Relaxed);
        }

        pub fn get_executed(&self) -> usize {
            self.operations_executed.load(Ordering::Relaxed)
        }

        pub fn get_failed(&self) -> usize {
            self.operations_failed.load(Ordering::Relaxed)
        }

        pub fn get_in_flight(&self) -> usize {
            self.operations_in_flight.load(Ordering::Relaxed)
        }

        pub fn overhead_breakdown_us(&self) -> OverheadBreakdown {
            let ops = self.operations_executed.load(Ordering::Relaxed).max(1) as f64;

            OverheadBreakdown {
                ready_set_update_us: self.ready_set_update_ns.load(Ordering::Relaxed) as f64
                    / 1000.0
                    / ops,
                work_stealing_us: self.work_stealing_ns.load(Ordering::Relaxed) as f64
                    / 1000.0
                    / ops,
                input_collection_us: self.input_collection_ns.load(Ordering::Relaxed) as f64
                    / 1000.0
                    / ops,
                operation_dispatch_us: self.operation_dispatch_ns.load(Ordering::Relaxed) as f64
                    / 1000.0
                    / ops,
                token_routing_us: self.token_routing_ns.load(Ordering::Relaxed) as f64
                    / 1000.0
                    / ops,
            }
        }

        pub fn total_overhead_per_op_us(&self) -> f64 {
            let b = self.overhead_breakdown_us();
            b.ready_set_update_us
                + b.work_stealing_us
                + b.input_collection_us
                + b.operation_dispatch_us
                + b.token_routing_us
        }

        pub fn average_parallelism(&self) -> f64 {
            let samples = self.parallelism_samples.lock();
            if samples.is_empty() {
                1.0
            } else {
                samples.iter().sum::<usize>() as f64 / samples.len() as f64
            }
        }

        pub fn max_parallelism(&self) -> usize {
            self.max_concurrent_ops.load(Ordering::Relaxed)
        }
    }

    #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
    pub struct OverheadBreakdown {
        pub ready_set_update_us: f64,
        pub work_stealing_us: f64,
        pub input_collection_us: f64,
        pub operation_dispatch_us: f64,
        pub token_routing_us: f64,
    }

    /// Per-worker, single-threaded accumulator for the additive counters in
    /// `MetricsCollector`. Flushed into the shared collector on
    /// a coarse cadence (every `WORKER_METRICS_FLUSH_OPS` ops) and on Drop,
    /// so per-op recording stays in private cache lines.
    #[derive(Debug)]
    pub struct WorkerLocalMetrics {
        shared: Arc<MetricsCollector>,
        pending_ops: u32,

        ready_set_update_ns: u64,
        work_stealing_ns: u64,
        input_collection_ns: u64,
        operation_dispatch_ns: u64,
        token_routing_ns: u64,
        total_execution_time_us: u64,

        ready_set_update_count: u64,
        work_stealing_count: u64,
        input_collection_count: u64,
        operation_dispatch_count: u64,
        token_routing_count: u64,

        operations_executed: usize,
        operations_failed: usize,
    }

    impl WorkerLocalMetrics {
        pub fn new(shared: Arc<MetricsCollector>) -> Self {
            Self {
                shared,
                pending_ops: 0,
                ready_set_update_ns: 0,
                work_stealing_ns: 0,
                input_collection_ns: 0,
                operation_dispatch_ns: 0,
                token_routing_ns: 0,
                total_execution_time_us: 0,
                ready_set_update_count: 0,
                work_stealing_count: 0,
                input_collection_count: 0,
                operation_dispatch_count: 0,
                token_routing_count: 0,
                operations_executed: 0,
                operations_failed: 0,
            }
        }

        #[inline]
        fn is_empty(&self) -> bool {
            self.pending_ops == 0
        }

        #[inline]
        pub fn record_ready_set_update(&mut self, duration: Duration) {
            self.ready_set_update_ns += duration.as_nanos() as u64;
            self.ready_set_update_count += 1;
            self.pending_ops += 1;
        }

        #[inline]
        pub fn record_work_stealing(&mut self, duration: Duration) {
            self.work_stealing_ns += duration.as_nanos() as u64;
            self.work_stealing_count += 1;
            self.pending_ops += 1;
        }

        #[inline]
        pub fn record_input_collection(&mut self, duration: Duration) {
            self.input_collection_ns += duration.as_nanos() as u64;
            self.input_collection_count += 1;
            self.pending_ops += 1;
        }

        #[inline]
        pub fn record_operation_dispatch(&mut self, duration: Duration) {
            self.operation_dispatch_ns += duration.as_nanos() as u64;
            self.operation_dispatch_count += 1;
            self.pending_ops += 1;
        }

        #[inline]
        pub fn record_token_routing(&mut self, duration: Duration) {
            self.token_routing_ns += duration.as_nanos() as u64;
            self.token_routing_count += 1;
            self.pending_ops += 1;
        }

        #[inline]
        pub fn record_completion(&mut self) {
            self.operations_executed += 1;
            self.pending_ops += 1;
        }

        #[inline]
        pub fn record_failure(&mut self) {
            self.operations_failed += 1;
            self.pending_ops += 1;
        }

        #[inline]
        pub fn record_execution_time(&mut self, duration: Duration) {
            self.total_execution_time_us += duration.as_micros() as u64;
            self.pending_ops += 1;
        }

        /// Push accumulators to the shared collector and reset locals. Called
        /// automatically on Drop, but workers can invoke it explicitly to
        /// bound staleness during long-running loops.
        pub fn flush(&mut self) {
            let shared = Arc::clone(&self.shared);
            shared.merge_from(self);
            self.pending_ops = 0;
            self.ready_set_update_ns = 0;
            self.work_stealing_ns = 0;
            self.input_collection_ns = 0;
            self.operation_dispatch_ns = 0;
            self.token_routing_ns = 0;
            self.total_execution_time_us = 0;
            self.ready_set_update_count = 0;
            self.work_stealing_count = 0;
            self.input_collection_count = 0;
            self.operation_dispatch_count = 0;
            self.token_routing_count = 0;
            self.operations_executed = 0;
            self.operations_failed = 0;
        }

        /// Flush when pending op count crosses the configured threshold.
        #[inline]
        pub fn maybe_flush(&mut self) {
            if self.pending_ops >= WORKER_METRICS_FLUSH_OPS {
                self.flush();
            }
        }
    }

    impl Drop for WorkerLocalMetrics {
        fn drop(&mut self) {
            self.flush();
        }
    }
}

#[cfg(not(feature = "metrics"))]
mod disabled {
    use std::time::Duration;

    #[derive(Debug, Default, Clone, Copy)]
    pub struct MetricsCollector;

    #[allow(clippy::inline_always)]
    impl MetricsCollector {
        pub fn new() -> Self {
            Self
        }

        #[inline(always)]
        pub fn record_ready_set_update(&self, _: Duration) {}
        #[inline(always)]
        pub fn record_work_stealing(&self, _: Duration) {}
        #[inline(always)]
        pub fn record_input_collection(&self, _: Duration) {}
        #[inline(always)]
        pub fn record_operation_dispatch(&self, _: Duration) {}
        #[inline(always)]
        pub fn record_token_routing(&self, _: Duration) {}

        #[inline(always)]
        pub fn record_schedule(&self) {}
        #[inline(always)]
        pub fn record_completion(&self) {}
        #[inline(always)]
        pub fn record_failure(&self) {}
        #[inline(always)]
        pub fn release_in_flight(&self) {}

        #[inline(always)]
        pub fn record_execution_time(&self, _: Duration) {}

        #[inline(always)]
        pub fn get_executed(&self) -> usize {
            0
        }
        #[inline(always)]
        pub fn get_failed(&self) -> usize {
            0
        }
        #[inline(always)]
        pub fn get_in_flight(&self) -> usize {
            0
        }
    }

    #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
    pub struct OverheadBreakdown {
        pub ready_set_update_us: f64,
        pub work_stealing_us: f64,
        pub input_collection_us: f64,
        pub operation_dispatch_us: f64,
        pub token_routing_us: f64,
    }

    // Stays API-compatible with the enabled variant so worker code compiles
    // without `cfg` guards around the local-metrics handle.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct WorkerLocalMetrics;

    #[allow(clippy::inline_always)]
    impl WorkerLocalMetrics {
        #[inline(always)]
        pub fn new(_shared: std::sync::Arc<MetricsCollector>) -> Self {
            Self
        }
        #[inline(always)]
        pub fn record_ready_set_update(&mut self, _: Duration) {}
        #[inline(always)]
        pub fn record_work_stealing(&mut self, _: Duration) {}
        #[inline(always)]
        pub fn record_input_collection(&mut self, _: Duration) {}
        #[inline(always)]
        pub fn record_operation_dispatch(&mut self, _: Duration) {}
        #[inline(always)]
        pub fn record_token_routing(&mut self, _: Duration) {}
        #[inline(always)]
        pub fn record_completion(&mut self) {}
        #[inline(always)]
        pub fn record_failure(&mut self) {}
        #[inline(always)]
        pub fn record_execution_time(&mut self, _: Duration) {}
        #[inline(always)]
        pub fn flush(&mut self) {}
        #[inline(always)]
        pub fn maybe_flush(&mut self) {}
    }
}

#[cfg(feature = "metrics")]
pub use enabled::{MetricsCollector, OverheadBreakdown, WorkerLocalMetrics};

#[cfg(not(feature = "metrics"))]
pub use disabled::{MetricsCollector, OverheadBreakdown, WorkerLocalMetrics};

/// Snapshot of scheduler metrics for inclusion in execution results.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SchedulerMetrics {
    pub per_op_overhead_us: f64,
    pub overhead_breakdown: OverheadBreakdown,
    pub max_parallelism: usize,
    pub avg_parallelism: f64,
    pub operations_executed: usize,
    pub operations_failed: usize,
}

impl SchedulerMetrics {
    #[cfg(feature = "metrics")]
    pub fn from_collector(collector: &MetricsCollector) -> Self {
        Self {
            per_op_overhead_us: collector.total_overhead_per_op_us(),
            overhead_breakdown: collector.overhead_breakdown_us(),
            max_parallelism: collector.max_parallelism(),
            avg_parallelism: collector.average_parallelism(),
            operations_executed: collector.get_executed(),
            operations_failed: collector.get_failed(),
        }
    }

    #[cfg(not(feature = "metrics"))]
    pub fn from_collector(_collector: &MetricsCollector) -> Self {
        Self::default()
    }

    /// Wire-shape serialization. The struct's serde field names ARE the wire keys;
    /// `serde_json::to_value` is the single source of truth.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// Times a block only when the `metrics` feature is enabled.
#[macro_export]
macro_rules! timed {
    ($metrics:expr, $method:ident, $block:expr) => {{
        #[cfg(feature = "metrics")]
        let __start = std::time::Instant::now();

        let __result = $block;

        #[cfg(feature = "metrics")]
        $metrics.$method(__start.elapsed());

        __result
    }};
}

/// Times an async block only when the `metrics` feature is enabled.
#[macro_export]
macro_rules! timed_async {
    ($metrics:expr, $method:ident, $block:expr) => {{
        #[cfg(feature = "metrics")]
        let __start = std::time::Instant::now();

        let __result = $block.await;

        #[cfg(feature = "metrics")]
        $metrics.$method(__start.elapsed());

        __result
    }};
}
