//! File-based artifact cache keyed by BLAKE3 hash of graph binary representation.
//!
//! Cache location: `~/.cache/apxm/artifacts/<hash>.apxmobj`

use std::env;
use std::fs;
use std::path::PathBuf;

use apxm_graph::ApxmGraph;
use crate::error::DriverError;

/// Returns `true` when the cache is explicitly disabled via `APXM_NO_CACHE=1`.
pub fn cache_disabled() -> bool {
    env::var("APXM_NO_CACHE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Compute BLAKE3 hash of graph's JSON representation.
///
/// Uses JSON as the serialization format (not bincode) because ApxmGraph
/// contains HashMap fields that don't have a stable iteration order with bincode.
/// JSON serialization with serde_json guarantees alphabetic key sorting.
pub fn graph_hash(graph: &ApxmGraph) -> Result<String, DriverError> {
    // Serialize to JSON with sorted keys for deterministic hashing
    let json = graph.to_json()
        .map_err(|e| DriverError::Driver(e.to_string()))?;
    let hash = blake3::hash(json.as_bytes());
    Ok(hash.to_hex().to_string())
}

/// Return the cache directory, creating it if necessary.
fn cache_dir() -> Result<PathBuf, DriverError> {
    let base = dirs::cache_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| DriverError::Driver("cannot determine cache directory".into()))?;
    let dir = base.join("apxm").join("artifacts");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Try to load a cached artifact by its hash.  Returns `None` on miss.
pub fn load_cached(hash: &str) -> Result<Option<Vec<u8>>, DriverError> {
    if cache_disabled() {
        return Ok(None);
    }
    let path = cache_dir()?.join(format!("{hash}.apxmobj"));
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
    let path = cache_dir()?.join(format!("{hash}.apxmobj"));
    fs::write(&path, artifact_bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        let graph_a = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            parameters: vec![],
            metadata: std::collections::HashMap::new(),
        };
        let graph_b = graph_a.clone();
        // Identical graphs must produce identical hashes
        assert_eq!(graph_hash(&graph_a).unwrap(), graph_hash(&graph_b).unwrap());
    }

    #[test]
    fn cache_disabled_env() {
        // Default: not disabled
        assert!(!cache_disabled() || env::var("APXM_NO_CACHE").is_ok());
    }
}
