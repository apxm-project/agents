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
            operation: request.operation_type,
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

    // Priority 2: Model-based routing — collect every backend whose model()
    // matches and apply the routing strategy across that subset. A first-match
    // short-circuit here would pin all replica traffic to one backend and
    // defeat `replicas > 1`.
    if let Some(ref model) = criteria.model {
        let candidates = find_backends_for_model(backends, model);
        if !candidates.is_empty() {
            return select_by_strategy(
                &candidates,
                health_monitor,
                strategy,
                round_robin_counter,
                Some(model.as_str()),
            );
        }
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

    // Priority 5: Select across all backends based on strategy
    let all: Vec<String> = backends.read().keys().cloned().collect();
    select_by_strategy(&all, health_monitor, strategy, round_robin_counter, None)
}

/// Collect every backend whose `model()` matches the requested model id.
///
/// Returning every match (rather than the first) lets the routing strategy
/// distribute traffic across replicas registered under distinct backend names
/// but sharing the same served-model id.
fn find_backends_for_model(
    backends: &Arc<RwLock<HashMap<String, Arc<dyn LLMBackend>>>>,
    model: &str,
) -> Vec<String> {
    // Only use explicit backend registrations. Provider-name heuristics belong
    // outside the registry because they undermine APXM's registration model.
    let guard = backends.read();
    let mut matches: Vec<String> = guard
        .iter()
        .filter_map(|(name, backend)| {
            if backend.model() == model {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();
    // Deterministic order is required for round-robin: the AtomicUsize counter
    // indexes into this vector, and an unstable iteration order would scramble
    // the distribution across requests.
    matches.sort();
    matches
}

/// Select a backend from `candidates` using the configured strategy.
///
/// `candidates` is the pre-filtered set (e.g. all backends serving a
/// particular model, or every registered backend when no model filter
/// applies). `model_filter` is only used to build a clearer error message
/// when no candidate is routable.
fn select_by_strategy(
    candidates: &[String],
    health_monitor: &HealthMonitor,
    strategy: &RoutingStrategy,
    round_robin_counter: &Arc<AtomicUsize>,
    model_filter: Option<&str>,
) -> Result<String> {
    if candidates.is_empty() {
        anyhow::bail!(
            "No backends registered in the LLM registry.\n\
             Check that the APXM backend configuration has [[backends]] entries."
        );
    }

    match strategy {
        RoutingStrategy::FirstHealthy => {
            select_first_healthy(candidates, health_monitor, model_filter)
        }
        RoutingStrategy::RoundRobin => select_round_robin(
            candidates,
            health_monitor,
            round_robin_counter,
            model_filter,
        ),
        RoutingStrategy::LowLatency => select_low_latency(candidates, health_monitor, model_filter),
    }
}

/// Select the first healthy (or degraded) backend.
///
/// Unhealthy backends are never returned: surfacing an explicit error keeps
/// misconfigurations visible at the call site instead of silently masking
/// them by routing to a known-bad backend.
fn select_first_healthy(
    candidates: &[String],
    health_monitor: &HealthMonitor,
    model_filter: Option<&str>,
) -> Result<String> {
    for name in candidates {
        let status = health_monitor.status(name);
        if status == HealthStatus::Healthy || status == HealthStatus::Unknown {
            return Ok(name.clone());
        }
    }

    for name in candidates {
        if health_monitor.status(name) == HealthStatus::Degraded {
            return Ok(name.clone());
        }
    }

    Err(no_healthy_backends_error(candidates, model_filter))
}

/// Round-robin across the candidates that are currently Healthy.
///
/// `Unknown` is treated as Unhealthy for round-robin: a backend that has
/// never been probed must not silently absorb production traffic.
/// `Degraded` is included so a single misbehaving replica does not collapse
/// the whole pool, but it never falls back to returning an unhealthy
/// backend — exhausted pools surface as an explicit routing error.
fn select_round_robin(
    candidates: &[String],
    health_monitor: &HealthMonitor,
    counter: &Arc<AtomicUsize>,
    model_filter: Option<&str>,
) -> Result<String> {
    let routable: Vec<&String> = candidates
        .iter()
        .filter(|name| {
            matches!(
                health_monitor.status(name),
                HealthStatus::Healthy | HealthStatus::Degraded
            )
        })
        .collect();

    if routable.is_empty() {
        return Err(no_healthy_backends_error(candidates, model_filter));
    }

    let idx = counter.fetch_add(1, Ordering::Relaxed) % routable.len();
    Ok(routable[idx].clone())
}

/// Select backend with lowest average latency. Ignores Unhealthy backends.
fn select_low_latency(
    candidates: &[String],
    health_monitor: &HealthMonitor,
    model_filter: Option<&str>,
) -> Result<String> {
    let mut best_backend: Option<(String, std::time::Duration)> = None;

    for name in candidates {
        if health_monitor.status(name) == HealthStatus::Unhealthy {
            continue;
        }

        if let Some(avg_latency) = health_monitor.average_latency(name) {
            match &best_backend {
                None => best_backend = Some((name.clone(), avg_latency)),
                Some((_, best_latency)) if avg_latency < *best_latency => {
                    best_backend = Some((name.clone(), avg_latency));
                }
                _ => {}
            }
        }
    }

    if let Some((name, _)) = best_backend {
        Ok(name)
    } else {
        // No backend has observed latency yet; defer to first-healthy
        // semantics over the same candidate set so the error path is
        // consistent (and still hard-fails when nothing is routable).
        select_first_healthy(candidates, health_monitor, model_filter)
    }
}

fn no_healthy_backends_error(candidates: &[String], model_filter: Option<&str>) -> anyhow::Error {
    if let Some(model) = model_filter {
        anyhow::anyhow!(
            "no healthy backends for model '{}'; candidates = {:?}",
            model,
            candidates
        )
    } else {
        anyhow::anyhow!("no healthy backends; candidates = {:?}", candidates)
    }
}
