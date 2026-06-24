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

        stmt.query_row(params![key.0.min(i64::MAX as u64) as i64], |row| {
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
        .ok()
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
            // Evict aged entries if at capacity.
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

            // Evict aged entries if at capacity.
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
