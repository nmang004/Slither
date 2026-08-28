use anyhow::Result;
use rusqlite::Connection;

pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    // NORMAL is safe under WAL and much faster; a 5s busy timeout lets the
    // dashboard and server coexist without immediate SQLITE_BUSY errors.
    conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS jobs (
            id              TEXT PRIMARY KEY,
            type            TEXT NOT NULL,
            status          TEXT NOT NULL,
            domain          TEXT NOT NULL,
            url             TEXT NOT NULL,
            config          TEXT NOT NULL,
            progress        TEXT,
            result_summary  TEXT,
            output_dir      TEXT,
            error           TEXT,
            created_at      TEXT NOT NULL,
            started_at      TEXT,
            completed_at    TEXT
        );

        CREATE TABLE IF NOT EXISTS webhooks (
            id              TEXT PRIMARY KEY,
            url             TEXT NOT NULL,
            events          TEXT NOT NULL,
            created_at      TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS job_webhooks (
            job_id          TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
            url             TEXT NOT NULL,
            fired           INTEGER DEFAULT 0
        );

        -- One row per live slither process that may own jobs in this home.
        -- `heartbeat_at` is the liveness signal: a process refreshes it on a
        -- timer, so a row that has stopped advancing belongs to a process that
        -- is gone. Without this, any second process attaching to the same
        -- SLITHER_HOME declared every in-flight job of the first one dead.
        CREATE TABLE IF NOT EXISTS job_owners (
            id              TEXT PRIMARY KEY,
            pid             INTEGER NOT NULL,
            started_at      TEXT NOT NULL,
            heartbeat_at    TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_jobs_domain ON jobs(domain);
        CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
        CREATE INDEX IF NOT EXISTS idx_jobs_created ON jobs(created_at);
        ",
    )?;

    // Migration: `jobs.owner_id` records which process is responsible for a
    // job. Databases created before ownership tracking existed do not have it,
    // and their rows keep a NULL owner (treated as unowned, i.e. recoverable).
    if !column_exists(conn, "jobs", "owner_id")? {
        conn.execute_batch("ALTER TABLE jobs ADD COLUMN owner_id TEXT;")?;
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_jobs_owner ON jobs(owner_id);")?;

    Ok(())
}

/// Whether `table` already has a column named `column`.
fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn open_db(db_path: &std::path::Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(db_path)?;
    init_db(&conn)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_db_creates_tables() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"jobs".to_string()));
        assert!(tables.contains(&"webhooks".to_string()));
        assert!(tables.contains(&"job_webhooks".to_string()));
        assert!(tables.contains(&"job_owners".to_string()));
    }

    #[test]
    fn test_init_db_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        init_db(&conn).unwrap(); // Should not error
    }

    /// A database written before ownership tracking existed must gain the
    /// column rather than failing to open — the migration runs on every start.
    #[test]
    fn owner_id_is_added_to_a_pre_existing_jobs_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE jobs (
                id TEXT PRIMARY KEY, type TEXT NOT NULL, status TEXT NOT NULL,
                domain TEXT NOT NULL, url TEXT NOT NULL, config TEXT NOT NULL,
                progress TEXT, result_summary TEXT, output_dir TEXT, error TEXT,
                created_at TEXT NOT NULL, started_at TEXT, completed_at TEXT
            );",
        )
        .unwrap();
        assert!(!column_exists(&conn, "jobs", "owner_id").unwrap());

        init_db(&conn).unwrap();

        assert!(column_exists(&conn, "jobs", "owner_id").unwrap());
        // And running it again must not try to add the column twice.
        init_db(&conn).unwrap();
    }
}
