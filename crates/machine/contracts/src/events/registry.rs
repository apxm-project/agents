//! Event payload registries for extension decoders.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use serde::de::DeserializeOwned;
use thiserror::Error;

use super::kind;
use super::payload::EventPayload;
use super::{EventKind, core_event_kind};

/// Decoder function for one registered event payload type.
pub type EventPayloadDecoder = fn(serde_json::Value) -> serde_json::Result<Box<dyn EventPayload>>;

/// One event kind plus its payload decoder.
#[derive(Clone, Copy)]
pub struct EventPayloadRegistration {
    /// Registered event kind.
    pub kind: EventKind,
    /// Payload decoder for this kind.
    pub decoder: EventPayloadDecoder,
}

/// Event payload registry failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EventRegistryError {
    /// Core event kinds are owned by `apxm-core` and cannot be overridden.
    #[error("event kind `{kind}` is a core event kind and cannot be registered as an extension")]
    CoreKind {
        /// Event wire name.
        kind: &'static str,
    },
    /// The event kind already has an extension decoder.
    #[error("event kind `{kind}` is already registered")]
    AlreadyRegistered {
        /// Event wire name.
        kind: &'static str,
    },
}

/// Registry of known payload decoders keyed by event wire name.
#[derive(Clone, Default)]
pub struct EventPayloadRegistry {
    registrations: HashMap<&'static str, EventPayloadRegistration>,
}

impl EventPayloadRegistry {
    /// Create an empty registry for extension events.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an event payload type for a kind.
    pub fn register_payload<T>(&mut self, kind: EventKind) -> Result<(), EventRegistryError>
    where
        T: EventPayload + DeserializeOwned,
    {
        self.register_decoder(kind, decode_payload::<T>)
    }

    /// Register a custom decoder for a kind.
    pub fn register_decoder(
        &mut self,
        kind: EventKind,
        decoder: EventPayloadDecoder,
    ) -> Result<(), EventRegistryError> {
        let kind_name = kind.name();
        if core_event_kind(kind_name).is_some() {
            return Err(EventRegistryError::CoreKind { kind: kind_name });
        }
        if self.registrations.contains_key(kind_name) {
            return Err(EventRegistryError::AlreadyRegistered { kind: kind_name });
        }

        self.registrations
            .insert(kind_name, EventPayloadRegistration { kind, decoder });
        Ok(())
    }

    /// Merge another extension registry into this one.
    pub fn extend(&mut self, other: EventPayloadRegistry) -> Result<(), EventRegistryError> {
        for registration in other.registrations.values().copied() {
            self.register_decoder(registration.kind, registration.decoder)?;
        }
        Ok(())
    }

    /// Return `true` when a kind name has a decoder in this registry.
    pub fn contains(&self, kind_name: &str) -> bool {
        self.registrations.contains_key(kind_name)
    }

    /// Decode a payload with a registered decoder.
    ///
    /// The serialized `kind` discriminator is stripped before invoking the
    /// typed decoder because concrete payload structs do not carry it.
    pub fn decode(
        &self,
        kind_name: &str,
        payload_json: serde_json::Value,
    ) -> Option<serde_json::Result<Box<dyn EventPayload>>> {
        let registration = self.registrations.get(kind_name)?;
        Some((registration.decoder)(payload_without_kind(payload_json)))
    }
}

/// Register an extension payload decoder in the process-wide registry.
pub fn register_event_payload<T>(kind: EventKind) -> Result<(), EventRegistryError>
where
    T: EventPayload + DeserializeOwned,
{
    register_event_payload_decoder(EventPayloadRegistration {
        kind,
        decoder: decode_payload::<T>,
    })
}

/// Register an extension payload decoder in the process-wide registry.
pub fn register_event_payload_decoder(
    registration: EventPayloadRegistration,
) -> Result<(), EventRegistryError> {
    let kind_name = registration.kind.name();
    if core_event_kind(kind_name).is_some() {
        return Err(EventRegistryError::CoreKind { kind: kind_name });
    }

    let registry = global_registry();
    let mut registrations = registry.write().expect("event registry lock");
    if registrations.contains_key(kind_name) {
        return Err(EventRegistryError::AlreadyRegistered { kind: kind_name });
    }
    registrations.insert(kind_name, registration);
    Ok(())
}

/// Look up a globally registered extension event kind.
pub fn registered_event_kind(kind_name: &str) -> Option<EventKind> {
    let registry = global_registry();
    registry
        .read()
        .expect("event registry lock")
        .get(kind_name)
        .map(|registration| registration.kind)
}

/// List globally registered extension event kinds.
pub fn registered_event_kinds() -> Vec<EventKind> {
    let registry = global_registry();
    let mut kinds: Vec<EventKind> = registry
        .read()
        .expect("event registry lock")
        .values()
        .map(|registration| registration.kind)
        .collect();
    kinds.sort_by_key(|kind| kind.name());
    kinds
}

pub(crate) fn decode_registered_payload(
    kind_name: &str,
    payload_json: serde_json::Value,
) -> Option<serde_json::Result<Box<dyn EventPayload>>> {
    let registry = global_registry();
    let registration = *registry
        .read()
        .expect("event registry lock")
        .get(kind_name)?;
    Some((registration.decoder)(payload_without_kind(payload_json)))
}

pub(crate) fn event_kind_for_payload(kind_name: &str) -> EventKind {
    core_event_kind(kind_name)
        .or_else(|| registered_event_kind(kind_name))
        .unwrap_or_else(|| kind::opaque_event_kind(kind_name))
}

fn global_registry() -> &'static RwLock<HashMap<&'static str, EventPayloadRegistration>> {
    static REGISTRY: OnceLock<RwLock<HashMap<&'static str, EventPayloadRegistration>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

fn decode_payload<T>(payload_json: serde_json::Value) -> serde_json::Result<Box<dyn EventPayload>>
where
    T: EventPayload + DeserializeOwned,
{
    Ok(Box::new(serde_json::from_value::<T>(payload_json)?) as Box<dyn EventPayload>)
}

pub(crate) fn payload_without_kind(mut payload_json: serde_json::Value) -> serde_json::Value {
    if let serde_json::Value::Object(obj) = &mut payload_json {
        obj.remove("kind");
    }
    payload_json
}
