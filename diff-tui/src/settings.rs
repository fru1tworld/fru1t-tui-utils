use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::appearance::Appearance;

const DATABASE_VERSION: i64 = 1;
const APPEARANCE_KEY: &str = "appearance";

pub struct Settings {
    connection: Connection,
}

impl Settings {
    pub fn open_default() -> Result<Self> {
        let path = default_database_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Cannot create settings directory: {}", dir.display()))?;
        }
        Self::open(&path).with_context(|| format!("Cannot open settings: {}", path.display()))
    }

    fn open(path: &Path) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        let settings = Self { connection };
        settings.migrate()?;
        Ok(settings)
    }

    #[cfg(test)]
    fn in_memory() -> Self {
        Self::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn migrate(&self) -> Result<()> {
        let current_version: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if current_version >= DATABASE_VERSION {
            return Ok(());
        }
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute_batch(
            "CREATE TABLE settings (
                key    TEXT PRIMARY KEY,
                value  TEXT NOT NULL
            );",
        )?;
        transaction.pragma_update(None, "user_version", DATABASE_VERSION)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn appearance(&self) -> Result<Option<Appearance>> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [APPEARANCE_KEY],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.as_deref().and_then(Appearance::from_label))
    }

    pub fn save_appearance(&self, appearance: Appearance) -> Result<()> {
        self.connection.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [APPEARANCE_KEY, appearance.label()],
        )?;
        Ok(())
    }
}

fn default_database_path() -> PathBuf {
    std::env::var_os("DIFF_TUI_DB").map_or_else(
        || {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("diff-tui")
                .join("settings.db")
        },
        PathBuf::from,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_round_trips_and_overwrites() {
        let settings = Settings::in_memory();
        assert_eq!(settings.appearance().unwrap(), None);
        settings.save_appearance(Appearance::Light).unwrap();
        assert_eq!(settings.appearance().unwrap(), Some(Appearance::Light));
        settings.save_appearance(Appearance::Dark).unwrap();
        assert_eq!(settings.appearance().unwrap(), Some(Appearance::Dark));
    }
}
