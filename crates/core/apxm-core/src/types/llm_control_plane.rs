use crate::types::{
    ModelInfo, ProviderProtocol, normalize_endpoint_for_protocol, resolve_builtin_provider,
};
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const APXM_CONFIG_ENV_VAR: &str = "APXM_CONFIG";
pub const APXM_USE_LLM_ENV_VAR: &str = "APXM_USE_LLM";
pub const APXM_LLM_BACKEND_ENV_VAR: &str = "APXM_LLM_BACKEND";
pub const APXM_MODEL_ENV_VAR: &str = "APXM_MODEL";

const APXM_DIR_NAME: &str = ".apxm";
const APXM_CONFIG_FILE_NAME: &str = "config.toml";
const ENV_VALUE_PREFIX: &str = "env:";

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ApxmLlmConfigFile {
    pub chat: ApxmLlmChatConfig,
    pub llm_backends: Vec<ApxmLlmBackendConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ApxmLlmChatConfig {
    pub providers: Vec<String>,
    pub default_backend: Option<String>,
    pub default_model: Option<String>,
    pub planning_model: Option<String>,
    pub routing: ApxmLlmRoutingConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ApxmLlmRoutingConfig {
    pub operation_routes: HashMap<String, ApxmOperationRouteConfig>,
    pub model_aliases: HashMap<String, ApxmModelAliasConfig>,
    pub fallback_chains: Vec<ApxmBackendFallbackConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ApxmOperationRouteConfig {
    pub backend: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApxmModelAliasConfig {
    pub model: String,
    pub backend: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ApxmBackendFallbackConfig {
    pub backend: String,
    pub fallbacks: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ApxmLlmBackendConfig {
    pub name: String,
    pub provider: Option<String>,
    pub protocol: Option<ProviderProtocol>,
    #[serde(default, alias = "model")]
    pub default_model: Option<String>,
    #[serde(default)]
    pub models: Vec<ApxmRegisteredModelConfig>,
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
    #[serde(default)]
    pub options: HashMap<String, String>,
    #[serde(default)]
    pub extra_headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ApxmRegisteredModelConfig {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<ModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResolvedApxmModelConfig {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<ModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedApxmBackendConfig {
    pub name: String,
    pub provider_id: String,
    pub protocol: ProviderProtocol,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    #[serde(default)]
    pub models: Vec<ResolvedApxmModelConfig>,
    pub endpoint: Option<String>,
    #[serde(default)]
    pub options: HashMap<String, String>,
    #[serde(default)]
    pub extra_headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedApxmModelAlias {
    pub model: String,
    pub backend: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApxmLlmControlPlane {
    pub config_path: Option<PathBuf>,
    #[serde(default)]
    pub backends: HashMap<String, ResolvedApxmBackendConfig>,
    pub default_backend: Option<String>,
    pub default_model: Option<String>,
    #[serde(default)]
    pub model_backends: HashMap<String, String>,
    #[serde(default)]
    pub model_aliases: HashMap<String, ResolvedApxmModelAlias>,
    #[serde(default)]
    pub fallback_chains: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub operation_routes: HashMap<String, ApxmOperationRouteConfig>,
}

impl ApxmLlmControlPlane {
    pub fn should_enable(cwd: &Path) -> bool {
        env_flag(APXM_USE_LLM_ENV_VAR)
            || env::var(APXM_CONFIG_ENV_VAR)
                .ok()
                .is_some_and(|value| !value.trim().is_empty())
            || Self::has_registration(cwd)
    }

    pub fn has_registration(cwd: &Path) -> bool {
        resolve_config_path(cwd).is_some_and(|path| path.exists())
    }

    pub fn load_scoped(cwd: &Path) -> io::Result<Self> {
        let config_path = resolve_config_path(cwd);

        let config = match config_path.as_ref() {
            Some(path) if path.exists() => Some(load_toml_file::<ApxmLlmConfigFile>(path)?),
            _ => None,
        };

        let chat = config
            .as_ref()
            .map(|cfg| cfg.chat.clone())
            .unwrap_or_default();
        let allowed_backends = (!chat.providers.is_empty())
            .then(|| chat.providers.iter().cloned().collect::<HashSet<String>>());

        let mut backends = HashMap::new();
        let mut model_backends = HashMap::new();
        let mut model_aliases = HashMap::new();
        let mut ordered_backend_names = Vec::new();

        let backend_configs = config
            .as_ref()
            .map(|cfg| cfg.llm_backends.clone())
            .unwrap_or_default();

        for backend in backend_configs {
            if let Some(allowed) = &allowed_backends
                && !allowed.contains(&backend.name)
            {
                continue;
            }

            let resolved = resolve_backend(&backend)?;
            register_backend_models(&resolved, &mut model_backends, &mut model_aliases);
            ordered_backend_names.push(resolved.name.clone());
            backends.insert(resolved.name.clone(), resolved);
        }

        if let Some(config) = config.as_ref() {
            for (alias, target) in &config.chat.routing.model_aliases {
                model_aliases.insert(
                    alias.clone(),
                    ResolvedApxmModelAlias {
                        model: target.model.clone(),
                        backend: target.backend.clone(),
                    },
                );
            }
        }

        let fallback_chains = chat
            .routing
            .fallback_chains
            .iter()
            .map(|chain| (chain.backend.clone(), chain.fallbacks.clone()))
            .collect::<HashMap<_, _>>();

        let default_backend = env::var(APXM_LLM_BACKEND_ENV_VAR)
            .ok()
            .filter(|backend| backends.contains_key(backend))
            .or_else(|| {
                chat.default_backend
                    .clone()
                    .filter(|backend| backends.contains_key(backend))
            })
            .or_else(|| ordered_backend_names.into_iter().next());

        let default_model = env::var(APXM_MODEL_ENV_VAR)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or(chat.default_model.clone())
            .or_else(|| {
                backends
                    .values()
                    .find_map(|backend| backend.default_model.clone())
            })
            .or_else(|| {
                backends
                    .values()
                    .flat_map(|backend| backend.models.iter())
                    .map(|model| model.id.clone())
                    .next()
            });

        Ok(Self {
            config_path,
            backends,
            default_backend,
            default_model,
            model_backends,
            model_aliases,
            fallback_chains,
            operation_routes: chat.routing.operation_routes,
        })
    }

    pub fn resolve_model_selection(&self, model: Option<&str>) -> (Option<String>, Option<String>) {
        let Some(model) = model else {
            return (None, None);
        };

        if let Some(target) = self.model_aliases.get(model) {
            let backend = target
                .backend
                .clone()
                .or_else(|| self.model_backends.get(&target.model).cloned());
            return (Some(target.model.clone()), backend);
        }

        let backend = self.model_backends.get(model).cloned();
        (Some(model.to_string()), backend)
    }
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn resolve_config_path(cwd: &Path) -> Option<PathBuf> {
    if let Ok(path) = env::var(APXM_CONFIG_ENV_VAR) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    find_project_config_path(cwd).or_else(default_config_path)
}

fn find_project_config_path(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        let candidate = dir.join(APXM_DIR_NAME).join(APXM_CONFIG_FILE_NAME);
        if candidate.exists() {
            return Some(candidate);
        }
        current = dir.parent();
    }
    None
}

fn default_config_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join(APXM_DIR_NAME).join(APXM_CONFIG_FILE_NAME))
}

fn load_toml_file<T>(path: &Path) -> io::Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let contents = fs::read_to_string(path)?;
    toml::from_str(&contents).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse {}: {error}", path.display()),
        )
    })
}

fn resolve_backend(config: &ApxmLlmBackendConfig) -> io::Result<ResolvedApxmBackendConfig> {
    let provider_id = config
        .provider
        .as_deref()
        .unwrap_or(&config.name)
        .trim()
        .to_ascii_lowercase();

    let protocol = resolve_backend_protocol(config, &provider_id)?;
    let api_key = resolve_api_key(config, protocol, &provider_id)?;
    let default_model = config
        .default_model
        .as_deref()
        .map(|value| resolve_env_value(value, "default_model", &config.name))
        .transpose()?;
    let endpoint = config
        .endpoint
        .as_deref()
        .map(|value| resolve_env_value(value, "endpoint", &config.name))
        .transpose()?
        .or_else(|| {
            resolve_builtin_provider(&provider_id)
                .and_then(|spec| spec.default_base_url.map(ToString::to_string))
        })
        .map(|value| normalize_endpoint_for_protocol(protocol, &value));
    let options = resolve_string_map(&config.options, "options", &config.name)?;
    let extra_headers = resolve_string_map(&config.extra_headers, "extra_headers", &config.name)?;
    let models = config
        .models
        .iter()
        .map(|model| {
            let id = resolve_env_value(&model.id, "models.id", &config.name)?;
            let aliases = model
                .aliases
                .iter()
                .map(|alias| resolve_env_value(alias, "models.aliases", &config.name))
                .collect::<io::Result<Vec<_>>>()?;
            Ok(ResolvedApxmModelConfig {
                id,
                aliases,
                info: model.info.clone(),
            })
        })
        .collect::<io::Result<Vec<_>>>()?;

    Ok(ResolvedApxmBackendConfig {
        name: config.name.clone(),
        provider_id,
        protocol,
        api_key,
        default_model,
        models,
        endpoint,
        options,
        extra_headers,
    })
}

fn resolve_backend_protocol(
    config: &ApxmLlmBackendConfig,
    provider_id: &str,
) -> io::Result<ProviderProtocol> {
    if let Some(protocol) = config.protocol {
        return Ok(protocol);
    }

    if let Ok(protocol) = provider_id.parse::<ProviderProtocol>() {
        return Ok(protocol);
    }

    resolve_builtin_provider(provider_id)
        .map(|spec| spec.protocol)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Unknown APXM provider '{}' for backend '{}'",
                    provider_id, config.name
                ),
            )
        })
}

fn resolve_api_key(
    config: &ApxmLlmBackendConfig,
    protocol: ProviderProtocol,
    provider_id: &str,
) -> io::Result<Option<String>> {
    if let Some(api_key) = config.api_key.as_deref() {
        let resolved = resolve_env_value(api_key, "api_key", &config.name)?;
        if !resolved.is_empty() {
            return Ok(Some(resolved));
        }
    }

    if let Some(spec) = resolve_builtin_provider(provider_id)
        && let Some(env_key) = spec.api_key_env_var
        && let Ok(value) = env::var(env_key)
        && !value.trim().is_empty()
    {
        return Ok(Some(value));
    }

    if matches!(protocol, ProviderProtocol::Ollama) {
        return Ok(Some(String::new()));
    }

    Ok(None)
}

fn resolve_env_value(value: &str, field: &str, backend: &str) -> io::Result<String> {
    if let Some(env_name) = value.strip_prefix(ENV_VALUE_PREFIX) {
        return env::var(env_name).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Environment variable '{}' not set for field '{}' in backend '{}'",
                    env_name, field, backend
                ),
            )
        });
    }

    Ok(value.to_string())
}

