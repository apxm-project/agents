//! SQLite storage backend with connection pooling for high performance
//! Provides persistent, queryable storage suitable for LTM (Long-Term Memory)

use crate::storage::embedder::{Embedder, cosine_similarity};
use crate::storage::{SearchResult, StorageBackend, StorageResult, backend::BackendStats};
use apxm_core::{error::RuntimeError, types::values::Value};
use async_trait::async_trait;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{path::Path, sync::Arc, time::Duration};

const SQLITE_SPACE: &str = "sqlite";
const SCHEMA_KV_STORE: &str = r"
CREATE TABLE IF NOT EXISTS kv_store (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) WITHOUT ROWID;
";

/// FTS5 virtual table for BM25 full-text search on key + value content.
const SCHEMA_KV_FTS: &str = r"
CREATE VIRTUAL TABLE IF NOT EXISTS kv_fts USING fts5(
    key,
    content,
    tokenize='unicode61'
);
";

/// Embeddings table for vector similarity search.
const SCHEMA_KV_EMBEDDINGS: &str = r"
CREATE TABLE IF NOT EXISTS kv_embeddings (
    key        TEXT PRIMARY KEY NOT NULL,
    embedding  BLOB NOT NULL
);
";

/// Default hybrid search weights.
const VECTOR_WEIGHT: f64 = 0.7;
const BM25_WEIGHT: f64 = 0.3;

#[inline]
fn mem_err(context: &'static str, e: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Memory {
        message: format!("sqlite {context}: {e}"),
        space: Some(SQLITE_SPACE.to_string()),
    }
}

#[inline]
fn ser_err(context: &'static str, e: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Serialization(format!("{context}: {e}"))
}

/// Escape user input for use inside a `LIKE` pattern, treating the query as a literal substring.
/// Uses backslash as the escape character (paired with `ESCAPE '\\'`).
#[inline]
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

#[derive(Clone)]
pub struct SqliteBackend {
    pool: SqlitePool,
    embedder: Option<Arc<dyn Embedder>>,
}

impl SqliteBackend {
    /// Time:  O(1) per connection (schema is constant-size)
    /// Space: O(1)
    pub async fn new<P: AsRef<Path>>(path: P, max_connections: Option<u32>) -> StorageResult<Self> {
        let path = path.as_ref();
        let in_memory = path == Path::new(":memory:");

        let max_conn = if in_memory {
            1
        } else {
            max_connections.unwrap_or(8).max(1)
        };

        if !in_memory && let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| mem_err("create_dir_all", e))?;
        }

        let mut options = SqliteConnectOptions::new()
            .busy_timeout(Duration::from_secs(30))
            .foreign_keys(true)
            .statement_cache_capacity(128);

        options = if in_memory {
            options.in_memory(true)
        } else {
            options.filename(path).create_if_missing(true)
        };

        if !in_memory {
            options = options
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal);
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(max_conn)
            .acquire_timeout(Duration::from_secs(30))
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query(SCHEMA_KV_STORE).execute(&mut *conn).await?;
                    sqlx::query(SCHEMA_KV_FTS).execute(&mut *conn).await?;
                    sqlx::query(SCHEMA_KV_EMBEDDINGS)
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(options)
            .await
            .map_err(|e| mem_err("pool connect", e))?;

        Ok(Self {
            pool,
            embedder: None,
        })
    }

    pub async fn in_memory() -> StorageResult<Self> {
        Self::new(":memory:", None).await
    }

    /// Attach an embedder for vector similarity search.
    ///
    /// When set, `put()` generates and stores embeddings automatically,
    /// and `search_vector()` uses hybrid BM25 + cosine similarity scoring.
    pub fn with_embedder(mut self, embedder: Arc<dyn Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Extract text content from a JSON value for FTS5 indexing.
    fn extract_text_for_fts(value_json: &str) -> String {
        // Try to parse as JSON and extract text-like fields
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(value_json) {
            let mut parts = Vec::new();
            Self::collect_text_fields(&parsed, &mut parts);
            if !parts.is_empty() {
                return parts.join(" ");
            }
        }
        // Fall back to raw JSON string (FTS5 will tokenize it)
        value_json.to_string()
    }

    /// Recursively collect string values from a JSON structure.
    fn collect_text_fields(value: &serde_json::Value, parts: &mut Vec<String>) {
        match value {
            serde_json::Value::String(s) => parts.push(s.clone()),
            serde_json::Value::Array(arr) => {
                for item in arr {
                    Self::collect_text_fields(item, parts);
                }
            }
            serde_json::Value::Object(obj) => {
                for v in obj.values() {
                    Self::collect_text_fields(v, parts);
                }
            }
            _ => {}
        }
    }
}

