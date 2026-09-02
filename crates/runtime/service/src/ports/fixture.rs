//! The deterministic development inference backend.
//!
//! A Runtime Service started without any hosted provider has an empty backend
//! roster, so every `model.call` fails before send with
//! `model_target_not_registered` and the whole invocation commits as failed.
//! That is correct for a deployment and useless for development and tests: a
//! program that composes one model call with host capabilities cannot be run
//! at all without a provider account.
//!
//! This backend closes that gap without inventing a provider. It is selected
//! only by an explicit `APXM_BACKEND=fixture`, it serves only the exact model
//! references `APXM_BACKEND_MODEL` names, and its answer is a pure function of
//! the request: the same request always produces the same completion, and a
//! different request always produces a different one. The completion is
//! `apxm-fixture:<sha256 of the request>`, which no provider would ever
//! return, so a fixture answer can never be mistaken for a real one in a
//! transcript, a receipt or a report.
//!
//! Nothing here is a fallback. With `APXM_BACKEND` unset the roster is the
//! only source of backends, exactly as before; with it set to anything else
//! the selection is refused and recorded. Registration is also refused when a
//! roster backend already binds one of the named references, so an
//! intentionally configured provider is never displaced by a fixture.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::Arc;

use apxm_backends::llm::LLMRegistry;
use apxm_backends::llm::backends::{LLMBackend, LLMRequest, LLMResponse, Role};
use apxm_core::types::{FinishReason, GraphHintProjector, ModelInfo, TokenUsage};
use async_trait::async_trait;
use sha2::{Digest, Sha256};

/// The one `APXM_BACKEND` value that selects a development backend.
pub const FIXTURE_SELECTOR: &str = "fixture";

/// The registry name the fixture backend registers under.
pub const FIXTURE_BACKEND_NAME: &str = "fixture";

/// The prefix every fixture completion carries.
///
/// It is part of the contract, not decoration: a consumer reading a receipt
/// can tell a development answer from a provider answer by this prefix alone.
pub const FIXTURE_COMPLETION_PREFIX: &str = "apxm-fixture:";

/// Deterministic tokens-per-character divisor.
///
/// The fixture reports a stated estimate rather than a provider count. Four
/// characters per token is the estimate, applied to the request rendering and
/// to the completion, so usage is a function of the request like everything
/// else here.
const CHARACTERS_PER_TOKEN: usize = 4;

/// A backend whose completion is a pure function of the request.
pub struct FixtureBackend {
    /// The exact model references this backend was bound to, in the order the
    /// selection named them. The first is what `model()` reports.
    models: Vec<String>,
}

impl FixtureBackend {
    /// Build a fixture backend that serves the supplied exact model references.
    pub fn new(models: Vec<String>) -> Self {
        Self { models }
    }

    /// Render the request into the exact bytes the completion digests.
    ///
    /// Every authored field the local model port can carry appears here, so
    /// two requests that differ in any of them get different completions. The
    /// rendering is line-oriented and prefixed per field, so no two distinct
    /// requests can render to the same string.
    fn render(request: &LLMRequest) -> String {
        let mut rendered = String::new();
        rendered.push_str("model\t");
        rendered.push_str(request.model.as_deref().unwrap_or(""));
        rendered.push_str("\nprompt\t");
        rendered.push_str(&request.prompt.replace('\n', "\\n"));
        rendered.push('\n');
        for message in &request.messages {
            let role = match message.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };
            rendered.push_str("message\t");
            rendered.push_str(role);
            rendered.push('\t');
            rendered.push_str(&message.text_content().replace('\n', "\\n"));
            rendered.push('\n');
        }
        let _ = writeln!(rendered, "temperature\t{}", request.temperature);
        let _ = writeln!(rendered, "max_tokens\t{:?}", request.max_tokens);
        let _ = writeln!(rendered, "top_p\t{:?}", request.top_p);
        for stop in &request.stop_sequences {
            rendered.push_str("stop\t");
            rendered.push_str(&stop.replace('\n', "\\n"));
            rendered.push('\n');
        }
        rendered
    }

    fn completion_of(rendered: &str) -> String {
        let digest = Sha256::digest(rendered.as_bytes());
        format!("{FIXTURE_COMPLETION_PREFIX}{digest:x}")
    }

    fn response(&self, request: &LLMRequest) -> LLMResponse {
        let rendered = Self::render(request);
        let content = Self::completion_of(&rendered);
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| self.model().to_owned());
        let usage = TokenUsage::new(
            rendered.chars().count().div_ceil(CHARACTERS_PER_TOKEN),
            content.chars().count().div_ceil(CHARACTERS_PER_TOKEN),
        );
        LLMResponse::new(content, model, usage, FinishReason::Stop)
    }
}

