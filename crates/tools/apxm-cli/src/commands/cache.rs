//! MemoCache management commands.

use std::path::PathBuf;

use anyhow::Result;

use super::cli::*;

pub fn cache_command(action: CacheAction, json: bool) -> Result<()> {
    match action {
        CacheAction::Stats => cache_stats_command(json),
        CacheAction::Clear { yes } => cache_clear_command(yes, json),
        CacheAction::Export { output } => cache_export_command(output, json),
    }
}

fn get_cache_db_path() -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".apxm").join("cache.db"))
}

#[cfg(feature = "driver")]
pub fn cache_stats_command(json: bool) -> Result<()> {
    use rusqlite::Connection;

    let db_path = get_cache_db_path()?;

    if !db_path.exists() {
        if json {
            println!("{{\"exists\": false}}");
        } else {
            println!("No cache database found at {}", db_path.display());
        }
        return Ok(());
    }

    let conn = Connection::open(&db_path)?;

    // Get total count
    let total_entries: i64 =
        conn.query_row("SELECT COUNT(*) FROM memo_cache", [], |row| row.get(0))?;

    // Get total size (approximate from content lengths)
    let total_size: i64 = conn
        .query_row("SELECT SUM(LENGTH(content)) FROM memo_cache", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);

    // Get oldest and newest entries
    let oldest: Option<i64> = conn
        .query_row("SELECT MIN(inserted_at) FROM memo_cache", [], |row| {
            row.get(0)
        })
        .ok();

    let newest: Option<i64> = conn
        .query_row("SELECT MAX(inserted_at) FROM memo_cache", [], |row| {
            row.get(0)
        })
        .ok();

    // Get hit statistics (we don't track this in the schema, so we'll just show entry count)
    // In a real implementation, you'd add a hit_count column to track this

    if json {
        let output = serde_json::json!({
            "exists": true,
            "path": db_path.display().to_string(),
            "total_entries": total_entries,
            "total_size_bytes": total_size,
            "oldest_entry_timestamp": oldest,
            "newest_entry_timestamp": newest,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("MemoCache Statistics");
        println!("===================");
        println!("Database: {}", db_path.display());
        println!("Total entries: {}", total_entries);
        println!("Total size: {:.2} MB", total_size as f64 / 1_000_000.0);

        if let Some(oldest_ts) = oldest {
            use std::time::{Duration, SystemTime, UNIX_EPOCH};
            let oldest_time = UNIX_EPOCH + Duration::from_secs(oldest_ts as u64);
            if let Ok(duration) = SystemTime::now().duration_since(oldest_time) {
                println!("Oldest entry: {} days ago", duration.as_secs() / 86400);
            }
        }

        if let Some(newest_ts) = newest {
            use std::time::{Duration, SystemTime, UNIX_EPOCH};
            let newest_time = UNIX_EPOCH + Duration::from_secs(newest_ts as u64);
            if let Ok(duration) = SystemTime::now().duration_since(newest_time) {
                println!("Newest entry: {} seconds ago", duration.as_secs());
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "driver"))]
pub fn cache_stats_command(json: bool) -> Result<()> {
    if json {
        println!("{{\"error\": \"Cache commands require the driver feature\"}}");
    } else {
        println!("Cache commands require the driver feature");
        println!("Rebuild with: cargo build -p apxm-cli --features driver");
    }
    Err(anyhow::anyhow!("Driver feature required"))
}

#[cfg(feature = "driver")]
pub fn cache_clear_command(yes: bool, json: bool) -> Result<()> {
    use rusqlite::Connection;
    use std::io::{self, Write};

    let db_path = get_cache_db_path()?;

    if !db_path.exists() {
        if json {
            println!("{{\"exists\": false, \"cleared\": 0}}");
        } else {
            println!("No cache database found");
        }
        return Ok(());
    }

    let conn = Connection::open(&db_path)?;

    // Get count before clearing
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM memo_cache", [], |row| row.get(0))?;

    if count == 0 {
        if json {
            println!("{{\"exists\": true, \"cleared\": 0}}");
        } else {
            println!("Cache is already empty");
        }
        return Ok(());
    }

    if !yes && !json {
        print!("Delete {} cached entries? [y/N] ", count);
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled");
            return Ok(());
        }
    }

    conn.execute("DELETE FROM memo_cache", [])?;

    if json {
        println!("{{\"exists\": true, \"cleared\": {}}}", count);
    } else {
        println!("Cleared {} entries from cache", count);
    }

    Ok(())
}

#[cfg(not(feature = "driver"))]
pub fn cache_clear_command(_yes: bool, json: bool) -> Result<()> {
    if json {
        println!("{{\"error\": \"Cache commands require the driver feature\"}}");
    } else {
        println!("Cache commands require the driver feature");
    }
    Err(anyhow::anyhow!("Driver feature required"))
}

#[cfg(feature = "driver")]
pub fn cache_export_command(output: Option<PathBuf>, json_flag: bool) -> Result<()> {
    use rusqlite::Connection;

    let db_path = get_cache_db_path()?;

    if !db_path.exists() {
        if json_flag {
            println!("{{\"exists\": false, \"entries\": []}}");
        } else {
            println!("No cache database found");
        }
        return Ok(());
    }

    let conn = Connection::open(&db_path)?;
    let mut stmt = conn.prepare(
        "SELECT key, content, model, input_tokens, output_tokens, inserted_at, ttl_secs FROM memo_cache"
    )?;

    let entries: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "key": row.get::<_, i64>(0)?,
                "content": row.get::<_, String>(1)?,
                "model": row.get::<_, String>(2)?,
                "input_tokens": row.get::<_, i64>(3)?,
                "output_tokens": row.get::<_, i64>(4)?,
                "inserted_at": row.get::<_, i64>(5)?,
                "ttl_secs": row.get::<_, i64>(6)?,
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let output_json = serde_json::json!({
        "exists": true,
        "entries": entries,
        "total": entries.len(),
    });

    let json_str = serde_json::to_string_pretty(&output_json)?;

    if let Some(path) = output {
        std::fs::write(&path, json_str)?;
        if !json_flag {
            println!("Exported {} entries to {}", entries.len(), path.display());
        }
    } else {
        println!("{}", json_str);
    }

    Ok(())
}

#[cfg(not(feature = "driver"))]
pub fn cache_export_command(_output: Option<PathBuf>, json: bool) -> Result<()> {
    if json {
        println!("{{\"error\": \"Cache commands require the driver feature\"}}");
    } else {
        println!("Cache commands require the driver feature");
    }
    Err(anyhow::anyhow!("Driver feature required"))
}

// ========================================================================
// Workflow command handlers
// ========================================================================

