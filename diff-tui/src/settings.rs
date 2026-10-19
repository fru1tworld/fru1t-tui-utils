use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use tui_kit::sqlite;

use crate::appearance::Appearance;

const MIGRATIONS: &[sqlite::Migration] = &[create_settings];
const APPEARANCE_KEY: &str = "appearance";

pub struct Settings {
    connection: Connection,
}

impl Settings {
    pub fn open_default() -> Result<Self> {
        let path = sqlite::database_path("DIFF_TUI_DB", "diff-tui", "settings.db");
        sqlite::create_parent_dir(&path)
            .with_context(|| format!("설정 디렉터리를 만들 수 없습니다: {}", path.display()))?;
        Self::open(&path).with_context(|| format!("설정을 열 수 없습니다: {}", path.display()))
    }

    fn open(path: &Path) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        sqlite::configure(&connection)?;
        sqlite::migrate(&connection, MIGRATIONS)?;
        Ok(Self { connection })
    }

    #[cfg(test)]
    fn in_memory() -> Self {
        Self::from_connection(Connection::open_in_memory().unwrap()).unwrap()
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

fn create_settings(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE settings (
            key    TEXT PRIMARY KEY,
            value  TEXT NOT NULL
        );",
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
