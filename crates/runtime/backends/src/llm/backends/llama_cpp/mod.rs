//! llama.cpp adapter targeting the APXM-org `apxm` llama-server branch.

use super::openai::OpenAIBackend;
use super::openai::backend::validate_provider_dispatch;
use super::traits::StreamChunk;
use super::{LLMBackend, LLMRequest, LLMResponse};
use anyhow::Result;
use apxm_core::constants::llm::apxm::graph_hints as hint_keys;
use apxm_core::types::{
    BackendMechanismRef, GraphHintCapabilities, GraphHintField, GraphHintPlan, GraphHintProjector,
    GraphLifecycleCapability, GraphMetadata, GraphStatusSnapshot, ModelCapabilities, ModelInfo,
    ProjectionOutcome,
};
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

pub struct LlamaCppBackend {
    inner: OpenAIBackend,
}

impl LlamaCppBackend {
    pub async fn new(api_key: &str, config: Option<serde_json::Value>) -> Result<Self> {
        Ok(Self {
            inner: OpenAIBackend::new(api_key, config).await?,
        })
    }

    fn inject_hints(mut request: LLMRequest) -> LLMRequest {
        let Some(hints) = request.apxm_hints.clone() else {
            return request;
        };
        let mut extra = request
            .extra_body
            .take()
            .unwrap_or_else(|| serde_json::json!({}));
        if let serde_json::Value::Object(ref mut map) = extra {
            map.insert(
                hint_keys::ENVELOPE.into(),
                serde_json::to_value(&hints).unwrap_or_default(),
            );
            if hints.prefers_reuse() {
                map.insert(
                    hint_keys::LLAMA_CACHE_PROMPT.into(),
                    serde_json::json!(true),
                );
            }
        }
        request.extra_body = Some(extra);
        request
    }
}

#[async_trait]
impl LLMBackend for LlamaCppBackend {
    async fn generate(&self, request: LLMRequest) -> Result<LLMResponse> {
        validate_provider_dispatch(&request)?;
        self.inner.generate(Self::inject_hints(request)).await
    }

    fn generate_stream(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + '_>> {
        self.inner.generate_stream(Self::inject_hints(request))
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
    fn graph_hint_capabilities(&self) -> GraphHintCapabilities {
        let mut caps = GraphHintCapabilities::none();
        caps.lifecycle = GraphLifecycleCapability::NotNeeded;
        caps.fields.insert(
            GraphHintField::ReusePreference,
            apxm_core::types::GraphHintFieldCapability::Direct {
                evidence: [apxm_core::types::EvidenceKind::AdapterProjection]
                    .into_iter()
                    .collect(),
            },
        );
        caps.fields.insert(
            GraphHintField::Scope,
            apxm_core::types::GraphHintFieldCapability::Direct {
                evidence: [apxm_core::types::EvidenceKind::AdapterProjection]
                    .into_iter()
                    .collect(),
            },
        );
        caps
    }

    fn plan_graph_hints(
        &self,
        hints: Option<&apxm_core::types::ApxmGraphHints>,
    ) -> Result<GraphHintPlan, String> {
        let Some(hints) = hints else {
            return Ok(GraphHintPlan {
                outcomes: Default::default(),
            });
        };
        hints.validate()?;
        let mut outcomes = GraphHintPlan::omitted_unsupported().outcomes;
        outcomes.insert(
            GraphHintField::Scope,
            ProjectionOutcome::Applied {
                mechanism_ref: BackendMechanismRef::LlamaApxmEnvelope,
            },
        );
        if hints.prefers_reuse() {
            outcomes.insert(
                GraphHintField::ReusePreference,
                ProjectionOutcome::Applied {
                    mechanism_ref: BackendMechanismRef::LlamaCachePrompt,
                },
            );
        }
        Ok(GraphHintPlan { outcomes })
    }
}
