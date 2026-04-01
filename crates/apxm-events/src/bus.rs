//! Broadcast-based event bus for APXM events.

use std::fmt;

use tokio::sync::broadcast;

use crate::event::ApxmEvent;

/// A broadcast event bus that fans out events to all subscribers.
///
/// Uses `tokio::sync::broadcast` under the hood. Subscribers that fall
/// behind will receive a [`EventBusError::Lagged`] indicating how many
/// events were missed.
pub struct EventBus {
    tx: broadcast::Sender<ApxmEvent>,
}

impl EventBus {
    /// Create a new bus with the default capacity (1024).
    pub fn new() -> Self {
        Self::with_capacity(1024)
    }

    /// Create a new bus with a specific channel capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Publish an event to all current subscribers.
    ///
    /// Returns `Ok(())` on success. Returns `Err(event)` if there are no
    /// active subscribers (the event is returned so the caller can decide
    /// what to do with it).
    pub fn publish(&self, event: ApxmEvent) -> Result<(), ApxmEvent> {
        self.tx.send(event).map(|_| ()).map_err(|e| e.0)
    }

    /// Create a new subscriber that receives all future events.
    pub fn subscribe(&self) -> EventSubscriber {
        EventSubscriber {
            rx: self.tx.subscribe(),
        }
    }

    /// How many active subscribers exist.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// A subscriber that receives events from an [`EventBus`].
pub struct EventSubscriber {
    rx: broadcast::Receiver<ApxmEvent>,
}

impl EventSubscriber {
    /// Wait for the next event.
    ///
    /// Returns `Err(Lagged(n))` if this subscriber fell behind by `n` events.
    /// Returns `Err(Closed)` if the bus has been dropped.
    pub async fn recv(&mut self) -> Result<ApxmEvent, EventBusError> {
        match self.rx.recv().await {
            Ok(event) => Ok(event),
            Err(broadcast::error::RecvError::Lagged(n)) => Err(EventBusError::Lagged(n)),
            Err(broadcast::error::RecvError::Closed) => Err(EventBusError::Closed),
        }
    }
}

/// Errors that can occur when receiving events from a subscriber.
#[derive(Debug, Clone)]
pub enum EventBusError {
    /// The subscriber fell behind and missed `n` events.
    Lagged(u64),
    /// The event bus has been dropped.
    Closed,
}

impl fmt::Display for EventBusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventBusError::Lagged(n) => write!(f, "subscriber lagged behind by {n} events"),
            EventBusError::Closed => write!(f, "event bus closed"),
        }
    }
}

impl std::error::Error for EventBusError {}
