//! Compiler-evidence-driven correlated model batch dispatch.
//!
//! The scheduler supplies only nodes that share a compiler-stamped batch group.
//! This coordinator waits for a bounded ready subset, invokes the backend's
//! correlated transport once, and returns each outcome to its original node.
//! When runtime capability evidence is absent, every caller falls back to the
//! existing per-node model dispatch path.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use apxm_backends::{CorrelatedLLMOutcome, CorrelatedLLMRequest, LLMRegistry, LLMRequest};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, NodeId, execution::ExecutionDag};
use parking_lot::Mutex;
use tokio::sync::oneshot;

#[derive(Debug)]
pub(crate) enum CorrelatedBatchDispatch {
    NotMember,
    Fallback,
    Response(apxm_backends::LLMResponse),
}

#[derive(Clone)]
pub(crate) struct CorrelatedBatchDispatcher {
    registry: Arc<LLMRegistry>,
    node_groups: Arc<HashMap<NodeId, String>>,
    groups: Arc<Mutex<HashMap<String, BatchGroup>>>,
    scheduler_batch_limit: usize,
}

struct BatchGroup {
    member_count: usize,
    completed: usize,
    route: Option<BatchRoute>,
    pending: BTreeMap<NodeId, PendingRequest>,
}

#[derive(Clone, PartialEq, Eq)]
struct BatchRoute {
    backend_name: String,
    model: String,
    max_batch_size: usize,
}

struct PendingRequest {
    request: LLMRequest,
    sender: oneshot::Sender<Result<apxm_backends::LLMResponse, String>>,
}

impl CorrelatedBatchDispatcher {
    pub(crate) fn from_dag(
        registry: Arc<LLMRegistry>,
        dag: &ExecutionDag,
        scheduler_batch_limit: usize,
    ) -> Option<Arc<Self>> {
        if scheduler_batch_limit == 0 {
            return None;
        }

        let mut grouped: BTreeMap<String, Vec<NodeId>> = BTreeMap::new();
        for node in &dag.nodes {
            if !matches!(
                node.op_type,
                AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
            ) {
                continue;
            }
            let Some(group_id) = node
                .attributes
                .get(graph_attrs::BATCH_GROUP)
                .and_then(apxm_core::types::Value::as_str)
                .map(str::trim)
                .filter(|group_id| !group_id.is_empty())
            else {
                continue;
            };
            grouped
                .entry(group_id.to_string())
                .or_default()
                .push(node.id);
        }

        let mut node_groups = HashMap::new();
        let mut groups = HashMap::new();
        for (group_id, mut members) in grouped {
            if members.len() < 2 {
                continue;
            }
            members.sort_unstable();
            for node_id in &members {
                node_groups.insert(*node_id, group_id.clone());
            }
            groups.insert(
                group_id,
                BatchGroup {
                    member_count: members.len(),
                    completed: 0,
                    route: None,
                    pending: BTreeMap::new(),
                },
            );
        }

        (!groups.is_empty()).then(|| {
            Arc::new(Self {
                registry,
                node_groups: Arc::new(node_groups),
                groups: Arc::new(Mutex::new(groups)),
                scheduler_batch_limit,
            })
        })
    }

