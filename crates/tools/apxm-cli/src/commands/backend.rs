//! Backend management (cloud / on-prem / local containers).

#[cfg(feature = "driver")]
use std::collections::HashSet;

use anyhow::Result;
use apxm_credentials::docker::{ContainerStatus, DockerManager};
use colored::Colorize;

use super::cli::*;
use super::dekk_hints;
use super::implementations::{Status, print_section_header, print_status_line};

#[cfg(feature = "driver")]
const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";

/// Fetch installed models from a running Ollama instance and register them.
///
/// Returns `(added, skipped)` counts. `existing` model IDs are skipped.
#[cfg(feature = "driver")]
fn ollama_model_caps(base_url: &str, model_name: &str) -> (bool, bool, usize) {
    // Query /api/show for real capabilities — no hardcoding model family names.
    // Returns (supports_functions, supports_vision, context_window).
    let client = reqwest::blocking::Client::new();
    let Ok(resp) = client
        .post(&format!("{base_url}/api/show"))
        .json(&serde_json::json!({"model": model_name}))
        .send()
    else {
        return (false, false, 128000);
    };
    if !resp.status().is_success() {
        return (false, false, 128000);
    }
    let Ok(json) = resp.json::<serde_json::Value>() else {
        return (false, false, 128000);
    };

    // capabilities: ["completion", "tools", "vision", "thinking", ...]
    let caps = json
        .get("capabilities")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    let supports_functions = caps.iter().any(|c| c.as_str() == Some("tools"));
    let supports_vision = caps.iter().any(|c| c.as_str() == Some("vision"));

    // context_window from model_info — key ends with ".context_length"
    let ctx_window = json
        .get("model_info")
        .and_then(|info| info.as_object())
        .and_then(|obj| {
            obj.iter()
                .find(|(k, _)| k.ends_with(".context_length"))
                .and_then(|(_, v)| v.as_u64())
                .map(|n| n as usize)
        })
        .unwrap_or(128000);

    (supports_functions, supports_vision, ctx_window)
}

#[cfg(feature = "driver")]
fn sync_ollama_models(
    store: &apxm_credentials::backend::BackendStore,
    backend_name: &str,
    base_url: &str,
    existing: &std::collections::HashSet<String>,
) -> Result<(usize, usize)> {
    let resp = reqwest::blocking::get(&format!("{base_url}/api/tags")).map_err(|e| {
        anyhow::anyhow!(
            "Cannot reach Ollama at {base_url}: {e}\nMake sure it is running: ollama serve"
        )
    })?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "Ollama API error ({}): {}",
            resp.status(),
            resp.text().unwrap_or_default()
        );
    }

    let body: serde_json::Value = resp
        .json()
        .map_err(|e| anyhow::anyhow!("Failed to parse Ollama response: {e}"))?;

    let models_arr = body
        .get("models")
        .and_then(|m| m.as_array())
        .ok_or_else(|| anyhow::anyhow!("Unexpected Ollama response format"))?;

    let mut added = 0usize;
    let mut skipped = 0usize;

    for m in models_arr {
        if let Some(model_name) = m.get("name").and_then(|n| n.as_str()) {
            if existing.contains(model_name) {
                skipped += 1;
                continue;
            }
            let (supports_functions, supports_vision, ctx_window) =
                ollama_model_caps(base_url, model_name);
            let model = apxm_core::types::ModelConfig {
                id: model_name.to_string(),
                aliases: vec![],
                context_window: ctx_window,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                supports_vision,
                supports_functions,
                supports_thinking: false,
                supports_custom_temperature: None,
                max_output_tokens: None,
                tags: vec![],
            };
            store
                .add_model(backend_name, model)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            added += 1;
        }
    }

    Ok((added, skipped))
}

