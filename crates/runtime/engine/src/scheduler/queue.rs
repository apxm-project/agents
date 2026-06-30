//! Priority queue for ready operations.
//!
//! This module provides a 4-level priority queue for scheduling operations.
//! Operations are enqueued by priority and dequeued FIFO within each priority level.

use std::sync::Arc;

use apxm_core::types::NodeId;
use crossbeam_deque::Injector;

/// Priority levels for operations.
///
/// Higher numbers = higher priority (executed first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Priority {
    /// Lowest priority (0-29).
    Low = 0,
    /// Normal priority (30-59).
    Normal = 1,
    /// High priority (60-89).
    High = 2,
    /// Critical priority (90-100).
    Critical = 3,
}

impl Priority {
    /// Convert a numeric priority (0-100) to a Priority level.
    pub fn from_u8(priority: u8) -> Self {
        match priority {
            x if x >= 90 => Priority::Critical,
            x if x >= 60 => Priority::High,
            x if x >= 30 => Priority::Normal,
            _ => Priority::Low,
        }
    }

    /// Get the priority as a usize index (for array indexing).
    #[inline]
    pub fn as_index(self) -> usize {
        self as usize
    }

    /// Stable lower-case label used in metrics and hook payloads.
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
            Priority::Critical => "critical",
        }
    }

    /// Number of priority levels.
    pub const COUNT: usize = 4;
}

/// Priority queue for ready operations.
///
/// Maintains 4 separate FIFO queues (one per priority level).
/// Operations are dequeued from the highest-priority non-empty queue.
pub struct PriorityQueue {
    /// Injectors for each priority level.
    ///
    /// Index 0 = Low, 1 = Normal, 2 = High, 3 = Critical
    injectors: [Arc<Injector<NodeId>>; Priority::COUNT],
}

impl PriorityQueue {
    /// Create a new priority queue.
    pub fn new() -> Self {
        Self {
            injectors: [
                Arc::new(Injector::new()),
                Arc::new(Injector::new()),
                Arc::new(Injector::new()),
                Arc::new(Injector::new()),
            ],
        }
    }

    /// Push an operation to the queue at the given priority.
    #[inline]
    pub fn push(&self, node_id: NodeId, priority: Priority) {
        self.injectors[priority.as_index()].push(node_id);
    }

    /// Get a reference to the injector at the given priority level.
    ///
    /// Useful for work-stealing.
    #[inline]
    pub fn injector(&self, priority: Priority) -> &Arc<Injector<NodeId>> {
        &self.injectors[priority.as_index()]
    }

    /// Get references to all injectors.
    ///
    /// Returned in order: [Low, Normal, High, Critical]
    pub fn injectors(&self) -> &[Arc<Injector<NodeId>>; Priority::COUNT] {
        &self.injectors
    }

    /// Check if all queues are empty.
    pub fn is_empty(&self) -> bool {
        self.injectors.iter().all(|inj| inj.is_empty())
    }

    /// Get approximate total length across all queues.
    ///
    /// Note: This is an estimate due to concurrent modifications.
    pub fn len(&self) -> usize {
        self.injectors.iter().map(|inj| inj.len()).sum()
    }
}

impl Default for PriorityQueue {
    fn default() -> Self {
        Self::new()
    }
}
