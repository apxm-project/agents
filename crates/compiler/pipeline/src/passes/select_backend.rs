use crate::air_builder::AirModule;
use apxm_backends::llm::{BackendConfig, ModelConfig, ProviderProtocol};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, Value};
use thiserror::Error;

const RUNTIME_RESOLVES_MODEL: &[ProviderProtocol] = &[
    ProviderProtocol::Anthropic,
    ProviderProtocol::OpenAI,
    ProviderProtocol::Google,
    ProviderProtocol::Ollama,
    ProviderProtocol::Mock,
];

#[derive(Debug, Error)]
pub enum SelectBackendError {
    #[error("failed to parse backend catalog: {0}")]
    ParseCatalog(#[from] toml::de::Error),
    #[error(
        "node `{node}` references backend `{backend}`, but it is not registered. Registered backends: {registered}"
    )]
    UnknownBackend {
        node: String,
        backend: String,
        registered: String,
    },
    #[error(
        "node `{node}` requested protocol `{protocol}`, but no registered backends use it. Registered backends: {registered}"
    )]
    UnknownProtocol {
        node: String,
        protocol: String,
        registered: String,
    },
    #[error(
        "node `{node}` requested model `{model}`, but no registered backend/model matched it. Available routes: {routes}"
    )]
    UnknownModel {
        node: String,
        model: String,
        routes: String,
    },
    #[error("node `{node}` has ambiguous backend/model route `{model}`. Matched routes: {routes}")]
    AmbiguousModel {
        node: String,
        model: String,
        routes: String,
    },
    #[error(
        "node `{node}` selected backend `{backend}`, but it has no registered models. Add one or use a runtime-resolving protocol."
    )]
    NoModels { node: String, backend: String },
    #[error(
        "node `{node}` selected backend `{backend}`, but it serves multiple models. Set `{model_attr}` explicitly."
    )]
    AmbiguousImplicitModel {
        node: String,
        backend: String,
        model_attr: &'static str,
    },
}

#[derive(serde::Deserialize)]
struct BackendCatalog {
    #[serde(default)]
    backends: Vec<BackendConfig>,
}

pub fn parse_backend_catalog_toml(
    toml_text: &str,
) -> Result<Vec<BackendConfig>, SelectBackendError> {
    Ok(toml::from_str::<BackendCatalog>(toml_text)?.backends)
}

pub fn select_backend_from_toml(
    module: &mut AirModule,
    toml_text: Option<&str>,
) -> Result<usize, SelectBackendError> {
    let Some(toml_text) = toml_text else {
        return Ok(0);
    };
    let backends = parse_backend_catalog_toml(toml_text)?;
    select_backend(module, &backends)
}

pub fn select_backend(
    module: &mut AirModule,
    backends: &[BackendConfig],
) -> Result<usize, SelectBackendError> {
    let mut changed = 0;
    for node in &mut module.nodes {
        if !matches!(
            node.op,
            AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
        ) {
            continue;
        }
        let route = resolve_node_route(node, backends)?;
        let Some(route) = route else {
            continue;
        };
        if attr_string(&node.attributes, graph_attrs::BACKEND).as_deref() != Some(&route.backend) {
            node.attributes.insert(
                graph_attrs::BACKEND.to_string(),
                Value::String(route.backend.clone()),
            );
            changed += 1;
        }
        match route.model {
            Some(model)
                if attr_string(&node.attributes, graph_attrs::MODEL).as_deref() != Some(&model) =>
            {
                node.attributes
                    .insert(graph_attrs::MODEL.to_string(), Value::String(model));
                changed += 1;
            }
            Some(_) | None => {}
        }
    }
    Ok(changed)
}

struct ResolvedRoute {
    backend: String,
    model: Option<String>,
}

