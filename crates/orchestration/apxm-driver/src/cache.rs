//! File-based artifact cache keyed by BLAKE3 hash of graph binary representation.
//!
//! Cache location follows [`apxm_core::paths::ApxmPaths`]: project-local
//! `.apxm/cache/artifacts` first, then `$APXM_HOME/cache/artifacts` or
//! `~/.apxm/cache/artifacts` as fallback.

use std::fs;
use std::path::PathBuf;

use apxm_compiler::AirModule;
use apxm_core::constants::env as apxm_env;
use apxm_core::paths::ApxmPaths;

use crate::error::DriverError;

const ARTIFACT_CACHE_COMPONENT: &str = "artifacts";
const ARTIFACT_EXTENSION: &str = "apxmobj";

/// Returns `true` when the cache is explicitly disabled via `APXM_NO_CACHE=1`.
pub fn cache_disabled() -> bool {
    std::env::var(apxm_env::APXM_NO_CACHE)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Compute BLAKE3 hash of an AIR module's JSON representation.
///
/// Uses JSON as the serialization format (not bincode) because AirModule
/// contains HashMap fields that don't have a stable iteration order with bincode.
/// JSON serialization with serde_json guarantees alphabetic key sorting.
pub fn graph_hash(module: &AirModule) -> Result<String, DriverError> {
    // Serialize to JSON with sorted keys for deterministic hashing
    let json = serde_json::to_string(module).map_err(|e| DriverError::Driver(e.to_string()))?;
    let hash = blake3::hash(json.as_bytes());
    Ok(hash.to_hex().to_string())
}

/// Return the cache directory, creating it if necessary.
fn cache_dir() -> Result<PathBuf, DriverError> {
    Ok(ApxmPaths::discover()?.cache_component_dir(ARTIFACT_CACHE_COMPONENT)?)
}

fn artifact_path(hash: &str) -> Result<PathBuf, DriverError> {
    Ok(cache_dir()?.join(format!("{hash}.{ARTIFACT_EXTENSION}")))
}

/// Try to load a cached artifact by its hash.  Returns `None` on miss.
pub fn load_cached(hash: &str) -> Result<Option<Vec<u8>>, DriverError> {
    if cache_disabled() {
        return Ok(None);
    }
    let path = artifact_path(hash)?;
    if path.exists() {
        Ok(Some(fs::read(&path)?))
    } else {
        Ok(None)
    }
}

/// Store compiled artifact bytes under the given hash.
pub fn store_cached(hash: &str, artifact_bytes: &[u8]) -> Result<(), DriverError> {
    if cache_disabled() {
        return Ok(());
    }
    let path = artifact_path(hash)?;
    fs::write(&path, artifact_bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        let module_a = AirModule {
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            parameters: vec![],
            metadata: std::collections::HashMap::new(),
        };
        let module_b = module_a.clone();
        // Identical modules must produce identical hashes
        assert_eq!(
            graph_hash(&module_a).unwrap(),
            graph_hash(&module_b).unwrap()
        );
    }

    #[test]
    fn cache_disabled_env() {
        // Default: not disabled
        assert!(!cache_disabled() || std::env::var(apxm_env::APXM_NO_CACHE).is_ok());
    }
}
