//! llama.cpp adapter targeting the APXM-org `apxm` llama-server branch.

use super::graph_hint_dispatch::{record_graph_hint_evidence, stream_with_graph_hint_evidence};
use super::openai::OpenAIBackend;
use super::openai::backend::validate_provider_dispatch;
use super::traits::StreamChunk;
use super::{LLMBackend, LLMRequest, LLMResponse};
use anyhow::Result;
use apxm_core::constants::llm::apxm::HINTS_FIELD;
use apxm_core::types::{
    ApxmGraphHints, BackendMechanismRef, GraphHintCapabilities, GraphHintDispatchProjection,
    GraphHintField, GraphHintFieldCapability, GraphHintPlan, GraphHintProjector,
    GraphLifecycleCapability, GraphMetadata, GraphStatusSnapshot, ModelCapabilities, ModelInfo,
    ProjectionOutcome,
};
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

/// llama.cpp-owned request mechanisms. The common contract crate names none of
/// them; this adapter is their only owner.
mod mechanisms {
    /// Server option reusing a common prompt prefix.
    pub const CACHE_PROMPT: &str = "cache_prompt";

    /// Closed mechanism identifiers this adapter cites in evidence.
    pub const APXM_ENVELOPE: &str = "llama.apxm_envelope";
    pub const CACHE_PROMPT_MECHANISM: &str = "llama.cache_prompt";
}

fn mechanism(name: &str) -> BackendMechanismRef {
    BackendMechanismRef::new(name).expect("adapter-owned mechanism names are well formed")
}

pub struct LlamaCppBackend {
    inner: OpenAIBackend,
}

impl LlamaCppBackend {
    pub async fn new(api_key: &str, config: Option<serde_json::Value>) -> Result<Self> {
        Ok(Self {
            inner: OpenAIBackend::new(api_key, config).await?,
        })
    }

    /// Shape one provider request through the projector. Only fields the plan
    /// authorized reach `extra_body`.
    pub(crate) fn inject_hints(
        &self,
        mut request: LLMRequest,
        attempt: u32,
    ) -> Result<(LLMRequest, Option<GraphHintDispatchProjection>)> {
        let projected = self
            .project_graph_hints(request.apxm_hints.as_ref(), attempt)
            .map_err(|error| {
                anyhow::anyhow!("llama.cpp graph-hint projection rejected: {error}")
            })?;
        if projected.provider_fields.is_empty() {
            return Ok((request, Some(projected)));
        }
        let mut extra = request
            .extra_body
            .take()
            .unwrap_or_else(|| serde_json::json!({}));
        if let serde_json::Value::Object(ref mut map) = extra {
            for (key, value) in projected.provider_fields.clone() {
                map.entry(key).or_insert(value);
            }
        }
        request.extra_body = Some(extra);
        Ok((request, Some(projected)))
    }
}

#[async_trait]
impl LLMBackend for LlamaCppBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        validate_provider_dispatch(&request)?;
        let (request, projected) = self.inject_hints(request, 0)?;
        let response = self.inner.generate(request).await?;
        Ok(record_graph_hint_evidence(response, projected.as_ref()))
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + '_>> {
        let (request, projected) = match self.inject_hints(request, 0) {
            Ok(injected) => injected,
            Err(error) => return Box::pin(tokio_stream::iter(vec![Err(error)])),
        };
        stream_with_graph_hint_evidence(self.inner.generate_stream(request), projected)
    }

    fn name(&self) -> &str {
        crate::llm::ProviderProtocol::LlamaCpp.as_str()
    }

    fn model(&self) -> &str {
        self.inner.model()
    }

    async fn health_check(&self) -> Result<()> {
        self.inner.health_check().await
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.inner.list_models().await
    }

    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }

    async fn register_graph(&self, _metadata: GraphMetadata) -> Result<()> {
        Ok(())
    }

    async fn get_graph_status(&self, _graph_id: &str) -> Result<Option<GraphStatusSnapshot>> {
        Ok(None)
    }
}

impl GraphHintProjector for LlamaCppBackend {
    /// Topology, priority, affinity, horizon, warmup, and pipeline eligibility
    /// have no llama.cpp mechanism with the same useful effect, so this binding
    /// declares exactly two supported fields and nothing more.
    fn graph_hint_capabilities(&self) -> GraphHintCapabilities {
        let mut capabilities = GraphHintCapabilities::none();
        capabilities.lifecycle = GraphLifecycleCapability::NotNeeded;
        for field in [GraphHintField::Scope, GraphHintField::ReusePreference] {
            capabilities
                .fields
                .insert(field, GraphHintFieldCapability::Direct);
        }
        capabilities
    }

    fn plan_graph_hints(&self, hints: Option<&ApxmGraphHints>) -> Result<GraphHintPlan, String> {
        let capabilities = self.graph_hint_capabilities();
        let Some(hints) = hints else {
            return Ok(GraphHintPlan::absent(&capabilities));
        };
        hints.validate()?;
        let mut plan = GraphHintPlan::omitted_unsupported(hints, &capabilities).with_outcome(
            GraphHintField::Scope,
            ProjectionOutcome::Applied {
                mechanism_ref: mechanism(mechanisms::APXM_ENVELOPE),
            },
        );
        if hints.prefers_reuse() {
            plan = plan.with_outcome(
                GraphHintField::ReusePreference,
                ProjectionOutcome::Applied {
                    mechanism_ref: mechanism(mechanisms::CACHE_PROMPT_MECHANISM),
                },
            );
        }
        Ok(plan)
    }

    fn render_graph_hint_fields(
        &self,
        hints: &ApxmGraphHints,
        plan: &GraphHintPlan,
    ) -> Result<serde_json::Map<String, serde_json::Value>, String> {
        let mut fields = serde_json::Map::new();
        if let Some(envelope) = hints.project_envelope(plan) {
            fields.insert(HINTS_FIELD.to_owned(), envelope);
        }
        if plan.projects(GraphHintField::ReusePreference) {
            fields.insert(mechanisms::CACHE_PROMPT.to_owned(), serde_json::json!(true));
        }
        Ok(fields)
    }
}
