//! Response memoization -- hash-based caching for deterministic LLM calls.
//!
//! When an LLM call has `temperature == 0.0`, the response is deterministic
//! for a given (prompt, model, system_prompt, tools) tuple.  This module
//! provides an in-process cache keyed by a hash of those inputs.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

const DEFAULT_TTL: Duration = Duration::from_secs(3600);
const DEFAULT_MAX_ENTRIES: usize = 1024;

#[derive(Debug, Clone)]
struct CacheEntry {
    content: String,
    input_tokens: usize,
    output_tokens: usize,
    model: String,
    inserted_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemoKey(u64);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoStats {
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
    pub entries: usize,
}

#[derive(Debug)]
pub struct ResponseCache {
    entries: RwLock<HashMap<MemoKey, CacheEntry>>,
    ttl: Duration,
    max_entries: usize,
    hits: std::sync::atomic::AtomicUsize,
    misses: std::sync::atomic::AtomicUsize,
    evictions: std::sync::atomic::AtomicUsize,
}

impl Default for ResponseCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponseCache {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            ttl: DEFAULT_TTL,
            max_entries: DEFAULT_MAX_ENTRIES,
            hits: std::sync::atomic::AtomicUsize::new(0),
            misses: std::sync::atomic::AtomicUsize::new(0),
            evictions: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn with_max_entries(mut self, max: usize) -> Self {
        self.max_entries = max;
        self
    }

    /// Compute a memo key from the deterministic components of an LLM request.
    /// Returns `None` if the request is non-deterministic (temperature > 0).
    pub fn compute_key(
        prompt: &str,
        system_prompt: Option<&str>,
        model: Option<&str>,
        temperature: f64,
    ) -> Option<MemoKey> {
        if temperature != 0.0 {
            return None;
        }
        let mut hasher = DefaultHasher::new();
        prompt.hash(&mut hasher);
        system_prompt.unwrap_or("").hash(&mut hasher);
        model.unwrap_or("default").hash(&mut hasher);
        Some(MemoKey(hasher.finish()))
    }

    pub fn get(&self, key: MemoKey) -> Option<CachedResponse> {
        let entries = self.entries.read();
        if let Some(entry) = entries.get(&key)
            && entry.inserted_at.elapsed() < self.ttl
        {
            self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Some(CachedResponse {
                content: entry.content.clone(),
                input_tokens: entry.input_tokens,
                output_tokens: entry.output_tokens,
                model: entry.model.clone(),
            });
        }
        self.misses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        None
    }

    pub fn put(
        &self,
        key: MemoKey,
        content: String,
        input_tokens: usize,
        output_tokens: usize,
        model: String,
    ) {
        let mut entries = self.entries.write();

        if entries.len() >= self.max_entries {
            let before = entries.len();
            let ttl = self.ttl;
            entries.retain(|_, e| e.inserted_at.elapsed() < ttl);
            let evicted = before - entries.len();
            self.evictions
                .fetch_add(evicted, std::sync::atomic::Ordering::Relaxed);

            if entries.len() >= self.max_entries
                && let Some(oldest_key) = entries
                    .iter()
                    .min_by_key(|(_, e)| e.inserted_at)
                    .map(|(k, _)| *k)
            {
                entries.remove(&oldest_key);
                self.evictions
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        entries.insert(
            key,
            CacheEntry {
                content,
                input_tokens,
                output_tokens,
                model,
                inserted_at: Instant::now(),
            },
        );
    }

    pub fn stats(&self) -> MemoStats {
        MemoStats {
            hits: self.hits.load(std::sync::atomic::Ordering::Relaxed),
            misses: self.misses.load(std::sync::atomic::Ordering::Relaxed),
            evictions: self.evictions.load(std::sync::atomic::Ordering::Relaxed),
            entries: self.entries.read().len(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CachedResponse {
    pub content: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub model: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_key_deterministic() {
        let key = ResponseCache::compute_key("hello", Some("system"), Some("gpt-4"), 0.0);
        assert!(key.is_some());
    }

    #[test]
    fn test_compute_key_non_deterministic() {
        let key = ResponseCache::compute_key("hello", Some("system"), Some("gpt-4"), 0.7);
        assert!(key.is_none());
    }

    #[test]
    fn test_same_inputs_same_key() {
        let k1 = ResponseCache::compute_key("prompt", Some("sys"), Some("m"), 0.0).unwrap();
        let k2 = ResponseCache::compute_key("prompt", Some("sys"), Some("m"), 0.0).unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_different_inputs_different_key() {
        let k1 = ResponseCache::compute_key("prompt_a", None, None, 0.0).unwrap();
        let k2 = ResponseCache::compute_key("prompt_b", None, None, 0.0).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_put_and_get() {
        let cache = ResponseCache::new();
        let key = ResponseCache::compute_key("prompt", None, None, 0.0).unwrap();

        cache.put(key, "response".to_string(), 10, 5, "gpt-4".to_string());

        let cached = cache.get(key).expect("should hit cache");
        assert_eq!(cached.content, "response");
        assert_eq!(cached.input_tokens, 10);
        assert_eq!(cached.output_tokens, 5);
    }

    #[test]
    fn test_miss() {
        let cache = ResponseCache::new();
        let key = MemoKey(999);
        assert!(cache.get(key).is_none());
    }

    #[test]
    fn test_ttl_expiry() {
        let cache = ResponseCache::new().with_ttl(Duration::from_millis(1));
        let key = ResponseCache::compute_key("prompt", None, None, 0.0).unwrap();

        cache.put(key, "response".to_string(), 10, 5, "gpt-4".to_string());
        std::thread::sleep(Duration::from_millis(5));

        assert!(cache.get(key).is_none());
    }

    #[test]
    fn test_max_entries_eviction() {
        let cache = ResponseCache::new()
            .with_max_entries(2)
            .with_ttl(Duration::from_secs(3600));

        for i in 0..3 {
            let key = MemoKey(i);
            cache.put(key, format!("resp_{}", i), 1, 1, "m".to_string());
        }

        let stats = cache.stats();
        assert!(stats.entries <= 2);
    }

    #[test]
    fn test_stats() {
        let cache = ResponseCache::new();
        let key = ResponseCache::compute_key("p", None, None, 0.0).unwrap();

        cache.get(key); // miss
        cache.put(key, "r".to_string(), 1, 1, "m".to_string());
        cache.get(key); // hit

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }
}
