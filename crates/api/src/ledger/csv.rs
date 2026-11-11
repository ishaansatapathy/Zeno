//! CSV for spreadsheets, written so that opening a file is never dangerous.
//!
//! Two things matter. A field with a comma, quote or line break is quoted (quotes doubled), per
//! RFC 4180. And a *text* field that begins with `=`, `+`, `-`, `@`, a tab or a carriage return is
//! prefixed with an apostrophe, because spreadsheet programs run such a cell as a formula: a
//! course title is chosen by a creator, and an accountant who opens the export should not run it.
//! Numeric columns are written as plain numbers and are not touched.

use crate::ledger::repo::EntryRow;

pub const HEADER: &str = "date_utc,kind,order_id,course,amount_paise,amount_inr,note,entry_id";

/// A text cell, made safe.
pub fn text_cell(value: &str) -> String {
    let neutralised = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    if neutralised.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", neutralised.replace('"', "\"\""))
    } else {
        neutralised
    }
}

/// `-12345` paise as `-123.45`.
pub fn inr(paise: i64) -> String {
    let sign = if paise < 0 { "-" } else { "" };
    let paise = paise.unsigned_abs();
    format!("{sign}{}.{:02}", paise / 100, paise % 100)
}

pub fn row(entry: &EntryRow) -> String {
    format!(
        "{},{},{},{},{},{},{},{}\n",
        entry
            .created_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        text_cell(&entry.kind),
        entry.order_id.map(|id| id.to_string()).unwrap_or_default(),
        text_cell(entry.course_title.as_deref().unwrap_or("")),
        entry.amount_paise,
        inr(entry.amount_paise),
        text_cell(entry.memo.as_deref().unwrap_or("")),
        entry.id,
    )
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn plain_text_is_left_alone() {
        assert_eq!(text_cell("Physics for JEE"), "Physics for JEE");
        assert_eq!(text_cell(""), "");
    }

    #[test]
    fn separators_and_quotes_are_quoted() {
        assert_eq!(text_cell("Maths, Class 10"), "\"Maths, Class 10\"");
        assert_eq!(
            text_cell("The \"best\" course"),
            "\"The \"\"best\"\" course\""
        );
        assert_eq!(text_cell("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn anything_a_spreadsheet_would_run_is_neutralised() {
        for hostile in ["=1+1", "+1+1", "-1+1", "@SUM(A1)", "\t=1", "\r=1"] {
            let cell = text_cell(hostile);
            assert!(
                cell.starts_with('\'') || cell.starts_with("\"'"),
                "{hostile:?} -> {cell:?}"
            );
        }
        // The classic payload, with a comma in it, is both neutralised and quoted.
        assert_eq!(
            text_cell("=HYPERLINK(\"http://evil\",\"x\")"),
            "\"'=HYPERLINK(\"\"http://evil\"\",\"\"x\"\")\""
        );
        // A dash inside text is not a formula.
        assert_eq!(text_cell("Class 9 - 10"), "Class 9 - 10");
    }

    #[test]
    fn rupees_print_with_paise_and_sign() {
        assert_eq!(inr(49_900), "499.00");
        assert_eq!(inr(-1_178), "-11.78");
        assert_eq!(inr(5), "0.05");
        assert_eq!(inr(-5), "-0.05");
        assert_eq!(inr(0), "0.00");
        assert_eq!(inr(i64::MIN), "-92233720368547758.08");
    }

    #[test]
    fn a_row_has_the_columns_of_the_header() {
        let entry = EntryRow {
            id: Uuid::nil(),
            journal_id: Uuid::nil(),
            kind: "sale".into(),
            amount_paise: 49_900,
            order_id: Some(Uuid::nil()),
            course_title: Some("=Evil, \"Course\"".into()),
            memo: None,
            created_at: chrono::Utc
                .with_ymd_and_hms(2026, 11, 10, 9, 30, 0)
                .unwrap(),
        };
        let line = row(&entry);
        assert!(line.ends_with('\n'));
        assert!(line.starts_with("2026-11-10T09:30:00Z,sale,"));
        assert!(line.contains(",\"'=Evil, \"\"Course\"\"\","), "{line}");
        assert!(line.contains(",49900,499.00,"));
        assert_eq!(HEADER.split(',').count(), 8);
    }
}