fn resolve_node_route(
    node: &crate::air_builder::AirNode,
    backends: &[BackendConfig],
) -> Result<Option<ResolvedRoute>, SelectBackendError> {
    let backend = attr_string(&node.attributes, graph_attrs::BACKEND);
    let model = attr_string(&node.attributes, graph_attrs::MODEL);
    if backend.is_none() && model.is_none() {
        return Ok(None);
    }

    let protocol = attr_string(&node.attributes, graph_attrs::PROVIDER)
        .and_then(|provider| provider.parse::<ProviderProtocol>().ok());
    let candidates = filter_backends(node, backends, backend.as_deref(), protocol)?;

    if let Some(model) = model {
        let matches = candidates
            .iter()
            .filter_map(|backend| model_for(backend, &model).map(|model| (*backend, model)))
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [] => Err(SelectBackendError::UnknownModel {
                node: node.name.clone(),
                model,
                routes: format_routes(&candidates),
            }),
            [(backend, model_cfg)] => Ok(Some(ResolvedRoute {
                backend: backend.name.clone(),
                model: Some(model_cfg.id.clone()),
            })),
            _ => Err(SelectBackendError::AmbiguousModel {
                node: node.name.clone(),
                model,
                routes: format_routes(
                    &matches
                        .iter()
                        .map(|(backend, _)| *backend)
                        .collect::<Vec<_>>(),
                ),
            }),
        };
    }

    if backend.is_some() {
        let selected = candidates
            .first()
            .expect("filter_backends returns non-empty for explicit backend");
        return Ok(Some(ResolvedRoute {
            backend: selected.name.clone(),
            model: implicit_model(node, selected, true)?,
        }));
    }

    if candidates.len() == 1 {
        let selected = candidates[0];
        return Ok(Some(ResolvedRoute {
            backend: selected.name.clone(),
            model: implicit_model(node, selected, false)?,
        }));
    }

    Ok(None)
}

fn filter_backends<'a>(
    node: &crate::air_builder::AirNode,
    backends: &'a [BackendConfig],
    backend: Option<&str>,
    protocol: Option<ProviderProtocol>,
) -> Result<Vec<&'a BackendConfig>, SelectBackendError> {
    let mut filtered = backends.iter().collect::<Vec<_>>();
    if let Some(protocol) = protocol {
        filtered.retain(|item| item.protocol == protocol);
        if filtered.is_empty() {
            return Err(SelectBackendError::UnknownProtocol {
                node: node.name.clone(),
                protocol: protocol.to_string(),
                registered: format_backend_names(backends),
            });
        }
    }
    if let Some(name) = backend {
        filtered.retain(|item| item.name == name);
        if filtered.is_empty() {
            return Err(SelectBackendError::UnknownBackend {
                node: node.name.clone(),
                backend: name.to_string(),
                registered: format_backend_names(backends),
            });
        }
    }
    Ok(filtered)
}

fn implicit_model(
    node: &crate::air_builder::AirNode,
    backend: &BackendConfig,
    allow_defer: bool,
) -> Result<Option<String>, SelectBackendError> {
    match backend.models.as_slice() {
        [model] => Ok(Some(model.id.clone())),
        [] if backend.protocol == ProviderProtocol::Mock => Ok(None),
        [] if allow_defer && RUNTIME_RESOLVES_MODEL.contains(&backend.protocol) => Ok(None),
        [] => Err(SelectBackendError::NoModels {
            node: node.name.clone(),
            backend: backend.name.clone(),
        }),
        _ if allow_defer && RUNTIME_RESOLVES_MODEL.contains(&backend.protocol) => Ok(None),
        _ => Err(SelectBackendError::AmbiguousImplicitModel {
            node: node.name.clone(),
            backend: backend.name.clone(),
            model_attr: graph_attrs::MODEL,
        }),
    }
}

fn model_for<'a>(backend: &'a BackendConfig, value: &str) -> Option<&'a ModelConfig> {
    backend
        .models
        .iter()
        .find(|model| model.id == value || model.aliases.iter().any(|alias| alias == value))
}

