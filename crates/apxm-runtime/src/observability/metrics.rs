//! Zero-overhead metrics collection for runtime observability.
//!
//! Compiles to nothing when the `metrics` feature is disabled.

#[cfg(feature = "metrics")]
mod enabled {
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;

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

        #[inline]
        pub fn record_execution_time(&self, duration: Duration) {
            self.total_execution_time_us
                .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
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
}

#[cfg(not(feature = "metrics"))]
mod disabled {
    use std::time::Duration;

    #[derive(Debug, Default, Clone, Copy)]
    pub struct MetricsCollector;

    impl MetricsCollector {
        #[inline(always)]
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
}

#[cfg(feature = "metrics")]
pub use enabled::{MetricsCollector, OverheadBreakdown};

#[cfg(not(feature = "metrics"))]
pub use disabled::{MetricsCollector, OverheadBreakdown};

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

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "per_op_overhead_us": self.per_op_overhead_us,
            "overhead_breakdown": {
                "ready_set_update_us": self.overhead_breakdown.ready_set_update_us,
                "work_stealing_us": self.overhead_breakdown.work_stealing_us,
                "input_collection_us": self.overhead_breakdown.input_collection_us,
                "operation_dispatch_us": self.overhead_breakdown.operation_dispatch_us,
                "token_routing_us": self.overhead_breakdown.token_routing_us
            },
            "max_parallelism": self.max_parallelism,
            "avg_parallelism": self.avg_parallelism,
            "operations_executed": self.operations_executed,
            "operations_failed": self.operations_failed
        })
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
