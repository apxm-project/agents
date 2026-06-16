//! Compiler-owned optimization context.
//!
//! Runtime and driver configuration stay backend/resource oriented. This module
//! reads only compiler optimization inputs and turns them into transient pass
//! attributes for MLIR transforms.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use apxm_core::constants::env as apxm_env;
use apxm_core::constants::{cache as cache_constants, dspy as dspy_constants};
use apxm_core::error::compiler::{CompilerError, Result};
use apxm_core::error::{Error, ErrorCode};
use apxm_core::paths::ApxmPaths;
use apxm_core::types::PipelineConfig;
use serde::{Deserialize, Serialize};

mod toml_keys {
    pub const COMPILER: &str = "compiler";
    pub const OPTIMIZATION: &str = "optimization";
    pub const PROMPT_TUNING: &str = "prompt_tuning";
    pub const ENABLED: &str = "enabled";
    pub const TRAINING_DATA: &str = "training_data";
    pub const DATASET: &str = "dataset";
    pub const OPTIMIZER: &str = "optimizer";
    pub const AUTO: &str = "auto";
    pub const METRIC: &str = "metric";
    pub const NO_CACHE: &str = "no_cache";
    pub const BACKEND: &str = "backend";
    pub const MODEL: &str = "model";
    pub const PROTOCOL: &str = "protocol";
    pub const ENDPOINT: &str = "endpoint";
    pub const API_KEY: &str = "api_key";
    pub const HEADERS: &str = "headers";
}

/// Prompt optimizer implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptOptimizer {
    Mipro,
    Bootstrap,
    Copro,
}

impl Default for PromptOptimizer {
    fn default() -> Self {
        Self::Bootstrap
    }
}

impl PromptOptimizer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mipro => "mipro",
            Self::Bootstrap => "bootstrap",
            Self::Copro => "copro",
        }
    }
}

/// Prompt optimization budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptOptimizationBudget {
    Light,
    Medium,
    Heavy,
}

impl Default for PromptOptimizationBudget {
    fn default() -> Self {
        Self::Light
    }
}

impl PromptOptimizationBudget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Medium => "medium",
            Self::Heavy => "heavy",
        }
    }
}

/// Prompt optimization metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMetric {
    TokenOverlap,
    ExactMatch,
    Contains,
    LlmJudge,
}

impl Default for PromptMetric {
    fn default() -> Self {
        Self::TokenOverlap
    }
}

impl PromptMetric {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TokenOverlap => "token_overlap",
            Self::ExactMatch => "exact_match",
            Self::Contains => "contains",
            Self::LlmJudge => "llm_judge",
        }
    }
}

/// Generic compiler optimization context.
#[derive(Debug, Clone, Default)]
pub struct CompilerOptimizationContext {
    compiler_config_path: Option<PathBuf>,
}

impl CompilerOptimizationContext {
    pub fn from_pipeline_config(config: &PipelineConfig) -> Self {
        Self {
            compiler_config_path: config.compiler_config_path.clone(),
        }
    }

    /// Return whether compiler prompt tuning is explicitly configured.
    ///
    /// This check is intentionally side-effect free. It lets the pipeline
    /// decide whether the DSPy pass should be injected into the default
    /// O-level sequence without normalizing training data or touching the
    /// compiler cache unless that pass will actually run.
    pub fn prompt_optimization_configured(&self) -> Result<bool> {
        let Some((_config_path, root)) = self.load_config_root()? else {
            return Ok(false);
        };
        let Some(prompt_table) = prompt_tuning_table(&root) else {
            return Ok(false);
        };
        if !bool_field(prompt_table, toml_keys::ENABLED).unwrap_or(true) {
            return Ok(false);
        }
        Ok(string_field(prompt_table, toml_keys::TRAINING_DATA)
            .or_else(|| string_field(prompt_table, dspy_constants::TRAINING_DATA))
            .or_else(|| string_field(prompt_table, toml_keys::DATASET))
            .is_some())
    }