#[async_trait]
impl StorageBackend for SqliteBackend {
    async fn put(&self, key: &str, value: Value) -> StorageResult<()> {
        let value_json =
            serde_json::to_string(&value).map_err(|e| ser_err("serialize value", e))?;

        sqlx::query(
            r"
            INSERT INTO kv_store (key, value, updated_at)
            VALUES (?1, ?2, unixepoch())
            ON CONFLICT(key) DO UPDATE
              SET value = excluded.value,
                  updated_at = unixepoch();
            ",
        )
        .bind(key)
        .bind(&value_json)
        .execute(&self.pool)
        .await
        .map_err(|e| mem_err("put", e))?;

        // Maintain FTS5 index (virtual tables don't support ON CONFLICT)
        let content = Self::extract_text_for_fts(&value_json);
        let _ = sqlx::query("DELETE FROM kv_fts WHERE key = ?1")
            .bind(key)
            .execute(&self.pool)
            .await;
        sqlx::query("INSERT INTO kv_fts (key, content) VALUES (?1, ?2)")
            .bind(key)
            .bind(&content)
            .execute(&self.pool)
            .await
            .map_err(|e| mem_err("put fts", e))?;

        // Store embedding if embedder is available
        if let Some(embedder) = &self.embedder
            && let Ok(embedding) = embedder.embed_one(&content)
        {
            let blob = embedding_to_blob(&embedding);
            sqlx::query(
                r"
                    INSERT INTO kv_embeddings (key, embedding)
                    VALUES (?1, ?2)
                    ON CONFLICT(key) DO UPDATE
                      SET embedding = excluded.embedding;
                    ",
            )
            .bind(key)
            .bind(&blob)
            .execute(&self.pool)
            .await
            .map_err(|e| mem_err("put embedding", e))?;
        }

        Ok(())
    }

    async fn get(&self, key: &str) -> StorageResult<Option<Value>> {
        let value_json =
            sqlx::query_scalar::<_, String>("SELECT value FROM kv_store WHERE key = ?1")
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| mem_err("get", e))?;

