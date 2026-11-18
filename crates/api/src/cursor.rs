//! Keyset pagination for lists ordered newest first.
//!
//! A page is "the rows older than this one", not "rows 40 to 60", so a page costs the same at any
//! depth and does not skip or repeat rows when new ones arrive while someone scrolls. The cursor
//! is the sort key of the last row on the page: its creation time and id (the id breaks ties
//! between rows created in the same instant).
//!
//! The cursor is opaque to clients and is not trusted: it is decoded strictly, and a forged one
//! can only ever change which page of the caller's own rows they see.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::ApiError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeCursor {
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}

fn invalid() -> ApiError {
    ApiError::bad_request("invalid_cursor", "The pagination cursor is invalid.")
}

impl TimeCursor {
    pub fn encode(self) -> String {
        URL_SAFE_NO_PAD.encode(format!("{}|{}", self.created_at.to_rfc3339(), self.id))
    }

    pub fn decode(raw: &str) -> Result<Self, ApiError> {
        let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
        let text = String::from_utf8(bytes).map_err(|_| invalid())?;
        let (created_at, id) = text.split_once('|').ok_or_else(invalid)?;
        Ok(Self {
            created_at: DateTime::parse_from_rfc3339(created_at)
                .map_err(|_| invalid())?
                .with_timezone(&Utc),
            id: Uuid::parse_str(id).map_err(|_| invalid())?,
        })
    }
}

/// Splits `rows` (fetched with one more row than the page size) into the page and, if the extra
/// row was there, the cursor for the next page.
pub fn paginate<T>(
    mut rows: Vec<T>,
    limit: usize,
    key: impl Fn(&T) -> TimeCursor,
) -> (Vec<T>, Option<TimeCursor>) {
    if rows.len() <= limit {
        return (rows, None);
    }
    rows.truncate(limit);
    let next = rows.last().map(key);
    (rows, next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor() -> TimeCursor {
        TimeCursor {
            created_at: DateTime::parse_from_rfc3339("2026-11-18T10:15:30.123456Z")
                .unwrap()
                .with_timezone(&Utc),
            id: Uuid::parse_str("7d2f1c1e-0000-4000-8000-000000000001").unwrap(),
        }
    }

    #[test]
    fn a_cursor_survives_the_round_trip_to_the_microsecond() {
        let original = cursor();
        assert_eq!(TimeCursor::decode(&original.encode()).unwrap(), original);
    }

    #[test]
    fn junk_is_refused() {
        for junk in [
            "",
            "!!!",
            "bm90LWEtY3Vyc29y",
            &URL_SAFE_NO_PAD.encode("2026-11-18|nope"),
        ] {
            assert!(TimeCursor::decode(junk).is_err(), "{junk:?}");
        }
        let no_id = URL_SAFE_NO_PAD.encode("2026-11-18T10:15:30Z|");
        assert!(TimeCursor::decode(&no_id).is_err());
        let bad_time = URL_SAFE_NO_PAD.encode(format!("yesterday|{}", cursor().id));
        assert!(TimeCursor::decode(&bad_time).is_err());
    }

    #[test]
    fn a_short_page_has_no_next_cursor() {
        let key = |_: &u32| cursor();
        assert_eq!(paginate(vec![1, 2], 2, key), (vec![1, 2], None));
        assert_eq!(paginate(Vec::<u32>::new(), 5, key), (vec![], None));
    }

    #[test]
    fn a_full_page_points_at_its_last_row() {
        let rows = vec![1u32, 2, 3];
        let (page, next) = paginate(rows, 2, |n| TimeCursor {
            created_at: cursor().created_at,
            id: Uuid::from_u128(u128::from(*n)),
        });
        assert_eq!(page, vec![1, 2]);
        assert_eq!(next.unwrap().id, Uuid::from_u128(2));
    }
}
