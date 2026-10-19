use std::fmt;

use chrono::{DateTime, Local};
use rusqlite::Result;
use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::Serialize;

use crate::error::{Error, Result as AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct TodoId(pub(crate) i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ProjectId(pub(crate) i64);

impl fmt::Display for TodoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl ToSql for TodoId {
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl ToSql for ProjectId {
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for TodoId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(Self)
    }
}

impl FromSql for ProjectId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Project {
    pub(crate) id: ProjectId,
    pub(crate) name: String,
    pub(crate) position: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Todo {
    pub(crate) id: TodoId,
    pub(crate) text: String,
    pub(crate) created_at: i64,
    pub(crate) done: bool,
    pub(crate) parent_id: Option<TodoId>,
    #[serde(skip)]
    pub(crate) collapsed: bool,
    pub(crate) position: i64,
    pub(crate) project_id: ProjectId,
}

impl Todo {
    pub(crate) fn created_at_string(&self) -> String {
        DateTime::from_timestamp(self.created_at, 0).map_or_else(
            || "?".to_string(),
            |dt| {
                dt.with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            },
        )
    }
}

/// Trims a to-do's text and rejects it when nothing is left.
pub(crate) fn todo_text(text: &str) -> AppResult<&str> {
    non_empty(text, "내용을 입력하세요")
}

/// Trims a project name and rejects it when nothing is left.
pub(crate) fn project_name(name: &str) -> AppResult<&str> {
    non_empty(name, "이름을 입력하세요")
}

fn non_empty<'a>(text: &'a str, missing: &str) -> AppResult<&'a str> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::Invalid(missing.into()));
    }
    Ok(text)
}