    pub fn prompt_optimization(&self) -> Result<Option<PromptOptimizationRequest>> {
        let Some((config_path, root)) = self.load_config_root()? else {
            return Ok(None);
        };
        let Some(prompt_table) = prompt_tuning_table(&root) else {
            return Ok(None);
        };
        if !bool_field(prompt_table, toml_keys::ENABLED).unwrap_or(true) {
            return Ok(None);
        }

        let training_path = training_data_path(prompt_table, &config_path)?;
        let Some(training_path) = training_path else {
            return Ok(None);
        };
        let cache_dir = compiler_cache_dir()?;
        let normalized_training = normalize_training_data(&training_path, &cache_dir)?;
        let backend_json = resolve_backend_json(prompt_table)?;

        Ok(Some(PromptOptimizationRequest {
            training_data_path: normalized_training,
            cache_dir: cache_dir.join(cache_constants::DSPY_DIR),
            optimizer: enum_field(prompt_table, toml_keys::OPTIMIZER)?,
            budget: enum_field(prompt_table, toml_keys::AUTO)?,
            metric: enum_field(prompt_table, toml_keys::METRIC)?,
            no_cache: bool_field(prompt_table, toml_keys::NO_CACHE).unwrap_or(false),
            backend_json,
        }))
    }

    fn load_config_root(&self) -> Result<Option<(PathBuf, toml::Value)>> {
        let explicit = self
            .compiler_config_path
            .as_ref()
            .filter(|path| path.is_file())
            .cloned();
        let env_path = if explicit.is_none() {
            std::env::var_os(apxm_env::APXM_CONFIG)
                .map(PathBuf::from)
                .filter(|path| path.is_file())
        } else {
            None
        };
        let project_path = if explicit.is_none() && env_path.is_none() {
            ApxmPaths::discover()
                .ok()
                .map(|paths| paths.project_config_path())
                .filter(|path| path.is_file())
        } else {
            None
        };
        let compiler_path = if explicit.is_none() && env_path.is_none() && project_path.is_none() {
            ApxmPaths::discover()
                .ok()
                .map(|paths| paths.compiler_config_path())
                .filter(|path| path.is_file())
        } else {
            None
        };
        let Some(path) = explicit.or(env_path).or(project_path).or(compiler_path) else {
            return Ok(None);
        };
        let text = fs::read_to_string(&path).map_err(|err| {
            compiler_config_error(format!("Failed to read {}: {err}", path.display()))
        })?;
        let value = parse_config_document(&text).map_err(|err| {
            compiler_config_error(format!("Failed to parse {}: {err}", path.display()))
        })?;
        Ok(Some((path, value)))
    }
}

/// Resolved prompt-optimization request for transient pass attributes.
#[derive(Debug, Clone)]
pub struct PromptOptimizationRequest {
    pub training_data_path: PathBuf,
    pub cache_dir: PathBuf,
    pub optimizer: PromptOptimizer,
    pub budget: PromptOptimizationBudget,
    pub metric: PromptMetric,
    pub no_cache: bool,
    pub backend_json: String,
}

#[derive(Debug, Serialize)]
struct DspyBackendRequest<'a> {
    protocol: &'a str,
    model: &'a str,
    endpoint: Option<&'a str>,
    api_key: Option<&'a str>,
    headers: HashMap<String, String>,
}

fn prompt_tuning_table(root: &toml::Value) -> Option<&toml::value::Table> {
    table_at(
        root,
        &[
            toml_keys::COMPILER,
            toml_keys::OPTIMIZATION,
            toml_keys::PROMPT_TUNING,
        ],
    )
}

fn parse_config_document(text: &str) -> std::result::Result<toml::Value, toml::de::Error> {
    toml::from_str::<toml::value::Table>(text).map(toml::Value::Table)
}

fn table_at<'a>(root: &'a toml::Value, path: &[&str]) -> Option<&'a toml::value::Table> {
    let mut value = root;
    for segment in path {
        value = value.get(*segment)?;
    }
    value.as_table()
}

