//! Docker lifecycle management for local backends.
//!
//! This module provides functionality to start, stop, and monitor Docker
//! containers for local inference backends (vLLM, Ollama, etc.).

use apxm_core::types::{BackendConfig, BackendType};
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DockerError {
    #[error("Docker command failed: {0}")]
    CommandFailed(String),

    #[error("Backend '{0}' is not a local backend")]
    NotLocalBackend(String),

    #[error("Backend '{0}' has no Docker configuration")]
    NoDockerConfig(String),

    #[error("Container not found: {0}")]
    ContainerNotFound(String),

    #[error("Docker is not installed or not running")]
    DockerNotAvailable,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerStatus {
    Running,
    Stopped,
    NotFound,
}

impl std::fmt::Display for ContainerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerStatus::Running => write!(f, "running"),
            ContainerStatus::Stopped => write!(f, "stopped"),
            ContainerStatus::NotFound => write!(f, "not found"),
        }
    }
}

/// Docker container manager for local backends.
pub struct DockerManager;

impl DockerManager {
    /// Check if Docker is available on the system.
    pub fn is_available() -> bool {
        Command::new("docker")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// Start a local backend container.
    ///
    /// Returns the container ID on success.
    pub fn start(backend: &BackendConfig) -> Result<String, DockerError> {
        if backend.backend_type != BackendType::Local {
            return Err(DockerError::NotLocalBackend(backend.name.clone()));
        }

        let docker_config = backend
            .docker
            .as_ref()
            .ok_or_else(|| DockerError::NoDockerConfig(backend.name.clone()))?;

        if !Self::is_available() {
            return Err(DockerError::DockerNotAvailable);
        }

        // Check if container is already running
        if let Ok(status) = Self::status(&backend.name) {
            if status == ContainerStatus::Running {
                // Get container ID
                let output = Command::new("docker")
                    .args(["ps", "-q", "-f", &format!("name={}", backend.name)])
                    .output()?;
                if output.status.success() {
                    let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !container_id.is_empty() {
                        return Ok(container_id);
                    }
                }
            }
        }

        // Build docker run command
        let mut cmd = Command::new("docker");
        cmd.arg("run")
            .arg("-d") // Detached mode
            .arg("--name")
            .arg(&backend.name);

        // Add environment variables
        for (key, value) in &docker_config.env {
            cmd.arg("-e").arg(format!("{key}={value}"));
        }

        // Mount model path if specified
        if let Some(model_path) = &docker_config.model_path {
            cmd.arg("-v")
                .arg(format!("{model_path}:/models:ro")); // Read-only mount
        }

        // Add GPU support if tensor_parallel is specified
        if let Some(tensor_parallel) = docker_config.tensor_parallel {
            if tensor_parallel > 0 {
                cmd.arg("--gpus")
                    .arg(format!("all,capabilities=compute,utility"));
            }
        }

        // Add the image
        cmd.arg(&docker_config.image);

        // Add custom command or args
        if !docker_config.command.is_empty() {
            for arg in &docker_config.command {
                cmd.arg(arg);
            }
        } else if !docker_config.args.is_empty() {
            for arg in &docker_config.args {
                cmd.arg(arg);
            }
        }

        // Execute the command
        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DockerError::CommandFailed(format!(
                "Failed to start container: {stderr}"
            )));
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(container_id)
    }

