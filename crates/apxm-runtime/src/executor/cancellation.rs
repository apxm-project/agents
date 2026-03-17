//! Hierarchical cancellation tokens for propagating cancellation and timeouts.
//!
//! A `CancellationToken` forms a tree: cancelling a parent automatically
//! cancels all descendants.  Tokens are cheap to clone (interior `Arc`) and
//! safe to share across tasks.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

/// Shared inner state of a cancellation token.
struct Inner {
    cancelled: AtomicBool,
    children: Mutex<Vec<Arc<Inner>>>,
    parent: Option<Arc<Inner>>,
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
            parent: Some(Arc::clone(&self.inner)),
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
    let children = inner.children.lock();
    for child in children.iter() {
        cancel_recursive(child);
    }
}

fn is_cancelled_recursive(inner: &Arc<Inner>) -> bool {
    if inner.cancelled.load(Ordering::SeqCst) {
        return true;
    }
    if let Some(parent) = &inner.parent {
        return is_cancelled_recursive(parent);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_token_is_not_cancelled() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_cancel_sets_flag() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_parent_cancel_cascades_to_child() {
        let parent = CancellationToken::new();
        let child = parent.child();
        let grandchild = child.child();

        assert!(!grandchild.is_cancelled());
        parent.cancel();
        assert!(child.is_cancelled());
        assert!(grandchild.is_cancelled());
    }

    #[test]
    fn test_child_cancel_does_not_affect_parent() {
        let parent = CancellationToken::new();
        let child = parent.child();

        child.cancel();
        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
    }

    #[test]
    fn test_sibling_cancel_does_not_affect_sibling() {
        let parent = CancellationToken::new();
        let child_a = parent.child();
        let child_b = parent.child();

        child_a.cancel();
        assert!(child_a.is_cancelled());
        assert!(!child_b.is_cancelled());
        assert!(!parent.is_cancelled());
    }

    #[test]
    fn test_double_cancel_is_idempotent() {
        let token = CancellationToken::new();
        token.cancel();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_clone_shares_state() {
        let token = CancellationToken::new();
        let cloned = token.clone();
        token.cancel();
        assert!(cloned.is_cancelled());
    }

    #[tokio::test]
    async fn test_cancel_after_timeout() {
        let token = CancellationToken::new();
        token.cancel_after(Duration::from_millis(50));
        assert!(!token.is_cancelled());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_deep_hierarchy() {
        let root = CancellationToken::new();
        let mut current = root.clone();
        for _ in 0..100 {
            current = current.child();
        }
        assert!(!current.is_cancelled());
        root.cancel();
        assert!(current.is_cancelled());
    }
}
