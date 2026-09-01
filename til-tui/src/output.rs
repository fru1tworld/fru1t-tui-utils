use chrono::NaiveDate;

use crate::domain::TilEntry;

pub(crate) fn format_day_as_markdown(date: NaiveDate, entries: &[TilEntry]) -> String {
    let mut out = format!("- {date}\n");
    if entries.is_empty() {
        out.push_str("  - (기록 없음)\n");
        return out;
    }

    for entry in entries {
        let mut lines = entry.content.lines();
        let first = lines.next().unwrap_or("");
        let first = first.strip_prefix("- ").unwrap_or(first);
        out.push_str(&format!("  - {first}\n"));
        for line in lines {
            out.push_str("    ");
            out.push_str(line.trim_end_matches('\r'));
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_is_formatted_as_a_nested_markdown_list() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        let entries = vec![
            TilEntry {
                id: 1,
                content: "첫 줄".into(),
                recorded_at: 0,
            },
            TilEntry {
                id: 2,
                content: "둘째 기록\n세부 내용\n- 확인할 점".into(),
                recorded_at: 60,
            },
        ];

        assert_eq!(
            format_day_as_markdown(date, &entries),
            concat!(
                "- 2026-08-31\n",
                "  - 첫 줄\n",
                "  - 둘째 기록\n",
                "    세부 내용\n",
                "    - 확인할 점\n",
            )
        );
    }
}
