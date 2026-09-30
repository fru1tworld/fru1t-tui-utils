use std::fmt;

use chrono::{DateTime, Local, NaiveDate, NaiveTime};
use serde::Serialize;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct EntryId(pub(crate) i64);

impl fmt::Display for EntryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TilEntry {
    pub(crate) id: EntryId,
    pub(crate) content: String,
    #[serde(serialize_with = "serialize_recorded_at")]
    pub(crate) recorded_at: i64,
}

impl TilEntry {
    pub(crate) fn time_label(&self) -> String {
        format_local_timestamp(self.recorded_at, "%H:%M")
    }

    pub(crate) fn recorded_date(&self) -> Option<NaiveDate> {
        local_date_time(self.recorded_at).map(|date_time| date_time.date_naive())
    }

    pub(crate) fn recorded_time(&self) -> Option<NaiveTime> {
        local_date_time(self.recorded_at).map(|date_time| date_time.time())
    }
}

pub(crate) fn validate_content(content: &str) -> Result<&str> {
    let content = content.trim();
    if content.is_empty() {
        return Err(Error::InvalidInput(
            "기록 내용은 비워 둘 수 없습니다".into(),
        ));
    }
    Ok(content)
}

fn serialize_recorded_at<S>(timestamp: &i64, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&format_local_timestamp(*timestamp, "%Y-%m-%dT%H:%M:%S%:z"))
}

fn local_date_time(timestamp: i64) -> Option<DateTime<Local>> {
    DateTime::from_timestamp(timestamp, 0).map(|date_time| date_time.with_timezone(&Local))
}

fn format_local_timestamp(timestamp: i64, format: &str) -> String {
    local_date_time(timestamp)
        .map(|date_time| date_time.format(format).to_string())
        .unwrap_or_else(|| "?".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_validation_trims_boundaries() {
        assert_eq!(validate_content("  배운 내용  ").unwrap(), "배운 내용");
    }
}
