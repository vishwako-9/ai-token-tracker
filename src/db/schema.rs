use anyhow::{bail, Result};
use rusqlite::Connection;

const LATEST_SCHEMA_VERSION: i64 = 2;

const CREATE_USAGE_RECORDS_TABLE_SQL: &str = "
    CREATE TABLE IF NOT EXISTS usage_records (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        provider TEXT NOT NULL,
        model TEXT NOT NULL,
        input_tokens INTEGER NOT NULL,
        output_tokens INTEGER NOT NULL,
        cache_read_tokens INTEGER DEFAULT 0,
        cache_write_tokens INTEGER DEFAULT 0,
        reasoning_tokens INTEGER DEFAULT 0,
        cost_usd REAL,
        session_id TEXT,
        recorded_at TEXT NOT NULL,
        collected_at TEXT NOT NULL,
        metadata TEXT
    );
";

const CREATE_INDEXES_SQL: &str = "
    CREATE INDEX IF NOT EXISTS idx_provider ON usage_records(provider);
    CREATE INDEX IF NOT EXISTS idx_recorded_at ON usage_records(recorded_at);
    CREATE INDEX IF NOT EXISTS idx_model ON usage_records(model);
    CREATE INDEX IF NOT EXISTS idx_provider_recorded ON usage_records(provider, recorded_at);
    CREATE UNIQUE INDEX IF NOT EXISTS idx_dedup ON usage_records(
        provider,
        model,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        reasoning_tokens,
        recorded_at,
        COALESCE(session_id, ''),
        COALESCE(cost_usd, -1)
    );
";

// Request-volume table for providers whose local logs expose no token counts
// (Antigravity). Kept strictly separate from usage_records: no token or cost
// columns, so these rows can never pollute token/cost aggregates.
const CREATE_ANTIGRAVITY_REQUESTS_TABLE_SQL: &str = "
    CREATE TABLE IF NOT EXISTS antigravity_requests (
        date TEXT NOT NULL,
        model TEXT NOT NULL,
        request_count INTEGER NOT NULL,
        PRIMARY KEY (date, model)
    );
";

pub fn initialize(conn: &Connection) -> Result<()> {
    let current = current_schema_version(conn)?;

    if current > LATEST_SCHEMA_VERSION {
        bail!(
            "Database schema version {} is newer than supported version {}",
            current,
            LATEST_SCHEMA_VERSION
        );
    }

    if current == 0 {
        if table_exists(conn, "usage_records")? {
            migrate(conn, 1, LATEST_SCHEMA_VERSION)?;
        } else {
            let tx = conn.unchecked_transaction()?;
            create_schema_v1(&tx)?;
            set_schema_version(&tx, LATEST_SCHEMA_VERSION)?;
            tx.commit()?;
        }
        return Ok(());
    }

    migrate(conn, current, LATEST_SCHEMA_VERSION)
}

fn current_schema_version(conn: &Connection) -> Result<i64> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

fn set_schema_version(conn: &Connection, version: i64) -> Result<()> {
    conn.pragma_update(None, "user_version", version)?;
    Ok(())
}

fn create_schema_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(CREATE_USAGE_RECORDS_TABLE_SQL)?;
    conn.execute_batch(CREATE_INDEXES_SQL)?;
    conn.execute_batch(CREATE_ANTIGRAVITY_REQUESTS_TABLE_SQL)?;
    Ok(())
}

fn migrate(conn: &Connection, from: i64, to: i64) -> Result<()> {
    if from > to {
        bail!("Cannot migrate database backwards from version {} to {}", from, to);
    }
    if from == to {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    let mut version = from;
    while version < to {
        match version {
            1 => migrate_v1_to_v2(&tx)?,
            _ => bail!("No migration path from schema version {}", version),
        }
        version += 1;
        set_schema_version(&tx, version)?;
    }
    tx.commit()?;
    Ok(())
}

fn migrate_v1_to_v2(conn: &Connection) -> Result<()> {
    conn.execute_batch(CREATE_ANTIGRAVITY_REQUESTS_TABLE_SQL)?;
    Ok(())
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let exists = conn.query_row(
        "SELECT EXISTS(
            SELECT 1
            FROM sqlite_master
            WHERE type = 'table' AND name = ?1
        )",
        [name],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(exists != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initializes_fresh_database() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        assert_eq!(current_schema_version(&conn).unwrap(), LATEST_SCHEMA_VERSION);
        assert!(table_exists(&conn, "usage_records").unwrap());
        assert!(table_exists(&conn, "antigravity_requests").unwrap());
    }

    #[test]
    fn dedup_index_admits_distinct_rows() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        let insert = "INSERT OR IGNORE INTO usage_records (
            provider, model, input_tokens, output_tokens,
            cache_read_tokens, cache_write_tokens, reasoning_tokens,
            cost_usd, session_id, recorded_at, collected_at, metadata
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)";
        let run = |c: f64| {
            conn.execute(
                insert,
                rusqlite::params![
                    "opencode", "gpt-5", 100, 50, 0, 0, 0, c,
                    Option::<String>::None, "2026-08-15", "2026-08-15T00:00:00Z",
                    Option::<String>::None,
                ],
            )
        };
        assert_eq!(run(0.10).unwrap(), 1);
        assert_eq!(run(0.10).unwrap(), 0);
        assert_eq!(run(0.20).unwrap(), 1);
    }
}