    pub(crate) async fn dispatch(
        &self,
        node_id: NodeId,
        request: LLMRequest,
    ) -> Result<CorrelatedBatchDispatch, String> {
        let Some(group_id) = self.node_groups.get(&node_id) else {
            return Ok(CorrelatedBatchDispatch::NotMember);
        };
        let Some(route) = self
            .registry
            .correlated_batch_route(&request)
            .map_err(|error| error.to_string())?
        else {
            return Ok(CorrelatedBatchDispatch::Fallback);
        };
        let route = BatchRoute {
            backend_name: route.backend_name,
            model: route.model,
            max_batch_size: route.max_batch_size,
        };

        let (sender, receiver) = oneshot::channel();
        let ready_batch = {
            let mut groups = self.groups.lock();
            let group = groups
                .get_mut(group_id)
                .expect("node group map only references known groups");

            if let Some(existing_route) = &group.route {
                if existing_route != &route {
                    let pending = std::mem::take(&mut group.pending);
                    for (_, pending_request) in pending {
                        let _ = pending_request.sender.send(Err(
                            "compiler batch group resolved to different runtime routes".to_string(),
                        ));
                    }
                    return Ok(CorrelatedBatchDispatch::Fallback);
                }
            } else {
                group.route = Some(route.clone());
            }

            group
                .pending
                .insert(node_id, PendingRequest { request, sender });
            let remaining_members = group.member_count.saturating_sub(group.completed);
            let target = remaining_members
                .min(self.scheduler_batch_limit)
                .min(route.max_batch_size);
            if target == 0 || group.pending.len() < target {
                None
            } else {
                let selected_ids = group
                    .pending
                    .keys()
                    .copied()
                    .take(target)
                    .collect::<Vec<_>>();
                let selected = selected_ids
                    .into_iter()
                    .filter_map(|id| group.pending.remove(&id).map(|pending| (id, pending)))
                    .collect::<Vec<_>>();
                group.completed = group.completed.saturating_add(selected.len());
                Some(selected)
            }
        };

        if let Some(selected) = ready_batch {
            let requests = selected
                .iter()
                .map(|(id, pending)| CorrelatedLLMRequest {
                    correlation_id: id.to_string(),
                    request: pending.request.clone(),
                })
                .collect::<Vec<_>>();
            match self.registry.generate_correlated_batch(requests).await {
                Ok(outcomes) => {
                    let outcome_by_id = outcomes
                        .into_iter()
                        .map(|outcome| (outcome.correlation_id().to_string(), outcome))
                        .collect::<HashMap<_, _>>();
                    for (id, pending) in selected {
                        let result = match outcome_by_id.get(&id.to_string()) {
                            Some(CorrelatedLLMOutcome::Response { response, .. }) => {
                                Ok(response.clone())
                            }
                            Some(CorrelatedLLMOutcome::Failure { message, .. }) => {
                                Err(message.clone())
                            }
                            None => Err("backend omitted a correlated batch outcome".to_string()),
                        };
                        let _ = pending.sender.send(result);
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    for (_, pending) in selected {
                        let _ = pending.sender.send(Err(message.clone()));
                    }
                }
            }
        }

        match receiver.await {
            Ok(Ok(response)) => Ok(CorrelatedBatchDispatch::Response(response)),
            Ok(Err(message)) => Err(message),
            Err(_) => {
                Err("correlated batch coordinator closed before dispatch completed".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_backends::llm::backends::MockLLMBackend;
    use apxm_core::types::execution::Node;

    fn grouped_dag() -> ExecutionDag {
        let mut left = Node::new(1, AISOperationType::Ask);
        let mut right = Node::new(2, AISOperationType::Ask);
        for node in [&mut left, &mut right] {
            node.attributes.insert(
                graph_attrs::BATCH_GROUP.to_string(),
                apxm_core::types::Value::String("compiler-group".to_string()),
            );
        }
        let mut dag = ExecutionDag::new();
        dag.nodes = vec![left, right];
        dag
    }

    #[tokio::test]
    async fn dispatches_compiler_group_through_one_correlated_submission() {
        let registry = Arc::new(LLMRegistry::new());
        let backend = MockLLMBackend::static_response("batched").model_name("fixture-model");
        registry.register("mock", backend.clone()).expect("backend");
        registry
            .set_model_route("fixture-model", "mock")
            .expect("route");
        let dispatcher = CorrelatedBatchDispatcher::from_dag(registry, &grouped_dag(), 2)
            .expect("compiler group dispatcher");

        let left = dispatcher.dispatch(1, LLMRequest::new("left").with_model("fixture-model"));
        let right = dispatcher.dispatch(2, LLMRequest::new("right").with_model("fixture-model"));
        let (left, right) = tokio::join!(left, right);

        assert!(matches!(left, Ok(CorrelatedBatchDispatch::Response(_))));
        assert!(matches!(right, Ok(CorrelatedBatchDispatch::Response(_))));
        assert_eq!(backend.batch_submission_count(), 1);
    }
}