fn string_field<'a>(table: &'a toml::value::Table, key: &str) -> Option<&'a str> {
    table.get(key).and_then(toml::Value::as_str)
}

fn bool_field(table: &toml::value::Table, key: &str) -> Option<bool> {
    table.get(key).and_then(toml::Value::as_bool)
}

fn enum_field<T>(table: &toml::value::Table, key: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de> + Default,
{
    let Some(value) = table.get(key) else {
        return Ok(T::default());
    };
    value.clone().try_into().map_err(|err| {
        compiler_config_error(format!("Invalid compiler prompt tuning field {key}: {err}"))
    })
}

fn training_data_path(table: &toml::value::Table, config_path: &Path) -> Result<Option<PathBuf>> {
    let Some(path) = string_field(table, toml_keys::TRAINING_DATA)
        .or_else(|| string_field(table, dspy_constants::TRAINING_DATA))
        .or_else(|| string_field(table, toml_keys::DATASET))
    else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    let resolved = if path.is_absolute() {
        path
    } else {
        config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    };
    if !resolved.is_file() {
        return Err(compiler_config_error(format!(
            "Compiler prompt tuning training data not found: {}",
            resolved.display()
        )));
    }
    Ok(Some(resolved))
}

fn compiler_cache_dir() -> Result<PathBuf> {
    let paths = ApxmPaths::discover().map_err(|err| {
        compiler_config_error(format!(
            "Failed to discover APXM paths for compiler cache: {err}"
        ))
    })?;
    paths
        .cache_component_dir(cache_constants::COMPILER_DIR)
        .map_err(|err| compiler_config_error(format!("Failed to create compiler cache: {err}")))
}

fn normalize_training_data(source: &Path, compiler_cache: &Path) -> Result<PathBuf> {
    let bytes = fs::read(source).map_err(|err| {
        compiler_config_error(format!(
            "Failed to read prompt training data {}: {err}",
            source.display()
        ))
    })?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let training_dir = compiler_cache.join(cache_constants::DSPY_TRAINING_DIR);
    fs::create_dir_all(&training_dir).map_err(|err| {
        compiler_config_error(format!(
            "Failed to create prompt training cache {}: {err}",
            training_dir.display()
        ))
    })?;
    let extension = source
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or(apxm_core::constants::extensions::JSON_DATA);
    let destination = training_dir.join(format!("{hash}.{extension}"));
    if !destination.is_file() {
        fs::write(&destination, bytes).map_err(|err| {
            compiler_config_error(format!(
                "Failed to write normalized prompt training data {}: {err}",
                destination.display()
            ))
        })?;
    }
    Ok(destination)
}

fn resolve_backend_json(prompt_table: &toml::value::Table) -> Result<String> {
    let backend_table = prompt_table
        .get(toml_keys::BACKEND)
        .and_then(toml::Value::as_table)
        .ok_or_else(|| {
            compiler_config_error(
                "Compiler prompt tuning requires [compiler.optimization.prompt_tuning.backend]",
            )
        })?;
    let model = string_field(backend_table, toml_keys::MODEL)
        .ok_or_else(|| compiler_config_error("Compiler prompt tuning backend requires a model"))?;
    let protocol = string_field(backend_table, toml_keys::PROTOCOL).ok_or_else(|| {
        compiler_config_error("Compiler prompt tuning backend requires a protocol")
    })?;
    let headers = backend_table
        .get(toml_keys::HEADERS)
        .and_then(toml::Value::as_table)
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    let request = DspyBackendRequest {
        protocol,
        model,
        endpoint: string_field(backend_table, toml_keys::ENDPOINT),
        api_key: string_field(backend_table, toml_keys::API_KEY),
        headers,
    };
    serde_json::to_string(&request).map_err(CompilerError::Json)
}

fn compiler_config_error(message: impl Into<String>) -> CompilerError {
    CompilerError::InvalidInput(Box::new(Error::new_generic(
        ErrorCode::InvalidConfiguration,
        message,
    )))
}
