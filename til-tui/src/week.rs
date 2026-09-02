use chrono::{Datelike, Duration, NaiveDate, Weekday};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CalendarWeek {
    start: NaiveDate,
}

impl CalendarWeek {
    pub(crate) fn containing(date: NaiveDate) -> Self {
        let days_since_monday = i64::from(date.weekday().num_days_from_monday());
        Self {
            start: date - Duration::days(days_since_monday),
        }
    }

    pub(crate) fn start(self) -> NaiveDate {
        self.start
    }

    pub(crate) fn end(self) -> NaiveDate {
        self.start + Duration::days(6)
    }

    pub(crate) fn dates(self) -> impl Iterator<Item = NaiveDate> {
        (0..7).map(move |offset| self.start + Duration::days(offset))
    }

    pub(crate) fn shifted(self, weeks: i64) -> Self {
        Self {
            start: self.start + Duration::weeks(weeks),
        }
    }

    pub(crate) fn label(self) -> String {
        let thursday = self.start + Duration::days(3);
        let first = NaiveDate::from_ymd_opt(thursday.year(), thursday.month(), 1)
            .expect("유효한 연월의 첫날");
        let first_thursday_offset =
            (Weekday::Thu.num_days_from_monday() + 7 - first.weekday().num_days_from_monday()) % 7;
        let first_thursday = first + Duration::days(i64::from(first_thursday_offset));
        let first_week = Self::containing(first_thursday);
        let week_number = (self.start - first_week.start).num_weeks() + 1;

        format!(
            "{}년 {}월 {}주차",
            thursday.year(),
            thursday.month(),
            week_number
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn week_runs_from_monday_through_sunday() {
        let week = CalendarWeek::containing(date(2026, 9, 2));

        assert_eq!(week.start(), date(2026, 8, 31));
        assert_eq!(week.end(), date(2026, 9, 6));
        assert_eq!(week.dates().count(), 7);
    }

    #[test]
    fn thursday_decides_the_month_and_first_week() {
        assert_eq!(
            CalendarWeek::containing(date(2026, 8, 30)).label(),
            "2026년 8월 4주차"
        );
        assert_eq!(
            CalendarWeek::containing(date(2026, 8, 31)).label(),
            "2026년 9월 1주차"
        );
    }

    #[test]
    fn shifting_moves_exactly_one_calendar_week() {
        let week = CalendarWeek::containing(date(2026, 9, 2));

        assert_eq!(week.shifted(-1).start(), date(2026, 8, 24));
        assert_eq!(week.shifted(1).start(), date(2026, 9, 7));
    }
}