fn resolve_string_map(
    values: &HashMap<String, String>,
    prefix: &str,
    backend: &str,
) -> io::Result<HashMap<String, String>> {
    values
        .iter()
        .map(|(key, value)| {
            resolve_env_value(value, &format!("{prefix}.{key}"), backend)
                .map(|resolved| (key.clone(), resolved))
        })
        .collect()
}

fn register_backend_models(
    backend: &ResolvedApxmBackendConfig,
    model_backends: &mut HashMap<String, String>,
    model_aliases: &mut HashMap<String, ResolvedApxmModelAlias>,
) {
    if let Some(model) = &backend.default_model {
        model_backends.insert(model.clone(), backend.name.clone());
    }

    for model in &backend.models {
        model_backends.insert(model.id.clone(), backend.name.clone());
        for alias in &model.aliases {
            model_aliases.insert(
                alias.clone(),
                ResolvedApxmModelAlias {
                    model: model.id.clone(),
                    backend: Some(backend.name.clone()),
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_selection_uses_alias_backend() {
        let mut control_plane = ApxmLlmControlPlane::default();
        control_plane
            .model_backends
            .insert("gpt-5".into(), "openai".into());
        control_plane.model_aliases.insert(
            "fast".into(),
            ResolvedApxmModelAlias {
                model: "gpt-5".into(),
                backend: Some("openrouter".into()),
            },
        );

        let (model, backend) = control_plane.resolve_model_selection(Some("fast"));
        assert_eq!(model.as_deref(), Some("gpt-5"));
        assert_eq!(backend.as_deref(), Some("openrouter"));
    }

    #[test]
    fn resolve_backend_protocol_uses_builtin_aliases() {
        let backend = ApxmLlmBackendConfig {
            name: "gemini".into(),
            provider: Some("gemini".into()),
            ..Default::default()
        };

        let protocol = resolve_backend_protocol(&backend, "gemini").unwrap();
        assert_eq!(protocol, ProviderProtocol::Google);
    }
}
