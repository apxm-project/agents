//! Concurrency control and backpressure management.
//!
//! This module provides throttling to prevent overwhelming the system with
//! too many concurrent operations. Uses a semaphore to limit in-flight work.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use apxm_core::error::RuntimeError;
type RuntimeResult<T> = std::result::Result<T, RuntimeError>;

/// Concurrency controller for limiting in-flight operations.
///
/// Uses a semaphore to provide backpressure when too many operations
/// are executing simultaneously. This prevents resource exhaustion.
pub struct ConcurrencyControl {
    /// Semaphore controlling max in-flight operations.
    semaphore: Arc<Semaphore>,

    /// Maximum number of concurrent operations allowed.
    max_inflight: usize,

    /// Current number of in-flight operations (approximate).
    in_flight: Arc<AtomicUsize>,

    /// Cancellation signal.
    ///
    /// When true, acquire() will fail immediately.
    cancelled: Arc<AtomicBool>,
}

impl ConcurrencyControl {
    /// Create a new concurrency controller.
    ///
    /// # Arguments
    /// * `max_inflight` - Maximum number of operations that can execute concurrently
    pub fn new(max_inflight: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_inflight)),
            max_inflight,
            in_flight: Arc::new(AtomicUsize::new(0)),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Acquire a permit to execute an operation.
    ///
    /// Blocks until a permit is available or cancellation is triggered.
    ///
    /// Returns an RAII guard that releases the permit when dropped.
    pub async fn acquire(&self) -> RuntimeResult<ConcurrencyPermit> {
        // Check cancellation first
        if self.is_cancelled() {
            return Err(RuntimeError::SchedulerCancelled);
        }

        // Acquire an owned semaphore permit so the permit isn't bound to a lifetime.
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .map_err(|_| RuntimeError::SchedulerCancelled)?;

        // Increment in-flight counter
        self.in_flight.fetch_add(1, Ordering::Relaxed);

        Ok(ConcurrencyPermit {
            _permit: permit,
            in_flight: Arc::clone(&self.in_flight),
        })
    }

    /// Try to acquire a permit without blocking.
    ///
    /// Returns None if no permits are available or execution is cancelled.
    pub fn try_acquire(&self) -> Option<ConcurrencyPermit> {
        if self.is_cancelled() {
            return None;
        }

        // Use the owned try-acquire to avoid lifetime issues
        let permit = self.semaphore.clone().try_acquire_owned().ok()?;
        self.in_flight.fetch_add(1, Ordering::Relaxed);

        Some(ConcurrencyPermit {
            _permit: permit,
            in_flight: Arc::clone(&self.in_flight),
        })
    }

    /// Acquire a permit with a timeout.
    ///
    /// Returns an error if the timeout expires or execution is cancelled.
    pub async fn acquire_timeout(&self, timeout: Duration) -> RuntimeResult<ConcurrencyPermit> {
        if self.is_cancelled() {
            return Err(RuntimeError::SchedulerCancelled);
        }

        // Use acquire_owned in the timeout so the resulting permit is owned
        let permit = tokio::time::timeout(timeout, Arc::clone(&self.semaphore).acquire_owned())
            .await
            .map_err(|_| RuntimeError::Timeout {
                op_id: 0, // Generic timeout
                timeout,
            })?
            .map_err(|_| RuntimeError::SchedulerCancelled)?;

        self.in_flight.fetch_add(1, Ordering::Relaxed);

        Ok(ConcurrencyPermit {
            _permit: permit,
            in_flight: Arc::clone(&self.in_flight),
        })
    }

    /// Cancel all pending and future acquire attempts.
    ///
    /// Causes acquire() to return immediately with SchedulerCancelled error.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.semaphore.close();
    }

    /// Check if execution has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Get the number of available permits.
    ///
    /// Note: This is approximate due to concurrent modifications.
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Get the current number of in-flight operations.
    ///
    /// Note: This is approximate due to concurrent modifications.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
    }

    /// Get the maximum allowed in-flight operations.
    pub fn max_inflight(&self) -> usize {
        self.max_inflight
    }

    /// Check if the system is at capacity.
    pub fn is_at_capacity(&self) -> bool {
        self.available_permits() == 0
    }

    /// Get a cloneable handle to this controller.
    pub fn handle(&self) -> ConcurrencyControlHandle {
        ConcurrencyControlHandle {
            semaphore: Arc::clone(&self.semaphore),
            in_flight: Arc::clone(&self.in_flight),
            cancelled: Arc::clone(&self.cancelled),
        }
    }
}

/// RAII guard for a concurrency permit.
///
/// Automatically releases the permit when dropped.
#[derive(Debug)]
pub struct ConcurrencyPermit {
    _permit: OwnedSemaphorePermit,
    in_flight: Arc<AtomicUsize>,
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        // Decrement in-flight counter when permit is released
        self.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Cloneable handle to a ConcurrencyControl.
///
/// Allows multiple threads to interact with the same controller.
#[derive(Clone)]
pub struct ConcurrencyControlHandle {
    semaphore: Arc<Semaphore>,
    in_flight: Arc<AtomicUsize>,
    cancelled: Arc<AtomicBool>,
}

impl ConcurrencyControlHandle {
    /// Acquire a permit (see ConcurrencyControl::acquire).
    pub async fn acquire(&self) -> RuntimeResult<ConcurrencyPermit> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(RuntimeError::SchedulerCancelled);
        }

        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .map_err(|_| RuntimeError::SchedulerCancelled)?;

        self.in_flight.fetch_add(1, Ordering::Relaxed);

        Ok(ConcurrencyPermit {
            _permit: permit,
            in_flight: Arc::clone(&self.in_flight),
        })
    }

    /// Try to acquire without blocking.
    pub fn try_acquire(&self) -> Option<ConcurrencyPermit> {
        if self.cancelled.load(Ordering::Relaxed) {
            return None;
        }

        let permit = self.semaphore.clone().try_acquire_owned().ok()?;
        self.in_flight.fetch_add(1, Ordering::Relaxed);

        Some(ConcurrencyPermit {
            _permit: permit,
            in_flight: Arc::clone(&self.in_flight),
        })
    }

    /// Check if cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Get available permits count.
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Get in-flight count.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
    }
}