        value_json
            .map(|s| serde_json::from_str::<Value>(&s).map_err(|e| ser_err("deserialize value", e)))
            .transpose()
    }

    async fn delete(&self, key: &str) -> StorageResult<()> {
        sqlx::query("DELETE FROM kv_store WHERE key = ?1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| mem_err("delete", e))?;
        sqlx::query("DELETE FROM kv_fts WHERE key = ?1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| mem_err("delete fts", e))?;
        sqlx::query("DELETE FROM kv_embeddings WHERE key = ?1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| mem_err("delete embedding", e))?;
        Ok(())
    }

    async fn exists(&self, key: &str) -> StorageResult<bool> {
        let exists =
            sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM kv_store WHERE key = ?1)")
                .bind(key)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| mem_err("exists", e))?;

        Ok(exists != 0)
    }

    async fn list_keys(&self) -> StorageResult<Vec<String>> {
        sqlx::query_scalar::<_, String>("SELECT key FROM kv_store ORDER BY key")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| mem_err("list_keys", e))
    }

    async fn search(&self, query: &str, limit: usize) -> StorageResult<Vec<SearchResult>> {
        if limit == 0 || query.is_empty() {
            return Ok(Vec::new());
        }

        let escaped = escape_like(query);
        let pattern = format!("%{escaped}%");
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);

        let rows: Vec<(String, String)> = sqlx::query_as(
            r"
            SELECT key, value
            FROM kv_store
            WHERE key LIKE ?1 ESCAPE '\'
            ORDER BY key
            LIMIT ?2;
            ",
        )
        .bind(pattern)
        .bind(limit_i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| mem_err("search", e))?;

        let mut results = Vec::with_capacity(rows.len());
        for (key, value_json) in rows {
            let value = serde_json::from_str::<Value>(&value_json)
                .map_err(|e| ser_err("deserialize value", e))?;

            results.push(SearchResult {
                key,
                value,
                score: 1.0,
            });
        }

        Ok(results)
    }

    /// Hybrid search using FTS5 BM25 + optional vector cosine similarity.
    ///
    /// Without an embedder: uses FTS5 BM25 scoring only (still much better than substring).
    /// With an embedder: combines `score = vector_weight * cosine + bm25_weight * bm25_norm`.
    async fn search_vector(&self, query: &str, limit: usize) -> StorageResult<Vec<SearchResult>> {
        if limit == 0 || query.is_empty() {
            return Ok(Vec::new());
        }

        // Fetch extra candidates for re-ranking
        let fetch_limit = i64::try_from(limit * 3).unwrap_or(i64::MAX);

        // Step 1: FTS5 BM25 search
        let fts_rows: Vec<(String, f64)> = sqlx::query_as(
            r"
            SELECT key, rank
            FROM kv_fts
            WHERE kv_fts MATCH ?1
            ORDER BY rank
            LIMIT ?2
            ",
        )
        .bind(query)
        .bind(fetch_limit)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default(); // FTS5 MATCH can fail on odd query syntax

        if fts_rows.is_empty() {
            // Fall back to substring search if FTS5 found nothing
            return self.search(query, limit).await;
        }

        // Normalize BM25 scores to 0..1 (rank is negative, lower = better)
        let min_rank = fts_rows
            .iter()
            .map(|(_, r)| *r)
            .fold(f64::INFINITY, f64::min);
        let max_rank = fts_rows
            .iter()
            .map(|(_, r)| *r)
            .fold(f64::NEG_INFINITY, f64::max);
        let range = (max_rank - min_rank).max(f64::EPSILON);

        let mut scored: Vec<(String, f64)> = fts_rows
            .iter()
            .map(|(key, rank)| {
                // BM25 rank is negative; more negative = more relevant
                let bm25_norm = 1.0 - ((rank - min_rank) / range);
                (key.clone(), bm25_norm)
            })
            .collect();

        // Step 2: If embedder is available, combine with vector similarity
        if let Some(embedder) = &self.embedder
            && let Ok(query_embedding) = embedder.embed_one(query)
        {
            let keys: Vec<String> = scored.iter().map(|(k, _)| k.clone()).collect();
            let placeholders = keys
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(",");
            let sql =
                format!("SELECT key, embedding FROM kv_embeddings WHERE key IN ({placeholders})");

            let mut query_builder = sqlx::query_as::<_, (String, Vec<u8>)>(&sql);
            for key in &keys {
                query_builder = query_builder.bind(key);
            }

            if let Ok(emb_rows) = query_builder.fetch_all(&self.pool).await {
                let emb_map: std::collections::HashMap<String, Vec<f32>> = emb_rows
                    .into_iter()
                    .filter_map(|(key, blob)| blob_to_embedding(&blob).map(|e| (key, e)))
                    .collect();

                for (key, bm25_score) in &mut scored {
                    if let Some(stored_emb) = emb_map.get(key) {
                        let cos_sim =
                            cosine_similarity(&query_embedding, stored_emb).max(0.0) as f64;
                        *bm25_score = VECTOR_WEIGHT * cos_sim + BM25_WEIGHT * *bm25_score;
                    } else {
                        // No embedding available — use BM25 alone, weighted
                        *bm25_score *= BM25_WEIGHT;
                    }
                }
            }
        }

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        // Fetch values for top results
        let mut results = Vec::with_capacity(scored.len());
        for (key, score) in scored {
            if let Some(value) = self.get(&key).await? {
                results.push(SearchResult { key, value, score });
            }
        }

        Ok(results)
    }

    async fn clear(&self) -> StorageResult<()> {
        sqlx::query("DELETE FROM kv_store")
            .execute(&self.pool)
            .await
            .map_err(|e| mem_err("clear", e))?;
        sqlx::query("DELETE FROM kv_fts")
            .execute(&self.pool)
            .await
            .map_err(|e| mem_err("clear fts", e))?;
        sqlx::query("DELETE FROM kv_embeddings")
            .execute(&self.pool)
            .await
            .map_err(|e| mem_err("clear embeddings", e))?;
        Ok(())
    }

    async fn stats(&self) -> StorageResult<BackendStats> {
        let total_keys = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM kv_store")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| mem_err("stats count", e))?;

        let page_count = sqlx::query_scalar::<_, i64>("PRAGMA page_count")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| mem_err("stats page_count", e))?;

        let page_size = sqlx::query_scalar::<_, i64>("PRAGMA page_size")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| mem_err("stats page_size", e))?;

        let total_size_bytes = (page_count * page_size) as usize;

        Ok(BackendStats {
            total_keys: total_keys.max(0) as usize,
            total_size_bytes,
            backend_type: SQLITE_SPACE.to_string(),
        })
    }
}

/// Serialize f32 embedding as little-endian byte blob for SQLite storage.
fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserialize f32 embedding from little-endian byte blob.
fn blob_to_embedding(blob: &[u8]) -> Option<Vec<f32>> {
    let (chunks, []) = blob.as_chunks::<4>() else {
        return None;
    };

    Some(
        chunks
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
            .collect(),
    )
}
