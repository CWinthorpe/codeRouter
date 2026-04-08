use anyhow::{Context, Result};
use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;

const DB_FILENAME: &str = "metrics.db";

pub fn data_dir() -> Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("coderouter");

    if !dir.exists() {
        fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create data directory: {}", dir.display()))?;
    }

    Ok(dir)
}

pub fn db_path() -> Result<PathBuf> {
    Ok(data_dir()?.join(DB_FILENAME))
}

pub fn init_db() -> Result<Connection> {
    let path = db_path()?;
    let conn = Connection::open(&path)
        .with_context(|| format!("Failed to open SQLite database at {}", path.display()))?;

    run_migrations(&conn)?;

    Ok(conn)
}

pub fn init_in_memory_db() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;
    Ok(conn)
}

fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS requests (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            ts            INTEGER NOT NULL,
            group_alias   TEXT NOT NULL,
            provider_id   TEXT NOT NULL,
            model_id      TEXT NOT NULL,
            prompt_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cost_usd      REAL NOT NULL DEFAULT 0.0,
            latency_ms    INTEGER NOT NULL DEFAULT 0,
            status        TEXT NOT NULL,
            error_type    TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_requests_ts ON requests(ts);
        CREATE INDEX IF NOT EXISTS idx_requests_provider ON requests(provider_id, ts);
        CREATE INDEX IF NOT EXISTS idx_requests_group ON requests(group_alias, ts);
        ",
    )
    .context("Failed to run database migrations")?;

    Ok(())
}

pub fn clear_metrics() -> Result<()> {
    let path = db_path()?;
    let conn = Connection::open(&path)
        .with_context(|| format!("Failed to open SQLite database at {}", path.display()))?;
    conn.execute("DELETE FROM requests", [])
        .context("Failed to clear metrics data")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_in_memory_db() {
        let conn = init_in_memory_db().expect("Failed to init in-memory DB");
        let val: i32 = conn
            .query_row("SELECT 1", [], |r| r.get(0))
            .expect("Failed to query");
        assert_eq!(val, 1);
    }

    #[test]
    fn test_schema_exists() {
        let conn = init_in_memory_db().expect("Failed to init in-memory DB");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='requests'",
                [],
                |r| r.get(0),
            )
            .expect("Failed to check table existence");
        assert_eq!(count, 1);
    }

    #[test]
    fn test_indexes_exist() {
        let conn = init_in_memory_db().expect("Failed to init in-memory DB");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name LIKE 'idx_requests_%'",
                [],
                |r| r.get(0),
            )
            .expect("Failed to count indexes");
        assert_eq!(count, 3);
    }

    #[test]
    fn test_data_dir_creation() {
        let dir = data_dir().expect("Failed to get data dir");
        assert!(dir.ends_with("coderouter"));
    }
}