#[cfg(feature = "driver")]
pub async fn backend_command(action: BackendAction, json_output: bool) -> Result<()> {
    use apxm_core::types::{BackendConfig, BackendType, ProviderProtocol};
    use apxm_credentials::backend::BackendStore;
    use std::str::FromStr;

    let store = BackendStore::open().map_err(|e| anyhow::anyhow!("{e}"))?;

    match action {
        BackendAction::Add {
            name,
            r#type,
            protocol,
            endpoint,
            api_key,
            header,
        } => {
            // Parse protocol first (needed for Ollama smart defaults)
            let protocol =
                ProviderProtocol::from_str(&protocol).map_err(|e| anyhow::anyhow!("{e}"))?;
            let is_ollama = protocol == ProviderProtocol::Ollama;
            let api_key_not_required = matches!(
                protocol,
                ProviderProtocol::Ollama | ProviderProtocol::Vllm | ProviderProtocol::Mock
            );

            // Ollama smart defaults: type=local if not specified
            let backend_type = if r#type.is_empty() && is_ollama {
                BackendType::Local
            } else if r#type.is_empty() {
                anyhow::bail!("--type is required (cloud, onprem, or local)");
            } else {
                BackendType::from_str(&r#type).map_err(|e| anyhow::anyhow!("{e}"))?
            };

            // Ollama smart defaults: endpoint
            let endpoint = if endpoint.is_none() && is_ollama {
                Some(DEFAULT_OLLAMA_ENDPOINT.to_string())
            } else {
                endpoint
            };

            // API key not needed for local backends or protocols that support unauthenticated local/on-prem serving.
            let api_key = if api_key.is_some()
                || backend_type == BackendType::Local
                || api_key_not_required
            {
                api_key
            } else {
                eprint!("Enter backend API key for {name} (or press Enter to skip): ");
                let key = rpassword::read_password()
                    .map_err(|e| anyhow::anyhow!("Failed to read API key: {e}"))?;
                if key.is_empty() { None } else { Some(key) }
            };

            let headers: std::collections::HashMap<String, String> = header.into_iter().collect();

            let backend = BackendConfig {
                name: name.clone(),
                backend_type,
                protocol,
                endpoint: endpoint.clone(),
                api_key,
                headers,
                models: vec![],
                docker: None,
                auto_tool_choice: None,
            };

            store.add(backend).map_err(|e| anyhow::anyhow!("{e}"))?;

            // For Ollama: best-effort auto-sync of installed models
            let synced_count = if is_ollama {
                let base = endpoint.as_deref().unwrap_or(DEFAULT_OLLAMA_ENDPOINT);
                let empty = HashSet::new();
                sync_ollama_models(&store, &name, base, &empty)
                    .map(|(added, _)| added)
                    .unwrap_or(0)
            } else {
                0
            };

            if json_output {
                println!(
                    "{{\"status\":\"ok\",\"backend\":\"{name}\",\"synced_models\":{synced_count}}}"
                );
            } else {
                print_section_header("Backend Registered");
                print_status_line("Name", Status::Ok, &name);
                print_status_line("Type", Status::Ok, &format!("{backend_type}"));
                print_status_line("Protocol", Status::Ok, &format!("{protocol}"));
                print_status_line("Store", Status::Ok, &store.path().display().to_string());
                if is_ollama {
                    if synced_count > 0 {
                        print_status_line(
                            "Models synced",
                            Status::Ok,
                            &format!("{synced_count} installed models registered"),
                        );
                    } else {
                        print_status_line(
                            "Models",
                            Status::Warning,
                            &format!(
                                "Ollama not reachable - run `{}` after starting Ollama",
                                dekk_hints::BACKEND_SYNC_MODELS
                            ),
                        );
                    }
                }
            }
        }
        BackendAction::List { format } => {
            let backends = store.list().map_err(|e| anyhow::anyhow!("{e}"))?;

            if json_output || format == "json" {
                println!("{}", serde_json::to_string_pretty(&backends)?);
                return Ok(());
            }

            if backends.is_empty() {
                println!("No backends registered.");
                println!("Add one with: {}", dekk_hints::BACKEND_ADD_OPENAI);
                return Ok(());
            }

            print_section_header("Registered Backends");
            for backend in &backends {
                let key_display = backend
                    .api_key
                    .as_deref()
                    .map(|k| apxm_credentials::mask::mask_key(k))
                    .unwrap_or_else(|| "<none>".to_string());

                println!(
                    "  {:<16} {:<8} {:<10} key={}{}{}",
                    backend.name.bold(),
                    format!("{}", backend.backend_type),
                    format!("{}", backend.protocol),
                    key_display,
                    backend
                        .endpoint
                        .as_ref()
                        .map(|e| format!("  endpoint={e}"))
                        .unwrap_or_default(),
                    if !backend.models.is_empty() {
                        format!("  +{} models", backend.models.len())
                    } else {
                        String::new()
                    }
                );
            }
            println!();
            println!("Store: {}", store.path().display());
        }
        BackendAction::Remove { name } => {
            store.remove(&name).map_err(|e| anyhow::anyhow!("{e}"))?;

            if json_output {
                println!("{{\"status\":\"ok\",\"action\":\"removed\",\"backend\":\"{name}\"}}");
            } else {
                print_section_header("Backend Removed");
                print_status_line(&name, Status::Ok, "removed");
            }
        }
        BackendAction::Test { name } => {
            let backends_to_test: Vec<BackendConfig> = match name {
                Some(ref n) => {
                    let backend = store
                        .get(n)
                        .map_err(|e| anyhow::anyhow!("{e}"))?
                        .ok_or_else(|| anyhow::anyhow!("Backend '{n}' not found"))?;
                    vec![backend]
                }
                None => store.list().map_err(|e| anyhow::anyhow!("{e}"))?,
            };

            if backends_to_test.is_empty() {
                println!("No backends to test.");
                return Ok(());
            }

            print_section_header("Testing Backends");
            let mut all_ok = true;
            for backend in &backends_to_test {
                match apxm_credentials::validate::validate_backend(backend).await {
                    Ok(msg) => print_status_line(&backend.name, Status::Ok, &msg),
                    Err(e) => {
                        print_status_line(&backend.name, Status::Error, &e.to_string());
                        all_ok = false;
                    }
                }
            }
            if !all_ok {
                return Err(anyhow::anyhow!("Some backends failed validation"));
            }
        }
        BackendAction::Start { name } => {
            let backend = store
                .get(&name)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .ok_or_else(|| anyhow::anyhow!("Backend '{name}' not found"))?;

            let container_id = DockerManager::start(&backend)
                .map_err(|e| anyhow::anyhow!("Failed to start backend: {e}"))?;

            if json_output {
                println!(
                    "{{\"status\":\"ok\",\"backend\":\"{name}\",\"container_id\":\"{container_id}\"}}"
                );
            } else {
                print_section_header("Backend Started");
                print_status_line("Backend", Status::Ok, &name);
                print_status_line("Container ID", Status::Ok, &container_id);
            }
        }
        BackendAction::Stop { name } => {
            DockerManager::stop_by_name(&name)
                .map_err(|e| anyhow::anyhow!("Failed to stop backend: {e}"))?;

            if json_output {
                println!("{{\"status\":\"ok\",\"backend\":\"{name}\",\"action\":\"stopped\"}}");
            } else {
                print_section_header("Backend Stopped");
                print_status_line("Backend", Status::Ok, &name);
            }
        }
        BackendAction::Status { name } => {
            let backends_to_check: Vec<BackendConfig> = match name {
                Some(ref n) => {
                    let backend = store
                        .get(n)
                        .map_err(|e| anyhow::anyhow!("{e}"))?
                        .ok_or_else(|| anyhow::anyhow!("Backend '{n}' not found"))?;
                    vec![backend]
                }
                None => store.list().map_err(|e| anyhow::anyhow!("{e}"))?,
            };

            if json_output {
                let mut statuses = Vec::new();
                for backend in &backends_to_check {
                    if backend.backend_type == BackendType::Local {
                        let status = DockerManager::status(&backend.name)
                            .unwrap_or(ContainerStatus::NotFound);
                        statuses.push(serde_json::json!({
                            "backend": backend.name,
                            "status": status.to_string()
                        }));
                    }
                }
                println!("{}", serde_json::to_string_pretty(&statuses)?);
            } else {
                print_section_header("Backend Status");
                for backend in &backends_to_check {
                    if backend.backend_type == BackendType::Local {
                        let status = DockerManager::status(&backend.name)
                            .unwrap_or(ContainerStatus::NotFound);
                        let status_display = match status {
                            ContainerStatus::Running => Status::Ok,
                            ContainerStatus::Stopped => Status::Warning,
                            ContainerStatus::NotFound => Status::Error,
                        };
                        print_status_line(&backend.name, status_display, &status.to_string());
                    }
                }
            }
        }
        BackendAction::Logs { name, tail } => {
            let container_id = DockerManager::get_container_id(&name)
                .map_err(|e| anyhow::anyhow!("Failed to get container ID: {e}"))?
                .ok_or_else(|| anyhow::anyhow!("Container not found for backend '{name}'"))?;

            let logs = DockerManager::logs(&container_id, tail)
                .map_err(|e| anyhow::anyhow!("Failed to get logs: {e}"))?;

            if json_output {
                println!(
                    "{{\"status\":\"ok\",\"backend\":\"{name}\",\"logs\":{}}}",
                    serde_json::to_string(&logs)?
                );
            } else {
                println!("{}", logs);
            }
        }
        BackendAction::Restart { name } => {
            let container_id = DockerManager::get_container_id(&name)
                .map_err(|e| anyhow::anyhow!("Failed to get container ID: {e}"))?
                .ok_or_else(|| anyhow::anyhow!("Container not found for backend '{name}'"))?;

            DockerManager::restart(&container_id)
                .map_err(|e| anyhow::anyhow!("Failed to restart backend: {e}"))?;

            if json_output {
                println!("{{\"status\":\"ok\",\"backend\":\"{name}\",\"action\":\"restarted\"}}");
            } else {
                print_section_header("Backend Restarted");
                print_status_line("Backend", Status::Ok, &name);
            }
        }
        BackendAction::SyncModels { name, endpoint } => {
            let backend_cfg = store
                .get(&name)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .ok_or_else(|| anyhow::anyhow!("Backend '{}' not found", name))?;

            if backend_cfg.protocol != ProviderProtocol::Ollama {
                anyhow::bail!(
                    "sync-models only works for Ollama backends. '{}' uses protocol '{}'.",
                    name,
                    backend_cfg.protocol
                );
            }

            let base = endpoint
                .or_else(|| backend_cfg.endpoint.clone())
                .unwrap_or_else(|| DEFAULT_OLLAMA_ENDPOINT.to_string());

            let existing: HashSet<String> =
                backend_cfg.models.iter().map(|m| m.id.clone()).collect();

            let (added, skipped) = sync_ollama_models(&store, &name, &base, &existing)?;

            if json_output {
                println!(
                    "{{\"status\":\"ok\",\"backend\":\"{name}\",\"added\":{added},\"skipped\":{skipped}}}"
                );
            } else {
                print_section_header("Ollama Models Synced");
                print_status_line("Backend", Status::Ok, &name);
                print_status_line("Endpoint", Status::Ok, &base);
                if added > 0 {
                    print_status_line("Added", Status::Ok, &format!("{added} new models"));
                }
                if skipped > 0 {
                    print_status_line(
                        "Skipped",
                        Status::Warning,
                        &format!("{skipped} already registered"),
                    );
                }
                if added == 0 && skipped == 0 {
                    println!("  No models found. Install one with: ollama pull llama3.3");
                }
            }
        }
        BackendAction::AddModel {
            backend,
            model_id,
            alias,
            context_window,
            cost_input,
            cost_output,
            supports_vision,
            supports_functions,
            supports_thinking,
            tag,
        } => {
            use apxm_core::types::ModelConfig;

            let model = ModelConfig {
                id: model_id.clone(),
                aliases: alias,
                context_window,
                cost_per_1k_input: cost_input,
                cost_per_1k_output: cost_output,
                supports_vision,
                supports_functions,
                supports_thinking,
                supports_custom_temperature: None,
                max_output_tokens: None,
                tags: tag,
            };

            store
                .add_model(&backend, model)
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            if json_output {
                println!(
                    "{{\"status\":\"ok\",\"backend\":\"{backend}\",\"model\":\"{model_id}\"}}"
                );
            } else {
                print_section_header("Model Added");
                print_status_line("Backend", Status::Ok, &backend);
                print_status_line("Model", Status::Ok, &model_id);
            }
        }
    }

    Ok(())
}
