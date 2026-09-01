use chrono::{DateTime, Local, NaiveDate};
use serde::Serialize;

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TilEntry {
    pub(crate) id: i64,
    pub(crate) content: String,
    #[serde(serialize_with = "serialize_recorded_at")]
    pub(crate) recorded_at: i64,
}

impl TilEntry {
    pub(crate) fn time_label(&self) -> String {
        format_local_timestamp(self.recorded_at, "%H:%M")
    }

    pub(crate) fn recorded_date(&self) -> Option<NaiveDate> {
        DateTime::from_timestamp(self.recorded_at, 0)
            .map(|date_time| date_time.with_timezone(&Local).date_naive())
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

fn format_local_timestamp(timestamp: i64, format: &str) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .map(|date_time| date_time.with_timezone(&Local).format(format).to_string())
        .unwrap_or_else(|| "?".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_validation_trims_boundaries() {
        assert_eq!(validate_content("  배운 내용  ").unwrap(), "배운 내용");
    }

    #[test]
    fn content_validation_rejects_whitespace_only_input() {
        assert!(matches!(
            validate_content(" \t "),
            Err(Error::InvalidInput(_))
        ));
    }
}
