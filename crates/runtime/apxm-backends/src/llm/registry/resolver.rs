//! Request routing and backend selection logic.
//!
//! Determines which backend to use for a given request based on
//! explicit selection, operation type, model name, or defaults.

use super::health::{HealthMonitor, HealthStatus};
use crate::llm::backends::{LLMBackend, LLMRequest};
use anyhow::Result;
use apxm_core::types::AISOperationType;
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Routing strategy determines how backends are selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RoutingStrategy {
    /// Always use the first healthy backend
    #[default]
    FirstHealthy,
    /// Round-robin across healthy backends
    RoundRobin,
    /// Prefer backends with lowest latency
    LowLatency,
}

/// Selection criteria extracted from a request.
#[derive(Debug, Clone)]
pub struct SelectionCriteria {
    /// Explicitly requested backend name
    pub backend: Option<String>,
    /// Requested model name
    pub model: Option<String>,
    /// AIS operation type associated with the request.
    pub operation: Option<AISOperationType>,
}

impl SelectionCriteria {
    /// Extract selection criteria from a request.
    pub fn from_request(request: &LLMRequest) -> Self {
        SelectionCriteria {
            backend: request.backend.clone(),
            model: request.model.clone(),
            operation: request.operation_type.clone(),
        }
    }
}

/// Resolve which backend to use for a request.
pub fn resolve(
    criteria: &SelectionCriteria,
    backends: &Arc<RwLock<HashMap<String, Arc<dyn LLMBackend>>>>,
    operation_defaults: &Arc<DashMap<AISOperationType, String>>,
    default_backend: &Arc<RwLock<Option<String>>>,
    health_monitor: &HealthMonitor,
    strategy: &RoutingStrategy,
    round_robin_counter: &Arc<AtomicUsize>,
) -> Result<String> {
    // Priority 1: Explicit backend selection
    if let Some(ref backend_name) = criteria.backend {
        if backends.read().contains_key(backend_name) {
            return Ok(backend_name.clone());
        }
        anyhow::bail!("Explicitly requested backend '{}' not found", backend_name);
    }

    // Priority 2: Model-based routing
    if let Some(ref model) = criteria.model
        && let Some(backend_name) = find_backend_for_model(backends, model)
    {
        return Ok(backend_name);
    }

    // Priority 3: Operation-specific default
    if let Some(operation) = criteria.operation
        && let Some(entry) = operation_defaults.get(&operation)
    {
        let backend_name = entry.value().clone();
        if backends.read().contains_key(&backend_name) {
            return Ok(backend_name);
        }
    }

    // Priority 4: Global default backend
    {
        let default = default_backend.read();
        if let Some(ref backend_name) = *default
            && backends.read().contains_key(backend_name)
        {
            return Ok(backend_name.clone());
        }
    }

    // Priority 5: Select based on strategy
    select_by_strategy(backends, health_monitor, strategy, round_robin_counter)
}

/// Find a backend that supports the given model.
fn find_backend_for_model(
    backends: &Arc<RwLock<HashMap<String, Arc<dyn LLMBackend>>>>,
    model: &str,
) -> Option<String> {
    // Only use explicit backend registrations. Provider-name heuristics belong
    // outside the registry because they undermine APXM's registration model.
    let guard = backends.read();
    for (name, backend) in guard.iter() {
        if backend.model() == model {
            return Some(name.clone());
        }
    }

    None
}

/// Select a backend based on routing strategy.
fn select_by_strategy(
    backends: &Arc<RwLock<HashMap<String, Arc<dyn LLMBackend>>>>,
    health_monitor: &HealthMonitor,
    strategy: &RoutingStrategy,
    round_robin_counter: &Arc<AtomicUsize>,
) -> Result<String> {
    if backends.read().is_empty() {
        anyhow::bail!(
            "No backends registered in the LLM registry.\n\
             Check that the APXM backend configuration has [[backends]] entries."
        );
    }

    match strategy {
        RoutingStrategy::FirstHealthy => select_first_healthy(backends, health_monitor),
        RoutingStrategy::RoundRobin => {
            select_round_robin(backends, health_monitor, round_robin_counter)
        }
        RoutingStrategy::LowLatency => select_low_latency(backends, health_monitor),
    }
}

/// Select the first healthy backend.
fn select_first_healthy(
    backends: &Arc<RwLock<HashMap<String, Arc<dyn LLMBackend>>>>,
    health_monitor: &HealthMonitor,
) -> Result<String> {
    let guard = backends.read();

    // Try to find a healthy backend
    for name in guard.keys() {
        let status = health_monitor.status(name);
        if status == HealthStatus::Healthy || status == HealthStatus::Unknown {
            return Ok(name.clone());
        }
    }

    // If no healthy backend, try degraded
    for name in guard.keys() {
        let status = health_monitor.status(name);
        if status == HealthStatus::Degraded {
            return Ok(name.clone());
        }
    }

    // Last resort: return any backend
    if let Some(name) = guard.keys().next() {
        Ok(name.clone())
    } else {
        anyhow::bail!("No backends registered (unexpected)")
    }
}

/// Select backend using round-robin across healthy backends.
fn select_round_robin(
    backends: &Arc<RwLock<HashMap<String, Arc<dyn LLMBackend>>>>,
    health_monitor: &HealthMonitor,
    counter: &Arc<AtomicUsize>,
) -> Result<String> {
    let guard = backends.read();

    // Collect healthy backends
    let healthy: Vec<String> = guard
        .keys()
        .filter(|name| {
            let status = health_monitor.status(name);
            status == HealthStatus::Healthy || status == HealthStatus::Unknown
        })
        .cloned()
        .collect();

    drop(guard);

    if healthy.is_empty() {
        // Fall back to first_healthy logic (which includes degraded backends)
        return select_first_healthy(backends, health_monitor);
    }

    // Atomic round-robin selection
    let idx = counter.fetch_add(1, Ordering::Relaxed) % healthy.len();
    Ok(healthy[idx].clone())
}

/// Select backend with lowest average latency.
fn select_low_latency(
    backends: &Arc<RwLock<HashMap<String, Arc<dyn LLMBackend>>>>,
    health_monitor: &HealthMonitor,
) -> Result<String> {
    let guard = backends.read();
    let mut best_backend: Option<(String, std::time::Duration)> = None;

    for name in guard.keys() {
        let status = health_monitor.status(name);

        if status == HealthStatus::Unhealthy {
            continue;
        }

        if let Some(avg_latency) = health_monitor.average_latency(name) {
            match &best_backend {
                None => {
                    best_backend = Some((name.clone(), avg_latency));
                }
                Some((_, best_latency)) => {
                    if avg_latency < *best_latency {
                        best_backend = Some((name.clone(), avg_latency));
                    }
                }
            }
        }
    }

    drop(guard);

    if let Some((name, _)) = best_backend {
        Ok(name)
    } else {
        select_first_healthy(backends, health_monitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routing_strategy_default() {
        assert_eq!(RoutingStrategy::default(), RoutingStrategy::FirstHealthy);
    }

    #[test]
    fn test_model_matching() {
        let backends = Arc::new(RwLock::new(HashMap::<String, Arc<dyn LLMBackend>>::new()));
        assert!(find_backend_for_model(&backends, "gpt-4").is_none());
    }
}