fn attr_string(attrs: &std::collections::HashMap<String, Value>, key: &str) -> Option<String> {
    attrs.get(key).and_then(Value::as_str).map(str::to_string)
}

fn format_backend_names(backends: &[BackendConfig]) -> String {
    if backends.is_empty() {
        return "<none>".to_string();
    }
    let mut names = backends
        .iter()
        .map(|backend| backend.name.as_str())
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.join(", ")
}

fn format_routes(backends: &[&BackendConfig]) -> String {
    let mut routes = Vec::new();
    for backend in backends {
        if backend.models.is_empty() {
            routes.push(format!("{}:<none>", backend.name));
            continue;
        }
        routes.extend(
            backend
                .models
                .iter()
                .map(|model| format!("{}:{}", backend.name, model.id)),
        );
    }
    if routes.is_empty() {
        "<none>".to_string()
    } else {
        routes.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::air_builder::{AirModule, AirNode};
    use apxm_backends::llm::BackendType;
    use std::collections::HashMap;

    fn backend(
        name: &str,
        protocol: ProviderProtocol,
        models: &[(&str, &[&str])],
    ) -> BackendConfig {
        BackendConfig {
            name: name.to_string(),
            backend_type: BackendType::Cloud,
            protocol,
            endpoint: None,
            api_key: None,
            headers: HashMap::new(),
            models: models
                .iter()
                .map(|(id, aliases)| ModelConfig {
                    id: (*id).to_string(),
                    aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
                    context_window: 0,
                    supports_vision: false,
                    supports_functions: false,
                    supports_thinking: false,
                    supports_custom_temperature: None,
                    supports_structured_outputs: None,
                    max_output_tokens: None,
                })
                .collect(),
            auto_tool_choice: None,
            supports_structured_outputs: None,
        }
    }

    fn module(attrs: HashMap<String, Value>) -> AirModule {
        AirModule {
            name: "wf".to_string(),
            nodes: vec![AirNode {
                id: 1,
                name: "ask".to_string(),
                op: AISOperationType::Ask,
                attributes: attrs,
            }],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn resolves_backend_and_implicit_single_model() {
        let mut module = module(HashMap::from([(
            graph_attrs::BACKEND.to_string(),
            Value::String("primary".to_string()),
        )]));
        let changed = select_backend(
            &mut module,
            &[backend(
                "primary",
                ProviderProtocol::Anthropic,
                &[("claude-sonnet-4-6", &[])],
            )],
        )
        .expect("route resolves");

        assert_eq!(changed, 1);
        assert_eq!(
            attr_string(&module.nodes[0].attributes, graph_attrs::MODEL).as_deref(),
            Some("claude-sonnet-4-6")
        );
    }

    #[test]
    fn resolves_model_alias_to_canonical_backend_model() {
        let mut module = module(HashMap::from([(
            graph_attrs::MODEL.to_string(),
            Value::String("fast".to_string()),
        )]));
        select_backend(
            &mut module,
            &[backend(
                "primary",
                ProviderProtocol::OpenAI,
                &[("gpt-5-mini", &["fast"])],
            )],
        )
        .expect("alias resolves");

        assert_eq!(
            attr_string(&module.nodes[0].attributes, graph_attrs::BACKEND).as_deref(),
            Some("primary")
        );
        assert_eq!(
            attr_string(&module.nodes[0].attributes, graph_attrs::MODEL).as_deref(),
            Some("gpt-5-mini")
        );
    }

    #[test]
    fn rejects_ambiguous_backend_without_model() {
        let mut module = module(HashMap::from([(
            graph_attrs::BACKEND.to_string(),
            Value::String("vllm".to_string()),
        )]));
        let err = select_backend(
            &mut module,
            &[backend(
                "vllm",
                ProviderProtocol::Vllm,
                &[("a", &[]), ("b", &[])],
            )],
        )
        .expect_err("vllm cannot defer model choice");

        assert!(matches!(
            err,
            SelectBackendError::AmbiguousImplicitModel { .. }
        ));
    }
}
