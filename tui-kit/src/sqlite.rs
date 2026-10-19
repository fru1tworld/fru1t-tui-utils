use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, Result};

/// One schema step; the step at index `n` upgrades `user_version` to `n + 1`.
pub type Migration = fn(&Connection) -> Result<()>;

/// Applies the settings every app uses so the TUI and CLI can share one file.
pub fn configure(connection: &Connection) -> Result<()> {
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
}

/// Runs every migration newer than the database's `user_version`, each in its own transaction.
pub fn migrate(connection: &Connection, migrations: &[Migration]) -> Result<()> {
    let current: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    for (version, migration) in (1_i64..).zip(migrations) {
        if current < version {
            let transaction = connection.unchecked_transaction()?;
            migration(&transaction)?;
            transaction.pragma_update(None, "user_version", version)?;
            transaction.commit()?;
        }
    }
    Ok(())
}

/// Changes whenever another connection commits, which lets the TUI pick up CLI writes.
pub fn data_version(connection: &Connection) -> Result<i64> {
    connection.query_row("PRAGMA data_version", [], |row| row.get(0))
}

/// `$env_var` when set, otherwise `<data dir>/<app>/<file>`.
pub fn database_path(env_var: &str, app: &str, file: &str) -> PathBuf {
    std::env::var_os(env_var).map_or_else(
        || {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(app)
                .join(file)
        },
        PathBuf::from,
    )
}

pub fn create_parent_dir(path: &Path) -> std::io::Result<()> {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => std::fs::create_dir_all(dir),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_table(connection: &Connection) -> Result<()> {
        connection.execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY)")
    }

    fn add_column(connection: &Connection) -> Result<()> {
        connection.execute_batch("ALTER TABLE items ADD COLUMN name TEXT")
    }

    #[test]
    fn migrations_run_once_in_order() {
        let connection = Connection::open_in_memory().unwrap();
        configure(&connection).unwrap();
        migrate(&connection, &[create_table]).unwrap();
        migrate(&connection, &[create_table, add_column]).unwrap();
        migrate(&connection, &[create_table, add_column]).unwrap();

        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
        connection
            .execute("INSERT INTO items (name) VALUES ('a')", [])
            .unwrap();
    }
}