impl GraphHintProjector for FixtureBackend {}

#[async_trait]
impl LLMBackend for FixtureBackend {
    async fn generate(&self, request: LLMRequest) -> anyhow::Result<LLMResponse> {
        Ok(self.response(&request))
    }

    fn name(&self) -> &str {
        FIXTURE_BACKEND_NAME
    }

    fn model(&self) -> &str {
        self.models
            .first()
            .map_or(FIXTURE_BACKEND_NAME, String::as_str)
    }

    async fn health_check(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        Ok(self
            .models
            .iter()
            .map(|model| ModelInfo {
                id: model.clone(),
                name: model.clone(),
                context_window: 0,
                supports_vision: false,
                supports_functions: false,
            })
            .collect())
    }
}

/// The exact model references a `APXM_BACKEND_MODEL` value names.
///
/// The value is a comma-separated list. Surrounding whitespace is trimmed
/// because an environment file writes `A, B`; an empty entry is dropped rather
/// than bound, because the registry admits only exact references.
pub fn model_references(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|reference| !reference.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Register the development backend the environment selects, if any.
///
/// The three inputs are the two environment values and the roster evidence the
/// caller is accumulating. Nothing is read from the process environment here,
/// so the whole selection is exercisable without mutating it.
///
/// Every refusal is pushed onto `skipped` rather than returned: an
/// unresolvable target reports why it is unresolvable, and the caller's
/// construction never fails because of a development selection.
pub fn register_selected_backend(
    registry: &LLMRegistry,
    selector: Option<&str>,
    model_value: Option<&str>,
    bound_models: &mut BTreeSet<String>,
    skipped: &mut Vec<String>,
) {
    let Some(selector) = selector.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if selector != FIXTURE_SELECTOR {
        skipped.push(format!(
            "APXM_BACKEND names '{selector}', which selects no development backend; \
             the only admitted value is '{FIXTURE_SELECTOR}'"
        ));
        return;
    }
    let models = model_value.map(model_references).unwrap_or_default();
    if models.is_empty() {
        skipped.push(format!(
            "APXM_BACKEND selects the '{FIXTURE_SELECTOR}' development backend but \
             APXM_BACKEND_MODEL names no exact model reference, so it serves nothing. \
             Set APXM_BACKEND_MODEL to the model reference the program calls, or a \
             comma-separated list of them"
        ));
        return;
    }
    let backend: Arc<dyn LLMBackend> = Arc::new(FixtureBackend::new(models.clone()));
    if let Err(error) = registry.register_arc(FIXTURE_BACKEND_NAME, backend) {
        skipped.push(format!(
            "the '{FIXTURE_SELECTOR}' development backend did not register: {error}"
        ));
        return;
    }
    for model in models {
        // A reference the roster already binds keeps its configured backend.
        // A development selection never displaces a provider somebody chose.
        match registry.bind_model(model.clone(), FIXTURE_BACKEND_NAME) {
            Ok(()) => {
                bound_models.insert(model);
            }
            Err(error) => skipped.push(format!(
                "the '{FIXTURE_SELECTOR}' development backend did not take model \
                 reference '{model}': {error}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_backends::llm::backends::Message;

    fn registry_with_selection(
        selector: Option<&str>,
        model_value: Option<&str>,
    ) -> (LLMRegistry, BTreeSet<String>, Vec<String>) {
        let registry = LLMRegistry::new();
        let mut bound = BTreeSet::new();
        let mut skipped = Vec::new();
        register_selected_backend(&registry, selector, model_value, &mut bound, &mut skipped);
        (registry, bound, skipped)
    }

    #[test]
    fn an_unset_selector_registers_nothing() {
        let (registry, bound, skipped) = registry_with_selection(None, Some("a.model"));
        assert!(registry.backend_names().is_empty());
        assert!(bound.is_empty());
        assert!(skipped.is_empty(), "silence is the unselected behaviour");
    }

    #[test]
    fn an_unknown_selector_is_refused_and_named() {
        let (registry, bound, skipped) = registry_with_selection(Some("hosted"), Some("a.model"));
        assert!(registry.backend_names().is_empty());
        assert!(bound.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].contains("hosted"), "{}", skipped[0]);
        assert!(skipped[0].contains("fixture"), "{}", skipped[0]);
    }

    #[test]
    fn the_fixture_without_a_model_reference_serves_nothing_and_says_so() {
        let (registry, bound, skipped) = registry_with_selection(Some("fixture"), None);
        assert!(registry.backend_names().is_empty());
        assert!(bound.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].contains("APXM_BACKEND_MODEL"), "{}", skipped[0]);
    }

    #[test]
    fn the_fixture_binds_every_named_reference() {
        let (registry, bound, skipped) =
            registry_with_selection(Some(" fixture "), Some("a.model, b.model ,, "));
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(registry.backend_names(), vec![FIXTURE_BACKEND_NAME]);
        assert_eq!(
            bound,
            BTreeSet::from(["a.model".to_owned(), "b.model".to_owned()])
        );
    }

    #[test]
    fn a_reference_the_roster_already_binds_keeps_its_backend() {
        let registry = LLMRegistry::new();
        registry
            .register_arc(
                "configured",
                Arc::new(FixtureBackend::new(vec!["a.model".to_owned()])),
            )
            .expect("a test backend registers");
        registry
            .bind_model("a.model", "configured")
            .expect("the roster binds the reference first");

        let mut bound = BTreeSet::new();
        let mut skipped = Vec::new();
        register_selected_backend(
            &registry,
            Some("fixture"),
            Some("a.model,b.model"),
            &mut bound,
            &mut skipped,
        );

        assert_eq!(bound, BTreeSet::from(["b.model".to_owned()]));
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].contains("a.model"), "{}", skipped[0]);
        let request = {
            let mut request = LLMRequest::new("anything");
            request.model = Some("a.model".to_owned());
            request
        };
        assert_eq!(
            registry
                .resolve_backend(&request)
                .expect("the configured binding survives"),
            "configured"
        );
    }

    #[tokio::test]
    async fn the_same_request_always_gets_the_same_completion() {
        let backend = FixtureBackend::new(vec!["a.model".to_owned()]);
        let request = || {
            let mut request = LLMRequest::new("summarise the day");
            request.model = Some("a.model".to_owned());
            request
        };

        let first = backend.generate(request()).await.expect("a fixture answer");
        let second = backend.generate(request()).await.expect("a fixture answer");

        assert_eq!(first.content, second.content);
        assert_eq!(first.model, "a.model");
        assert_eq!(first.finish_reason, FinishReason::Stop);
        assert!(
            first.content.starts_with(FIXTURE_COMPLETION_PREFIX),
            "{}",
            first.content
        );
        assert_eq!(
            first.content.len(),
            FIXTURE_COMPLETION_PREFIX.len() + 64,
            "the completion carries one sha256 of the request"
        );
        assert_eq!(first.usage.total_tokens, second.usage.total_tokens);
        assert!(first.usage.input_tokens > 0 && first.usage.output_tokens > 0);
    }

    #[tokio::test]
    async fn a_different_request_gets_a_different_completion() {
        let backend = FixtureBackend::new(vec!["a.model".to_owned()]);
        let mut base = LLMRequest::new("summarise the day");
        base.model = Some("a.model".to_owned());

        let mut other_prompt = base.clone();
        other_prompt.prompt = "summarise the week".to_owned();
        let mut other_model = base.clone();
        other_model.model = Some("b.model".to_owned());
        let mut other_messages = base.clone();
        other_messages.messages = vec![Message::text(Role::User, "and the orders")];
        let mut other_temperature = base.clone();
        other_temperature.temperature = 0.1;
        let mut other_max_tokens = base.clone();
        other_max_tokens.max_tokens = Some(64);
        let mut other_stop = base.clone();
        other_stop.stop_sequences = vec!["END".to_owned()];

        let baseline = backend
            .generate(base)
            .await
            .expect("a fixture answer")
            .content;
        for (label, request) in [
            ("prompt", other_prompt),
            ("model", other_model),
            ("messages", other_messages),
            ("temperature", other_temperature),
            ("max_tokens", other_max_tokens),
            ("stop_sequences", other_stop),
        ] {
            let answered = backend
                .generate(request)
                .await
                .expect("a fixture answer")
                .content;
            assert_ne!(
                baseline, answered,
                "a request differing only in {label} must not reuse the completion"
            );
        }
    }

    #[tokio::test]
    async fn the_fixture_is_healthy_and_lists_what_it_serves() {
        let backend = FixtureBackend::new(vec!["a.model".to_owned(), "b.model".to_owned()]);
        backend.health_check().await.expect("always reachable");
        let models = backend.list_models().await.expect("the bound references");
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a.model", "b.model"]
        );
        assert_eq!(backend.model(), "a.model");
        assert_eq!(backend.name(), FIXTURE_BACKEND_NAME);
    }
}
