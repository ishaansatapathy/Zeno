use chrono::{Duration, NaiveDate, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::auth::extractor::AuthUser;
use crate::error::{ApiError, FieldError};
use crate::recordings::service as recordings;
use crate::state::AppState;
use crate::views::repo;

pub const DEFAULT_DAYS: u32 = 30;
pub const MAX_DAYS: u32 = 90;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Day {
    pub day: NaiveDate,
    pub views: i64,
    pub viewers: i64,
}

#[derive(Debug, Serialize)]
pub struct Views {
    /// All-time views that have been flushed. New views reach it within one flush interval.
    pub total: i64,
    /// Oldest first, one entry per day up to and including today (UTC), zero where nobody watched.
    pub days: Vec<Day>,
}

#[derive(Debug, Serialize)]
pub struct ViewsEnvelope {
    pub views: Views,
}

pub fn validate_days(days: Option<u32>) -> Result<u32, ApiError> {
    let days = days.unwrap_or(DEFAULT_DAYS);
    if (1..=MAX_DAYS).contains(&days) {
        Ok(days)
    } else {
        Err(ApiError::validation(vec![FieldError::new(
            "days",
            format!("Must be between 1 and {MAX_DAYS}."),
        )]))
    }
}

/// Lays the rows that exist over every day of the window, so a chart needs no gap-filling.
pub fn fill(today: NaiveDate, days: u32, rows: &[repo::DayRow]) -> Vec<Day> {
    (0..days)
        .rev()
        .map(|back| {
            let day = today - Duration::days(i64::from(back));
            let found = rows.iter().find(|row| row.day == day);
            Day {
                day,
                views: found.map_or(0, |row| row.views),
                viewers: found.map_or(0, |row| row.viewers),
            }
        })
        .collect()
}

/// The owner's view numbers for one recording.
pub async fn show(
    state: &AppState,
    user: &AuthUser,
    recording: Uuid,
    days: u32,
) -> Result<Views, ApiError> {
    // Role and ownership: a recording that is not the caller's is "not found".
    let row = recordings::get(state, user, recording).await?;
    let today = Utc::now().date_naive();
    let since = today - Duration::days(i64::from(days) - 1);
    let rows = repo::days_since(&state.db, recording, since).await?;
    Ok(Views {
        total: row.view_count,
        days: fill(today, days, &rows),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, n).unwrap()
    }

    #[test]
    fn the_window_is_filled_with_zeros_oldest_first() {
        let rows = vec![repo::DayRow {
            day: day(18),
            views: 7,
            viewers: 5,
        }];
        let filled = fill(day(19), 3, &rows);
        assert_eq!(
            filled,
            vec![
                Day {
                    day: day(17),
                    views: 0,
                    viewers: 0
                },
                Day {
                    day: day(18),
                    views: 7,
                    viewers: 5
                },
                Day {
                    day: day(19),
                    views: 0,
                    viewers: 0
                },
            ]
        );
    }

    #[test]
    fn days_are_bounded() {
        assert_eq!(validate_days(None).unwrap(), DEFAULT_DAYS);
        assert!(validate_days(Some(0)).is_err());
        assert!(validate_days(Some(1)).is_ok());
        assert!(validate_days(Some(MAX_DAYS)).is_ok());
        assert!(validate_days(Some(MAX_DAYS + 1)).is_err());
    }
}
