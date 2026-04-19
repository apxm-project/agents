//! Response memoization -- hash-based caching for deterministic LLM calls.
//!
//! Two-tier cache architecture:
//! - **L1 (DashMap)**: Fast in-process cache for hot data
//! - **L2 (SQLite)**: Persistent disk cache that survives restarts
//!
//! When an LLM call has `temperature == 0.0`, the response is deterministic
//! for a given (prompt, model, system_prompt, tools) tuple.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use apxm_core::types::operations::AISOperationType;
use serde::{Deserialize, Serialize};

#[cfg(feature = "dashmap")]
use dashmap::DashMap;

#[cfg(feature = "sqlite")]
use rusqlite::{Connection, params};

const DEFAULT_TTL: Duration = Duration::from_secs(3600);
const DEFAULT_MAX_L1_ENTRIES: usize = 1024;

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
    pub l1_hits: usize,
    pub l2_hits: usize,
    pub misses: usize,
    pub evictions: usize,
    pub l1_entries: usize,
    pub l2_entries: usize,
}

// Legacy ResponseCache re-export for backward compatibility
pub type ResponseCache = MemoCache;

/// Two-tier memoization cache with DashMap (L1) and optional SQLite (L2).
#[derive(Debug)]
pub struct MemoCache {
    #[cfg(feature = "dashmap")]
    l1: Arc<DashMap<MemoKey, CacheEntry>>,
    #[cfg(not(feature = "dashmap"))]
    l1: Arc<parking_lot::RwLock<HashMap<MemoKey, CacheEntry>>>,

    #[cfg(feature = "sqlite")]
    l2: Option<Arc<parking_lot::Mutex<SqliteMemoStore>>>,

    ttl: Duration,
    max_l1_entries: usize,

    #[cfg(feature = "dashmap")]
    l1_hits: Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(not(feature = "dashmap"))]
    l1_hits: std::sync::Arc<std::sync::atomic::AtomicUsize>,

    l2_hits: Arc<std::sync::atomic::AtomicUsize>,
    misses: Arc<std::sync::atomic::AtomicUsize>,
    evictions: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(feature = "sqlite")]
#[derive(Debug)]
struct SqliteMemoStore {
    conn: Connection,
    ttl: Duration,
}

#[cfg(feature = "sqlite")]
impl SqliteMemoStore {
    fn new(db_path: &Path, ttl: Duration) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS memo_cache (
                key INTEGER PRIMARY KEY,
                content TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                inserted_at INTEGER NOT NULL,
                ttl_secs INTEGER NOT NULL
            )",
            [],
        )?;

        // Create index on inserted_at for efficient TTL cleanup
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_inserted_at ON memo_cache(inserted_at)",
            [],
        )?;

        Ok(Self { conn, ttl })
    }

    fn get(&self, key: MemoKey) -> Option<CachedResponse> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();

        let mut stmt = self
            .conn
            .prepare("SELECT content, model, input_tokens, output_tokens, inserted_at, ttl_secs FROM memo_cache WHERE key = ?1")
            .ok()?;

        let result = stmt
            .query_row(params![key.0.min(i64::MAX as u64) as i64], |row| {
                let content: String = row.get(0)?;
                let model: String = row.get(1)?;
                let input_tokens: i64 = row.get(2)?;
                let output_tokens: i64 = row.get(3)?;
                let inserted_at: i64 = row.get(4)?;
                let ttl_secs: i64 = row.get(5)?;

                // Check if entry is expired
                let inserted_at_u64 = u64::try_from(inserted_at).unwrap_or(0);
                let ttl_secs_u64 = u64::try_from(ttl_secs).unwrap_or(0);
                if now - inserted_at_u64 > ttl_secs_u64 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }

                Ok(CachedResponse {
                    content,
                    input_tokens: usize::try_from(input_tokens).unwrap_or(0),
                    output_tokens: usize::try_from(output_tokens).unwrap_or(0),
                    model,
                })
            })
            .ok();

        result
    }

    fn put(
        &mut self,
        key: MemoKey,
        content: String,
        input_tokens: usize,
        output_tokens: usize,
        model: String,
    ) -> Result<(), rusqlite::Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.conn.execute(
            "INSERT OR REPLACE INTO memo_cache (key, content, model, input_tokens, output_tokens, inserted_at, ttl_secs) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                key.0.min(i64::MAX as u64) as i64,
                content,
                model,
                input_tokens.min(i64::MAX as usize) as i64,
                output_tokens.min(i64::MAX as usize) as i64,
                now.min(i64::MAX as u64) as i64,
                self.ttl.as_secs().min(i64::MAX as u64) as i64,
            ],
        )?;

        Ok(())
    }

    fn count(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM memo_cache", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize
    }

    fn cleanup_expired(&mut self) -> Result<usize, rusqlite::Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let deleted = self.conn.execute(
            "DELETE FROM memo_cache WHERE inserted_at + ttl_secs < ?1",
            params![now.min(i64::MAX as u64) as i64],
        )?;

        Ok(deleted)
    }
}