    /// Stop a running container.
    pub fn stop(container_id: &str) -> Result<(), DockerError> {
        if !Self::is_available() {
            return Err(DockerError::DockerNotAvailable);
        }

        let output = Command::new("docker")
            .args(["stop", container_id])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DockerError::CommandFailed(format!(
                "Failed to stop container: {stderr}"
            )));
        }

        Ok(())
    }

    /// Stop a container by backend name.
    pub fn stop_by_name(backend_name: &str) -> Result<(), DockerError> {
        if !Self::is_available() {
            return Err(DockerError::DockerNotAvailable);
        }

        let output = Command::new("docker")
            .args(["stop", backend_name])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DockerError::CommandFailed(format!(
                "Failed to stop container: {stderr}"
            )));
        }

        Ok(())
    }

    /// Remove a stopped container.
    pub fn remove(container_id: &str) -> Result<(), DockerError> {
        if !Self::is_available() {
            return Err(DockerError::DockerNotAvailable);
        }

        let output = Command::new("docker")
            .args(["rm", container_id])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DockerError::CommandFailed(format!(
                "Failed to remove container: {stderr}"
            )));
        }

        Ok(())
    }

    /// Get the status of a container by backend name.
    pub fn status(backend_name: &str) -> Result<ContainerStatus, DockerError> {
        if !Self::is_available() {
            return Err(DockerError::DockerNotAvailable);
        }

        // Check if container exists and is running
        let output = Command::new("docker")
            .args(["ps", "-a", "-f", &format!("name=^{backend_name}$"), "--format", "{{.Status}}"])
            .output()?;

        if !output.status.success() {
            return Ok(ContainerStatus::NotFound);
        }

        let status_str = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if status_str.is_empty() {
            Ok(ContainerStatus::NotFound)
        } else if status_str.starts_with("Up") {
            Ok(ContainerStatus::Running)
        } else {
            Ok(ContainerStatus::Stopped)
        }
    }

    /// Get the container ID for a backend.
    pub fn get_container_id(backend_name: &str) -> Result<Option<String>, DockerError> {
        if !Self::is_available() {
            return Err(DockerError::DockerNotAvailable);
        }

        let output = Command::new("docker")
            .args(["ps", "-a", "-q", "-f", &format!("name=^{backend_name}$")])
            .output()?;

        if !output.status.success() {
            return Ok(None);
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if container_id.is_empty() {
            Ok(None)
        } else {
            Ok(Some(container_id))
        }
    }

    /// Get container logs.
    pub fn logs(container_id: &str, tail: usize) -> Result<String, DockerError> {
        if !Self::is_available() {
            return Err(DockerError::DockerNotAvailable);
        }

        let output = Command::new("docker")
            .args(["logs", "--tail", &tail.to_string(), container_id])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DockerError::CommandFailed(format!(
                "Failed to get logs: {stderr}"
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Restart a container.
    pub fn restart(container_id: &str) -> Result<(), DockerError> {
        if !Self::is_available() {
            return Err(DockerError::DockerNotAvailable);
        }

        let output = Command::new("docker")
            .args(["restart", container_id])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DockerError::CommandFailed(format!(
                "Failed to restart container: {stderr}"
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_docker_availability() {
        // This test just checks that the function doesn't panic
        let _available = DockerManager::is_available();
    }

    #[test]
    fn test_not_local_backend_error() {
        use apxm_core::types::ProviderProtocol;

        let backend = BackendConfig {
            name: "cloud-backend".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: None,
            api_key: Some("key".to_string()),
            headers: HashMap::new(),
            models: vec![],
            docker: None,
        };

        let result = DockerManager::start(&backend);
        assert!(matches!(result, Err(DockerError::NotLocalBackend(_))));
    }

    #[test]
    fn test_no_docker_config_error() {
        use apxm_core::types::ProviderProtocol;

        let backend = BackendConfig {
            name: "local-backend".to_string(),
            backend_type: BackendType::Local,
            protocol: ProviderProtocol::Ollama,
            endpoint: Some("http://localhost:11434".to_string()),
            api_key: None,
            headers: HashMap::new(),
            models: vec![],
            docker: None,
        };

        let result = DockerManager::start(&backend);
        assert!(matches!(result, Err(DockerError::NoDockerConfig(_))));
    }

    #[test]
    fn test_container_status_display() {
        assert_eq!(ContainerStatus::Running.to_string(), "running");
        assert_eq!(ContainerStatus::Stopped.to_string(), "stopped");
        assert_eq!(ContainerStatus::NotFound.to_string(), "not found");
    }
}
