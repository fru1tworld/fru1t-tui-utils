use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use rusqlite::Connection;

use crate::domain::{TilEntry, validate_content};
use crate::error::{Error, Result};

const DATABASE_VERSION: i64 = 1;

pub(crate) struct SqliteTilRepository {
    connection: Connection,
}

impl SqliteTilRepository {
    pub(crate) fn open_default() -> Result<Self> {
        let path = default_database_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::open(path)
    }

    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        let repository = Self { connection };
        repository.connection.busy_timeout(Duration::from_secs(5))?;
        repository
            .connection
            .execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        repository.migrate()?;
        Ok(repository)
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self::from_connection(Connection::open_in_memory().expect("인메모리 DB 생성"))
            .expect("인메모리 DB 마이그레이션")
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
            "CREATE TABLE entries (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                text        TEXT    NOT NULL,
                created_at  INTEGER NOT NULL
            );
            CREATE INDEX entries_created_at_idx ON entries(created_at);",
        )?;
        transaction.pragma_update(None, "user_version", DATABASE_VERSION)?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn data_version(&self) -> Result<i64> {
        Ok(self
            .connection
            .query_row("PRAGMA data_version", [], |row| row.get(0))?)
    }

    pub(crate) fn entries_on(&self, date: NaiveDate) -> Result<Vec<TilEntry>> {
        let (start, end) = local_day_bounds(date)?;
        let mut statement = self.connection.prepare(
            "SELECT id, text, created_at
             FROM entries
             WHERE created_at >= ?1 AND created_at < ?2
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = statement.query_map((start, end), til_entry_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn entries_between(
        &self,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Result<Vec<TilEntry>> {
        let (start_timestamp, _) = local_day_bounds(start_date)?;
        let (_, end_timestamp) = local_day_bounds(end_date)?;
        let mut statement = self.connection.prepare(
            "SELECT id, text, created_at
             FROM entries
             WHERE created_at >= ?1 AND created_at < ?2
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = statement.query_map((start_timestamp, end_timestamp), til_entry_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn create_on(&self, date: NaiveDate, content: &str) -> Result<TilEntry> {
        let content = validate_content(content)?;
        let now = Local::now();
        let recorded_at = if date == now.date_naive() {
            now.timestamp()
        } else {
            let time = now.time().with_nanosecond(0).unwrap_or(NaiveTime::MIN);
            local_timestamp(date.and_time(time))?
        };
        self.insert_at(content, recorded_at)
    }

    fn insert_at(&self, content: &str, recorded_at: i64) -> Result<TilEntry> {
        self.connection.execute(
            "INSERT INTO entries (text, created_at) VALUES (?1, ?2)",
            (content, recorded_at),
        )?;
        Ok(TilEntry {
            id: self.connection.last_insert_rowid(),
            content: content.to_owned(),
            recorded_at,
        })
    }

    pub(crate) fn restore(&self, entry: &TilEntry) -> Result<()> {
        self.connection.execute(
            "INSERT INTO entries (id, text, created_at) VALUES (?1, ?2, ?3)",
            (entry.id, &entry.content, entry.recorded_at),
        )?;
        Ok(())
    }

    pub(crate) fn update_content(&self, id: i64, content: &str) -> Result<()> {
        let content = validate_content(content)?;
        let changed = self
            .connection
            .execute("UPDATE entries SET text = ?1 WHERE id = ?2", (content, id))?;
        ensure_entry_changed(id, changed)
    }

    pub(crate) fn move_to_date(&self, id: i64, date: NaiveDate) -> Result<i64> {
        let entry = self.find(id)?;
        let local_time = chrono::DateTime::from_timestamp(entry.recorded_at, 0)
            .map(|date_time| date_time.with_timezone(&Local).time())
            .unwrap_or(NaiveTime::MIN);
        let moved_at = local_timestamp(date.and_time(local_time))?;
        self.set_recorded_at(id, moved_at)?;
        Ok(entry.recorded_at)
    }

    pub(crate) fn set_recorded_at(&self, id: i64, recorded_at: i64) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE entries SET created_at = ?1 WHERE id = ?2",
            (recorded_at, id),
        )?;
        ensure_entry_changed(id, changed)
    }

    pub(crate) fn delete(&self, id: i64) -> Result<TilEntry> {
        let entry = self.find(id)?;
        let changed_rows = self
            .connection
            .execute("DELETE FROM entries WHERE id = ?1", [id])?;
        ensure_entry_changed(id, changed_rows)?;
        Ok(entry)
    }

    fn find(&self, id: i64) -> Result<TilEntry> {
        self.connection
            .query_row(
                "SELECT id, text, created_at FROM entries WHERE id = ?1",
                [id],
                til_entry_from_row,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Error::EntryNotFound(id),
                other => Error::Db(other),
            })
    }
}

fn ensure_entry_changed(id: i64, changed_rows: usize) -> Result<()> {
    if changed_rows == 0 {
        return Err(Error::EntryNotFound(id));
    }
    Ok(())
}

fn til_entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TilEntry> {
    Ok(TilEntry {
        id: row.get(0)?,
        content: row.get(1)?,
        recorded_at: row.get(2)?,
    })
}

fn local_timestamp(date_time: NaiveDateTime) -> Result<i64> {
    Local
        .from_local_datetime(&date_time)
        .single()
        .map(|date_time| date_time.timestamp())
        .ok_or_else(|| Error::InvalidInput("로컬 시각으로 변환할 수 없습니다".into()))
}

fn local_day_bounds(date: NaiveDate) -> Result<(i64, i64)> {
    let start = local_timestamp(date.and_time(NaiveTime::MIN))?;
    let next_date = date
        .succ_opt()
        .ok_or_else(|| Error::InvalidInput("다음 날짜를 계산할 수 없습니다".into()))?;
    let end = local_timestamp(next_date.and_time(NaiveTime::MIN))?;
    Ok((start, end))
}

fn default_database_path() -> PathBuf {
    std::env::var_os("TIL_TUI_DB").map_or_else(
        || {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("til-tui")
                .join("til.db")
        },
        PathBuf::from,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> SqliteTilRepository {
        SqliteTilRepository::in_memory()
    }

    #[test]
    fn initial_database_is_empty() {
        let repository = repository();
        let date = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();

        assert!(repository.entries_on(date).unwrap().is_empty());
    }

    #[test]
    fn create_update_move_and_delete_round_trip() {
        let repository = repository();
        let source_date = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        let target_date = NaiveDate::from_ymd_opt(2026, 8, 30).unwrap();
        let entry = repository.create_on(source_date, "새 기록").unwrap();

        repository.update_content(entry.id, "수정한 기록").unwrap();
        let original_timestamp = repository.move_to_date(entry.id, target_date).unwrap();
        assert_eq!(
            repository.entries_on(target_date).unwrap()[0].content,
            "수정한 기록"
        );

        repository
            .set_recorded_at(entry.id, original_timestamp)
            .unwrap();
        let deleted = repository.delete(entry.id).unwrap();
        assert_eq!(deleted.id, entry.id);
        repository.restore(&deleted).unwrap();
        assert!(
            repository
                .entries_on(source_date)
                .unwrap()
                .iter()
                .any(|candidate| candidate.id == entry.id)
        );
    }

    #[test]
    fn empty_content_is_rejected_at_the_repository_boundary() {
        let repository = repository();
        let date = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();

        assert!(matches!(
            repository.create_on(date, "  "),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn entries_between_returns_only_the_inclusive_date_range() {
        let repository = repository();
        let monday = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        let sunday = NaiveDate::from_ymd_opt(2026, 9, 6).unwrap();
        repository
            .create_on(monday - chrono::Duration::days(1), "이전 주")
            .unwrap();
        repository.create_on(monday, "월요일").unwrap();
        repository.create_on(sunday, "일요일").unwrap();
        repository
            .create_on(sunday + chrono::Duration::days(1), "다음 주")
            .unwrap();

        let entries = repository.entries_between(monday, sunday).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.content.as_str())
                .collect::<Vec<_>>(),
            ["월요일", "일요일"]
        );
    }
}
