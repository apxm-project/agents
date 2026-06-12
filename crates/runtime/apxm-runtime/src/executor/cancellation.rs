//! Hierarchical cancellation tokens for propagating cancellation and timeouts.
//!
//! A `CancellationToken` forms a tree: cancelling a parent automatically
//! cancels all descendants.  Tokens are cheap to clone (interior `Arc`) and
//! safe to share across tasks.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;

/// Shared inner state of a cancellation token.
struct Inner {
    cancelled: AtomicBool,
    children: Mutex<Vec<Arc<Inner>>>,
    parent: Option<Weak<Inner>>,
    notify: Notify,
}

/// A hierarchical cancellation token.
///
/// Tokens form a tree. Calling [`cancel`](CancellationToken::cancel) sets
/// the token's flag and cascades to every descendant.  Calling
/// [`is_cancelled`](CancellationToken::is_cancelled) walks up to the root
/// so that a child is considered cancelled if any ancestor has been
/// cancelled.
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<Inner>,
}

impl CancellationToken {
    /// Create a new root cancellation token.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                cancelled: AtomicBool::new(false),
                children: Mutex::new(Vec::new()),
                parent: None,
                notify: Notify::new(),
            }),
        }
    }

    /// Create a child token whose lifetime is bounded by this token.
    ///
    /// Cancelling the parent will cascade to the child, but cancelling
    /// the child does **not** affect the parent.
    pub fn child(&self) -> Self {
        let child_inner = Arc::new(Inner {
            cancelled: AtomicBool::new(false),
            children: Mutex::new(Vec::new()),
            parent: Some(Arc::downgrade(&self.inner)),
            notify: Notify::new(),
        });
        self.inner.children.lock().push(Arc::clone(&child_inner));
        Self { inner: child_inner }
    }

    /// Cancel this token and all descendants.
    pub fn cancel(&self) {
        cancel_recursive(&self.inner);
    }

    /// Returns `true` if this token or any ancestor has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        is_cancelled_recursive(&self.inner)
    }

    /// Wait until this token or any ancestor is cancelled.
    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            let notified = self.inner.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }

    /// Cancel this token after `timeout` elapses (non-blocking).
    ///
    /// Spawns a background Tokio task.  If the token is already cancelled
    /// the timer is a no-op.
    pub fn cancel_after(&self, timeout: Duration) {
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            cancel_recursive(&inner);
        });
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

// ── helpers ─────────────────────────────────────────────────────────

fn cancel_recursive(inner: &Arc<Inner>) {
    if inner.cancelled.swap(true, Ordering::SeqCst) {
        return; // already cancelled
    }

    inner.notify.notify_waiters();
    let children = inner.children.lock().clone();

    for child in children {
        cancel_recursive(&child);
    }
}

fn is_cancelled_recursive(inner: &Arc<Inner>) -> bool {
    if inner.cancelled.load(Ordering::SeqCst) {
        return true;
    }
    if let Some(parent) = &inner.parent
        && let Some(parent) = parent.upgrade()
    {
        return is_cancelled_recursive(&parent);
    }
    false
}