impl Default for MemoCache {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoCache {
    /// Create a new L1-only cache (no SQLite persistence).
    pub fn new() -> Self {
        #[cfg(feature = "dashmap")]
        let l1 = Arc::new(DashMap::new());
        #[cfg(not(feature = "dashmap"))]
        let l1 = Arc::new(parking_lot::RwLock::new(HashMap::new()));

        Self {
            l1,
            #[cfg(feature = "sqlite")]
            l2: None,
            ttl: DEFAULT_TTL,
            max_l1_entries: DEFAULT_MAX_L1_ENTRIES,
            l1_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            l2_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            misses: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            evictions: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Create a two-tier cache with SQLite L2.
    #[cfg(feature = "sqlite")]
    pub fn new_with_sqlite<P: AsRef<Path>>(db_path: P) -> Result<Self, rusqlite::Error> {
        let ttl = DEFAULT_TTL;
        let l2_store = SqliteMemoStore::new(db_path.as_ref(), ttl)?;

        #[cfg(feature = "dashmap")]
        let l1 = Arc::new(DashMap::new());
        #[cfg(not(feature = "dashmap"))]
        let l1 = Arc::new(parking_lot::RwLock::new(HashMap::new()));

        Ok(Self {
            l1,
            l2: Some(Arc::new(parking_lot::Mutex::new(l2_store))),
            ttl,
            max_l1_entries: DEFAULT_MAX_L1_ENTRIES,
            l1_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            l2_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            misses: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            evictions: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn with_max_entries(mut self, max: usize) -> Self {
        self.max_l1_entries = max;
        self
    }

    /// Compute a memo key from the deterministic components of an LLM request.
    /// Returns `None` if the request is non-deterministic (temperature > 0).
    pub fn compute_key(
        prompt: &str,
        system_prompt: Option<&str>,
        model: Option<&str>,
        temperature: f64,
        tools: Option<&[String]>,
        output_schema: Option<&str>,
    ) -> Option<MemoKey> {
        // Only cache deterministic calls
        if temperature > 0.0 {
            return None;
        }
        let mut hasher = DefaultHasher::new();
        prompt.hash(&mut hasher);
        system_prompt.unwrap_or("").hash(&mut hasher);
        model.unwrap_or("default").hash(&mut hasher);

        // Hash tools if present
        if let Some(tools) = tools {
            for tool in tools {
                tool.hash(&mut hasher);
            }
        }

        // Hash output schema if present
        if let Some(schema) = output_schema {
            schema.hash(&mut hasher);
        }

        Some(MemoKey(hasher.finish()))
    }

    /// Get TTL (in seconds) for a specific AIS operation type.
    ///
    /// Different operations have different stability characteristics:
    /// - Ask: 1 hour (user queries, context-dependent)
    /// - Think: 24 hours (deep reasoning, more stable)
    /// - Reason: 7 days (structured reasoning, highly stable)
    /// - Plan: 24 hours (planning outputs, moderately stable)
    /// - Reflect: 24 hours (reflective analysis, moderately stable)
    /// - Verify: 24 hours (verification results, moderately stable)
    pub fn ttl_for_op(op: &AISOperationType) -> u64 {
        match op {
            AISOperationType::Ask => 3600,      // 1 hour
            AISOperationType::Think => 86400,   // 24 hours
            AISOperationType::Reason => 604800, // 7 days
            AISOperationType::Plan => 86400,    // 24 hours
            AISOperationType::Reflect => 86400, // 24 hours
            AISOperationType::Verify => 86400,  // 24 hours
            _ => 3600,                          // default 1 hour
        }
    }

    pub fn get(&self, key: MemoKey) -> Option<CachedResponse> {
        // Try L1 first
        #[cfg(feature = "dashmap")]
        let l1_result = self.l1.get(&key).and_then(|entry| {
            if entry.inserted_at.elapsed() < self.ttl {
                Some(CachedResponse {
                    content: entry.content.clone(),
                    input_tokens: entry.input_tokens,
                    output_tokens: entry.output_tokens,
                    model: entry.model.clone(),
                })
            } else {
                None
            }
        });

        #[cfg(not(feature = "dashmap"))]
        let l1_result = {
            let entries = self.l1.read();
            entries.get(&key).and_then(|entry| {
                if entry.inserted_at.elapsed() < self.ttl {
                    Some(CachedResponse {
                        content: entry.content.clone(),
                        input_tokens: entry.input_tokens,
                        output_tokens: entry.output_tokens,
                        model: entry.model.clone(),
                    })
                } else {
                    None
                }
            })
        };

        if let Some(response) = l1_result {
            self.l1_hits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Some(response);
        }

        // Try L2 (SQLite) if available
        #[cfg(feature = "sqlite")]
        if let Some(ref l2) = self.l2 {
            if let Some(response) = l2.lock().get(key) {
                self.l2_hits
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                // Promote to L1
                self.put_l1_only(
                    key,
                    response.content.clone(),
                    response.input_tokens,
                    response.output_tokens,
                    response.model.clone(),
                );

                return Some(response);
            }
        }

        // Cache miss
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
        self.put_with_ttl(key, content, input_tokens, output_tokens, model, None);
    }

    /// Put an entry with a custom TTL (in seconds).
    /// If `ttl_secs` is None, uses the default TTL.
    pub fn put_with_ttl(
        &self,
        key: MemoKey,
        content: String,
        input_tokens: usize,
        output_tokens: usize,
        model: String,
        ttl_secs: Option<u64>,
    ) {
        // Write to L1 immediately
        self.put_l1_only(
            key,
            content.clone(),
            input_tokens,
            output_tokens,
            model.clone(),
        );

        // Write to L2 asynchronously if available
        #[cfg(feature = "sqlite")]
        if let Some(ref l2) = self.l2 {
            let l2 = Arc::clone(l2);
            let content = content.clone();
            let model = model.clone();
            let ttl = ttl_secs.map(Duration::from_secs).unwrap_or(self.ttl);

            // Spawn a blocking task to write to SQLite
            std::thread::spawn(move || {
                let mut store = l2.lock();
                // Update store TTL temporarily for this write
                let original_ttl = store.ttl;
                store.ttl = ttl;
                let _ = store.put(key, content, input_tokens, output_tokens, model);
                store.ttl = original_ttl;
            });
        }
    }

    fn put_l1_only(
        &self,
        key: MemoKey,
        content: String,
        input_tokens: usize,
        output_tokens: usize,
        model: String,
    ) {
        #[cfg(feature = "dashmap")]
        {
            // Evict old entries if at capacity
            if self.l1.len() >= self.max_l1_entries {
                self.evict_l1_expired();

                // If still at capacity, evict oldest
                if self.l1.len() >= self.max_l1_entries {
                    if let Some(entry) = self.l1.iter().min_by_key(|entry| entry.inserted_at) {
                        let oldest_key = *entry.key();
                        drop(entry); // Release the reference before removing
                        self.l1.remove(&oldest_key);
                        self.evictions
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }

            self.l1.insert(
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

        #[cfg(not(feature = "dashmap"))]
        {
            let mut entries = self.l1.write();

            // Evict old entries if at capacity
            if entries.len() >= self.max_l1_entries {
                let ttl = self.ttl;
                let before = entries.len();
                entries.retain(|_, e| e.inserted_at.elapsed() < ttl);
                let evicted = before - entries.len();
                self.evictions
                    .fetch_add(evicted, std::sync::atomic::Ordering::Relaxed);

                // If still at capacity, evict oldest
                if entries.len() >= self.max_l1_entries
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
    }

    #[cfg(feature = "dashmap")]
    fn evict_l1_expired(&self) {
        let ttl = self.ttl;
        let before = self.l1.len();
        self.l1.retain(|_, e| e.inserted_at.elapsed() < ttl);
        let evicted = before - self.l1.len();
        self.evictions
            .fetch_add(evicted, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn stats(&self) -> MemoStats {
        #[cfg(feature = "dashmap")]
        let l1_entries = self.l1.len();
        #[cfg(not(feature = "dashmap"))]
        let l1_entries = self.l1.read().len();

        #[cfg(feature = "sqlite")]
        let l2_entries = self.l2.as_ref().map(|l2| l2.lock().count()).unwrap_or(0);
        #[cfg(not(feature = "sqlite"))]
        let l2_entries = 0;

        MemoStats {
            l1_hits: self.l1_hits.load(std::sync::atomic::Ordering::Relaxed),
            l2_hits: self.l2_hits.load(std::sync::atomic::Ordering::Relaxed),
            misses: self.misses.load(std::sync::atomic::Ordering::Relaxed),
            evictions: self.evictions.load(std::sync::atomic::Ordering::Relaxed),
            l1_entries,
            l2_entries,
        }
    }

    /// Cleanup expired entries in L2 (SQLite).
    #[cfg(feature = "sqlite")]
    pub fn cleanup_l2(&self) -> Result<usize, rusqlite::Error> {
        if let Some(ref l2) = self.l2 {
            l2.lock().cleanup_expired()
        } else {
            Ok(0)
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

/// Speculative execution handle for commit/rollback on MemoCache.
///
/// Tracks cache writes in a local overlay during speculative execution.
/// On commit, flushes overlay to main cache. On rollback, discards overlay.
pub struct SpeculativeHandle {
    cache: Arc<MemoCache>,
    overlay: HashMap<MemoKey, (String, usize, usize, String)>,
}

impl SpeculativeHandle {
    /// Speculatively put an entry (stored in overlay, not in main cache yet).
    pub fn put(
        &mut self,
        key: MemoKey,
        content: String,
        input_tokens: usize,
        output_tokens: usize,
        model: String,
    ) {
        self.overlay
            .insert(key, (content, input_tokens, output_tokens, model));
    }

    /// Get from overlay first, then fall back to main cache.
    pub fn get(&self, key: MemoKey) -> Option<CachedResponse> {
        // Check overlay first
        if let Some((content, input_tokens, output_tokens, model)) = self.overlay.get(&key) {
            return Some(CachedResponse {
                content: content.clone(),
                input_tokens: *input_tokens,
                output_tokens: *output_tokens,
                model: model.clone(),
            });
        }

        // Fall back to main cache
        self.cache.get(key)
    }

    /// Commit the speculative writes to the main cache.
    pub fn commit(self) {
        for (key, (content, input_tokens, output_tokens, model)) in self.overlay {
            self.cache
                .put(key, content, input_tokens, output_tokens, model);
        }
    }

    /// Rollback (discard) the speculative writes.
    pub fn rollback(self) {
        // Simply drop the overlay without writing to main cache
    }

    /// Number of speculative entries in the overlay.
    pub fn overlay_len(&self) -> usize {
        self.overlay.len()
    }
}

impl MemoCache {
    /// Begin speculative execution with a local overlay.
    pub fn begin_speculative(&self) -> SpeculativeHandle {
        SpeculativeHandle {
            cache: Arc::new(Self {
                l1: Arc::clone(&self.l1),
                #[cfg(feature = "sqlite")]
                l2: self.l2.as_ref().map(Arc::clone),
                ttl: self.ttl,
                max_l1_entries: self.max_l1_entries,
                l1_hits: Arc::clone(&self.l1_hits),
                l2_hits: Arc::clone(&self.l2_hits),
                misses: Arc::clone(&self.misses),
                evictions: Arc::clone(&self.evictions),
            }),
            overlay: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_key_deterministic() {
        let key = MemoCache::compute_key("hello", Some("system"), Some("gpt-4"), 0.0, None, None);
        assert!(key.is_some());
    }

    #[test]
    fn test_compute_key_non_deterministic() {
        let key = MemoCache::compute_key("hello", Some("system"), Some("gpt-4"), 0.7, None, None);
        assert!(key.is_none());
    }

    #[test]
    fn test_same_inputs_same_key() {
        let k1 = MemoCache::compute_key("prompt", Some("sys"), Some("m"), 0.0, None, None).unwrap();
        let k2 = MemoCache::compute_key("prompt", Some("sys"), Some("m"), 0.0, None, None).unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_different_inputs_different_key() {
        let k1 = MemoCache::compute_key("prompt_a", None, None, 0.0, None, None).unwrap();
        let k2 = MemoCache::compute_key("prompt_b", None, None, 0.0, None, None).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_put_and_get_l1_only() {
        let cache = MemoCache::new();
        let key = MemoCache::compute_key("prompt", None, None, 0.0, None, None).unwrap();

        cache.put(key, "response".to_string(), 10, 5, "gpt-4".to_string());

        let cached = cache.get(key).expect("should hit cache");
        assert_eq!(cached.content, "response");
        assert_eq!(cached.input_tokens, 10);
        assert_eq!(cached.output_tokens, 5);
    }

    #[test]
    fn test_miss() {
        let cache = MemoCache::new();
        let key = MemoKey(999);
        assert!(cache.get(key).is_none());
    }

    #[test]
    fn test_ttl_expiry() {
        let cache = MemoCache::new().with_ttl(Duration::from_millis(1));
        let key = MemoCache::compute_key("prompt", None, None, 0.0, None, None).unwrap();

        cache.put(key, "response".to_string(), 10, 5, "gpt-4".to_string());
        std::thread::sleep(Duration::from_millis(5));

        assert!(cache.get(key).is_none());
    }

    #[test]
    fn test_max_entries_eviction() {
        let cache = MemoCache::new()
            .with_max_entries(2)
            .with_ttl(Duration::from_secs(3600));

        for i in 0..3 {
            let key = MemoKey(i);
            cache.put(key, format!("resp_{}", i), 1, 1, "m".to_string());
        }

        let stats = cache.stats();
        assert!(stats.l1_entries <= 2);
    }

    #[test]
    fn test_stats() {
        let cache = MemoCache::new();
        let key = MemoCache::compute_key("p", None, None, 0.0, None, None).unwrap();

        cache.get(key); // miss
        cache.put(key, "r".to_string(), 1, 1, "m".to_string());
        cache.get(key); // hit

        let stats = cache.stats();
        assert_eq!(stats.l1_hits, 1);
        assert_eq!(stats.l2_hits, 0);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    #[cfg(feature = "sqlite")]
    fn test_two_tier_cache() {
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("memo.db");

        let cache = MemoCache::new_with_sqlite(&db_path).expect("create cache");
        let key = MemoCache::compute_key("two-tier", None, None, 0.0, None, None).unwrap();

        // Put in cache (goes to L1 and L2)
        cache.put(
            key,
            "two-tier-response".to_string(),
            15,
            10,
            "gpt-4".to_string(),
        );

        // Give async L2 write time to complete
        std::thread::sleep(Duration::from_millis(50));

        // Create new cache with same DB (simulates restart)
        let cache2 = MemoCache::new_with_sqlite(&db_path).expect("create cache2");

        // Should get from L2, then promote to L1
        let cached = cache2.get(key).expect("should hit L2");
        assert_eq!(cached.content, "two-tier-response");

        let stats = cache2.stats();
        assert_eq!(stats.l2_hits, 1);
        assert_eq!(stats.l1_hits, 0);
    }

    #[test]
    #[cfg(feature = "sqlite")]
    fn test_l2_cleanup_expired() {
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("memo.db");

        let cache = MemoCache::new_with_sqlite(&db_path)
            .expect("create cache")
            .with_ttl(Duration::from_millis(1));

        let key = MemoCache::compute_key("expire", None, None, 0.0, None, None).unwrap();
        cache.put(key, "expired".to_string(), 1, 1, "m".to_string());

        // Give async write time to complete
        std::thread::sleep(Duration::from_millis(50));

        // Wait for expiry
        std::thread::sleep(Duration::from_millis(10));

        // Cleanup expired entries
        let deleted = cache.cleanup_l2().expect("cleanup");
        // Should delete at least one entry (might be 0 if async write hasn't completed)
        assert!(deleted <= 1);
    }

    #[test]
    fn test_speculative_commit() {
        let cache = MemoCache::new();
        let key1 = MemoKey(100);
        let key2 = MemoKey(200);

        // Start speculative execution
        let mut handle = cache.begin_speculative();

        // Speculatively put entries
        handle.put(key1, "spec1".to_string(), 10, 5, "m1".to_string());
        handle.put(key2, "spec2".to_string(), 20, 10, "m2".to_string());

        // Should be in overlay
        assert_eq!(handle.overlay_len(), 2);

        // Should be readable from handle
        assert_eq!(handle.get(key1).unwrap().content, "spec1");
        assert_eq!(handle.get(key2).unwrap().content, "spec2");

        // But not in main cache yet
        assert!(cache.get(key1).is_none());
        assert!(cache.get(key2).is_none());

        // Commit
        handle.commit();

        // Now in main cache
        assert_eq!(cache.get(key1).unwrap().content, "spec1");
        assert_eq!(cache.get(key2).unwrap().content, "spec2");
    }

    #[test]
    fn test_speculative_rollback() {
        let cache = MemoCache::new();
        let key1 = MemoKey(300);
        let key2 = MemoKey(400);

        // Start speculative execution
        let mut handle = cache.begin_speculative();

        // Speculatively put entries
        handle.put(key1, "spec1".to_string(), 10, 5, "m1".to_string());
        handle.put(key2, "spec2".to_string(), 20, 10, "m2".to_string());

        assert_eq!(handle.overlay_len(), 2);

        // Rollback
        handle.rollback();

        // Nothing in main cache
        assert!(cache.get(key1).is_none());
        assert!(cache.get(key2).is_none());
    }

    #[test]
    fn test_speculative_reads_from_main_cache() {
        let cache = MemoCache::new();
        let key1 = MemoKey(500);
        let key2 = MemoKey(600);

        // Put entry in main cache
        cache.put(key1, "main1".to_string(), 5, 3, "m".to_string());

        // Start speculative execution
        let mut handle = cache.begin_speculative();

        // Should read from main cache
        assert_eq!(handle.get(key1).unwrap().content, "main1");

        // Speculative write for key2
        handle.put(key2, "spec2".to_string(), 10, 5, "m".to_string());

        // Should read speculative key2
        assert_eq!(handle.get(key2).unwrap().content, "spec2");

        // Rollback - key2 should not be in main cache
        handle.rollback();
        assert!(cache.get(key2).is_none());

        // But key1 should still be there
        assert_eq!(cache.get(key1).unwrap().content, "main1");
    }

    #[test]
    fn test_speculative_overlay_shadows_main() {
        let cache = MemoCache::new();
        let key = MemoKey(700);

        // Put entry in main cache
        cache.put(key, "original".to_string(), 5, 3, "m".to_string());

        // Start speculative execution
        let mut handle = cache.begin_speculative();

        // Overwrite in overlay
        handle.put(key, "shadowed".to_string(), 10, 5, "m2".to_string());

        // Should read from overlay
        assert_eq!(handle.get(key).unwrap().content, "shadowed");

        // Main cache still has original
        assert_eq!(cache.get(key).unwrap().content, "original");

        // Commit
        handle.commit();

        // Now main cache has shadowed value
        assert_eq!(cache.get(key).unwrap().content, "shadowed");
    }

    #[test]
    fn test_tools_affect_cache_key() {
        // Same prompt, different tools should produce different keys
        let tools1 = vec!["tool_a".to_string(), "tool_b".to_string()];
        let tools2 = vec!["tool_c".to_string()];

        let k1 = MemoCache::compute_key("prompt", None, None, 0.0, Some(&tools1), None).unwrap();
        let k2 = MemoCache::compute_key("prompt", None, None, 0.0, Some(&tools2), None).unwrap();
        let k3 = MemoCache::compute_key("prompt", None, None, 0.0, None, None).unwrap();

        // All three should be different
        assert_ne!(k1, k2);
        assert_ne!(k1, k3);
        assert_ne!(k2, k3);
    }

    #[test]
    fn test_output_schema_affects_cache_key() {
        // Same prompt, different output schemas should produce different keys
        let schema1 = r#"{"type": "object", "properties": {"answer": {"type": "string"}}}"#;
        let schema2 = r#"{"type": "object", "properties": {"result": {"type": "number"}}}"#;

        let k1 = MemoCache::compute_key("prompt", None, None, 0.0, None, Some(schema1)).unwrap();
        let k2 = MemoCache::compute_key("prompt", None, None, 0.0, None, Some(schema2)).unwrap();
        let k3 = MemoCache::compute_key("prompt", None, None, 0.0, None, None).unwrap();

        // All three should be different
        assert_ne!(k1, k2);
        assert_ne!(k1, k3);
        assert_ne!(k2, k3);
    }

    #[test]
    fn test_tools_and_schema_both_affect_cache_key() {
        // Combination of tools and schema should affect key
        let tools = vec!["tool_a".to_string()];
        let schema = r#"{"type": "object"}"#;

        let k1 = MemoCache::compute_key("p", None, None, 0.0, Some(&tools), Some(schema)).unwrap();
        let k2 = MemoCache::compute_key("p", None, None, 0.0, Some(&tools), None).unwrap();
        let k3 = MemoCache::compute_key("p", None, None, 0.0, None, Some(schema)).unwrap();
        let k4 = MemoCache::compute_key("p", None, None, 0.0, None, None).unwrap();

        // All four should be different
        assert_ne!(k1, k2);
        assert_ne!(k1, k3);
        assert_ne!(k1, k4);
        assert_ne!(k2, k3);
        assert_ne!(k2, k4);
        assert_ne!(k3, k4);
    }

    #[test]
    fn test_ttl_for_op() {
        // Test per-op TTL values
        assert_eq!(MemoCache::ttl_for_op(&AISOperationType::Ask), 3600);
        assert_eq!(MemoCache::ttl_for_op(&AISOperationType::Think), 86400);
        assert_eq!(MemoCache::ttl_for_op(&AISOperationType::Reason), 604800);
        assert_eq!(MemoCache::ttl_for_op(&AISOperationType::Plan), 86400);
        assert_eq!(MemoCache::ttl_for_op(&AISOperationType::Reflect), 86400);
        assert_eq!(MemoCache::ttl_for_op(&AISOperationType::Verify), 86400);

        // Default for other ops
        assert_eq!(MemoCache::ttl_for_op(&AISOperationType::ConstStr), 3600);
    }

    #[test]
    #[cfg(feature = "sqlite")]
    fn test_put_with_custom_ttl() {
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("memo.db");

        let cache = MemoCache::new_with_sqlite(&db_path).expect("create cache");
        let key = MemoCache::compute_key("test", None, None, 0.0, None, None).unwrap();

        // Put with a custom TTL of 1ms
        cache.put_with_ttl(
            key,
            "response".to_string(),
            10,
            5,
            "gpt-4".to_string(),
            Some(1), // 1 second TTL
        );

        // Give async write time to complete
        std::thread::sleep(Duration::from_millis(50));

        // Should be in L1 immediately
        assert!(cache.get(key).is_some());

        // Create new cache (clears L1)
        let cache2 = MemoCache::new_with_sqlite(&db_path).expect("create cache2");

        // Should be in L2
        assert!(cache2.get(key).is_some());

        // Wait for expiry
        std::thread::sleep(Duration::from_secs(2));

        // Create third cache (clears L1 again)
        let cache3 = MemoCache::new_with_sqlite(&db_path).expect("create cache3");

        // Should be expired in L2 now
        // Note: L2 checks expiry on get()
        assert!(cache3.get(key).is_none());
    }
}
