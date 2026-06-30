//! Standard event sink implementations.

use std::sync::{Arc, mpsc};

use super::{ApxmEvent, EventEmitter};

/// Discards all events.
pub struct NoOpEmitter;

impl EventEmitter for NoOpEmitter {
    fn emit(&self, _event: ApxmEvent) {}
}

/// Sends events over a standard library MPSC channel.
pub struct ChannelEmitter {
    tx: mpsc::Sender<ApxmEvent>,
}

impl ChannelEmitter {
    pub fn new(tx: mpsc::Sender<ApxmEvent>) -> Self {
        Self { tx }
    }
}

impl EventEmitter for ChannelEmitter {
    fn emit(&self, event: ApxmEvent) {
        let _ = self.tx.send(event);
    }
}

/// Broadcasts events to multiple downstream emitters.
pub struct FanOutEmitter {
    emitters: Vec<Arc<dyn EventEmitter>>,
}

impl FanOutEmitter {
    pub fn new(emitters: Vec<Arc<dyn EventEmitter>>) -> Self {
        Self { emitters }
    }
}

impl EventEmitter for FanOutEmitter {
    fn emit(&self, event: ApxmEvent) {
        for emitter in &self.emitters {
            emitter.emit(event.clone());
        }
    }
}
