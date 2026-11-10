//! A small PDF writer for invoices, with no dependencies.
//!
//! It uses the two built-in Helvetica fonts, which every PDF viewer has, so nothing is embedded and
//! a file is a few kilobytes. The price of that: the built-in fonts cover Latin-1 only. Text in
//! other scripts (Devanagari, Tamil...) cannot be drawn, and the rupee sign is written as `Rs`.
//! [`can_render`] says whether an invoice can be drawn faithfully, and the download is refused
//! when it cannot: an invoice that shows a buyer's name as `?????` is worse than none. Embedding a
//! Unicode font is the way to lift that; the JSON form of the invoice always has the exact text.
//! Text that is merely long is never cut: it wraps, and spills onto further pages.
//!
//! Everything that goes into a string is escaped, and every length and offset in the file is
//! computed, not guessed: a viewer that checks the cross-reference table finds it correct.

use std::fmt::Write as _;

use crate::invoices::dto::InvoiceData;

const PAGE_WIDTH: f32 = 595.0; // A4
const PAGE_HEIGHT: f32 = 842.0;
const LEFT: f32 = 56.0;
const AMOUNT_X: f32 = 440.0;
// Even a line of wide Helvetica glyphs at 11 pt stays left of the amount column.
const MAX_LINE_CHARS: usize = 35;
const BOTTOM: f32 = 56.0;
const PAGE_STEP: f32 = PAGE_HEIGHT - 112.0;

#[derive(Clone, Copy)]
enum Font {
    Regular,
    Bold,
}

struct Text {
    x: f32,
    y: f32,
    font: Font,
    size: f32,
    text: String,
}

/// A PDF string literal's contents for `text`: printable ASCII as is (with `\`, `(`, `)` escaped),
/// Latin-1 as octal escapes, the rupee sign as `Rs`, and anything else as `?`.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '(' | ')' => {
                out.push('\\');
                out.push(c);
            }
            ' '..='~' => out.push(c),
            '\u{20B9}' => out.push_str("Rs"),
            '\u{A0}'..='\u{FF}' => {
                let _ = write!(out, "\\{:03o}", c as u32);
            }
            _ => out.push('?'),
        }
    }
    out
}

fn rupees(paise: i64) -> String {
    let sign = if paise < 0 { "-" } else { "" };
    let paise = paise.unsigned_abs();
    format!("{sign}Rs {}.{:02}", paise / 100, paise % 100)
}

/// Splits text at spaces where possible, including long unbroken words. Invoice data must never
/// be silently cut just because it does not fit in the first line or page.
fn wrap(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.lines() {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let mut rest = word.chars().peekable();
            while rest.peek().is_some() {
                if current.chars().count() == MAX_LINE_CHARS {
                    lines.push(std::mem::take(&mut current));
                }
                let space = usize::from(!current.is_empty());
                let available = MAX_LINE_CHARS - current.chars().count() - space;
                if rest.clone().count() <= available {
                    if space != 0 {
                        current.push(' ');
                    }
                    current.extend(rest.by_ref());
                } else if current.is_empty() {
                    current.extend(rest.by_ref().take(MAX_LINE_CHARS));
                    lines.push(std::mem::take(&mut current));
                } else {
                    lines.push(std::mem::take(&mut current));
                }
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}

fn layout(invoice: &InvoiceData) -> Vec<Text> {
    let mut items = Vec::new();
    let mut y = PAGE_HEIGHT - 72.0;
    let put = |items: &mut Vec<Text>, x: f32, y: f32, font: Font, size: f32, text: String| {
        items.push(Text {
            x,
            y,
            font,
            size,
            text,
        });
    };

    put(&mut items, LEFT, y, Font::Bold, 22.0, "INVOICE".to_owned());
    y -= 28.0;
    put(
        &mut items,
        LEFT,
        y,
        Font::Regular,
        11.0,
        format!("Number: {}", invoice.number),
    );
    y -= 16.0;
    put(
        &mut items,
        LEFT,
        y,
        Font::Regular,
        11.0,
        // The date in India, the same clock the financial year is counted by.
        format!(
            "Date: {}",
            (invoice.issued_at + chrono::Duration::minutes(330)).format("%d %B %Y")
        ),
    );
    y -= 30.0;

    put(&mut items, LEFT, y, Font::Bold, 11.0, "From".to_owned());
    y -= 15.0;
    let mut seller = wrap(&invoice.seller.name);
    if let Some(address) = &invoice.seller.address {
        seller.extend(wrap(address));
    }
    for line in seller {
        put(&mut items, LEFT, y, Font::Regular, 11.0, line);
        y -= 14.0;
    }
    y -= 12.0;

    put(
        &mut items,
        LEFT,
        y,
        Font::Bold,
        11.0,
        "Billed to".to_owned(),
    );
    y -= 15.0;
    for line in wrap(&invoice.buyer.name) {
        put(&mut items, LEFT, y, Font::Regular, 11.0, line);
        y -= 14.0;
    }
    if let Some(email) = &invoice.buyer.email {
        for line in wrap(email) {
            put(&mut items, LEFT, y, Font::Regular, 11.0, line);
            y -= 14.0;
        }
    }
    y -= 18.0;

    put(
        &mut items,
        LEFT,
        y,
        Font::Bold,
        11.0,
        "Description".to_owned(),
    );
    put(
        &mut items,
        AMOUNT_X,
        y,
        Font::Bold,
        11.0,
        "Amount".to_owned(),
    );
    y -= 6.0;
    put(&mut items, LEFT, y, Font::Regular, 11.0, "-".repeat(92));
    y -= 16.0;

    for line in &invoice.lines {
        let description = wrap(&line.description);
        let first = description.first().cloned().unwrap_or_default();
        put(&mut items, LEFT, y, Font::Regular, 11.0, first);
        put(
            &mut items,
            AMOUNT_X,
            y,
            Font::Regular,
            11.0,
            rupees(line.list_price_paise),
        );
        y -= 14.0;
        for more in description.into_iter().skip(1) {
            put(&mut items, LEFT, y, Font::Regular, 11.0, more);
            y -= 14.0;
        }
        if line.discount_paise > 0 {
            let label = match &line.coupon_code {
                Some(code) => format!("Discount (coupon {code})"),
                None => "Discount".to_owned(),
            };
            put(&mut items, LEFT, y, Font::Regular, 11.0, label);
            put(
                &mut items,
                AMOUNT_X,
                y,
                Font::Regular,
                11.0,
                rupees(-line.discount_paise),
            );
            y -= 14.0;
        }
    }
    y -= 4.0;
    put(&mut items, LEFT, y, Font::Regular, 11.0, "-".repeat(92));
    y -= 18.0;
    put(&mut items, LEFT, y, Font::Bold, 13.0, "Total".to_owned());
    put(
        &mut items,
        AMOUNT_X,
        y,
        Font::Bold,
        13.0,
        rupees(invoice.total_paise),
    );
    y -= 36.0;
    put(
        &mut items,
        LEFT,
        y,
        Font::Regular,
        9.0,
        format!("Order {}", invoice.order_id),
    );
    y -= 12.0;
    put(
        &mut items,
        LEFT,
        y,
        Font::Regular,
        9.0,
        "Paid online. This invoice shows the amount charged; it carries no tax details.".to_owned(),
    );
    items
}

/// The built-in PDF fonts cannot show every Unicode script. Refuse an inaccurate download
/// instead of silently replacing a legal name or address with question marks.
pub fn can_render(invoice: &InvoiceData) -> bool {
    layout(invoice).iter().all(|item| {
        item.text
            .chars()
            .all(|c| matches!(c, ' '..='~' | '\u{A0}'..='\u{FF}' | '\u{20B9}'))
    })
}

/// The invoice as a complete PDF file.
pub fn render(invoice: &InvoiceData) -> Vec<u8> {
    let mut pages = vec![String::new()];
    for item in layout(invoice) {
        let page = if item.y >= BOTTOM {
            0
        } else {
            ((BOTTOM - item.y) / PAGE_STEP).ceil() as usize
        };
        pages.resize_with(page + 1, String::new);
        let y = item.y + page as f32 * PAGE_STEP;
        let font = match item.font {
            Font::Regular => "F1",
            Font::Bold => "F2",
        };
        let _ = writeln!(
            pages[page],
            "BT /{font} {:.1} Tf {:.1} {:.1} Td ({}) Tj ET",
            item.size,
            item.x,
            y,
            literal(&item.text)
        );
    }

    let count = pages.len();
    let regular_font = 3 + count * 2;
    let bold_font = regular_font + 1;
    let kids = (0..count)
        .map(|index| format!("{} 0 R", 3 + index * 2))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {count} >>"),
    ];
    for (index, content) in pages.into_iter().enumerate() {
        let stream_id = 4 + index * 2;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH} {PAGE_HEIGHT}] \
             /Resources << /Font << /F1 {regular_font} 0 R /F2 {bold_font} 0 R >> >> \
             /Contents {stream_id} 0 R >>"
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}endstream",
            content.len(),
            content
        ));
    }
    objects.push(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_owned(),
    );
    objects.push(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
            .to_owned(),
    );

    // The text is ASCII throughout (non-ASCII was escaped), so a byte offset is a char offset.
    let mut file = String::from("%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, body) in objects.iter().enumerate() {
        offsets.push(file.len());
        let _ = write!(file, "{} 0 obj\n{}\nendobj\n", index + 1, body);
    }
    let xref_at = file.len();
    let _ = write!(file, "xref\n0 {}\n", objects.len() + 1);
    file.push_str("0000000000 65535 f \n");
    for offset in offsets {
        let _ = writeln!(file, "{offset:010} 00000 n ");
    }
    let _ = write!(
        file,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
        objects.len() + 1
    );
    file.into_bytes()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use uuid::Uuid;

    use super::*;
    use crate::invoices::dto::{Buyer, Line, Seller};

    fn invoice() -> InvoiceData {
        InvoiceData {
            v: 1,
            number: "ZNO/2026-27/000042".into(),
            issued_at: chrono::Utc
                .with_ymd_and_hms(2026, 11, 10, 9, 30, 0)
                .unwrap(),
            order_id: Uuid::nil(),
            payment_ref: Some("pay_1".into()),
            seller: Seller {
                name: "Zeno Learning Pvt Ltd".into(),
                address: Some("12 MG Road\nPune 411001".into()),
            },
            buyer: Buyer {
                name: "Asha Rao".into(),
                email: Some("asha@example.in".into()),
            },
            lines: vec![Line {
                description: "Physics for JEE".into(),
                list_price_paise: 49_900,
                discount_paise: 5_000,
                coupon_code: Some("LAUNCH10".into()),
                amount_paise: 44_900,
            }],
            total_paise: 44_900,
            currency: "INR".into(),
            tax: None,
        }
    }

    #[test]
    fn text_is_escaped_and_encoded_for_the_built_in_fonts() {
        assert_eq!(literal("plain text 123"), "plain text 123");
        assert_eq!(literal(r"a\b (c)"), r"a\\b \(c\)");
        assert_eq!(literal("caf\u{e9}"), "caf\\351");
        assert_eq!(literal("\u{20B9}499"), "Rs499");
        assert_eq!(literal("\u{0939}\u{093f}\u{0902}\u{0926}\u{0940}"), "?????");
        assert_eq!(literal("tab\there\nnewline"), "tab?here?newline");
        assert!(literal("\u{1F642}").chars().all(|c| c == '?'));
    }

    #[test]
    fn unsupported_script_is_not_silently_printed_as_question_marks() {
        let mut hindi = invoice();
        hindi.buyer.name = "आशा राव".into();
        assert!(!can_render(&hindi));
        assert!(can_render(&invoice()));
    }

    #[test]
    fn amounts_print_in_rupees_and_paise() {
        assert_eq!(rupees(49_900), "Rs 499.00");
        assert_eq!(rupees(5), "Rs 0.05");
        assert_eq!(rupees(-5_000), "-Rs 50.00");
    }

    #[test]
    fn long_text_wraps_without_losing_characters() {
        let wrapped = wrap(&"word ".repeat(100));
        assert!(wrapped.len() > 4);
        assert!(wrapped.iter().all(|l| l.chars().count() <= MAX_LINE_CHARS));
        let giant = wrap(&"x".repeat(500));
        assert_eq!(giant.concat(), "x".repeat(500));
        assert!(giant.iter().all(|l| l.chars().count() <= MAX_LINE_CHARS));
        assert_eq!(wrap("a\nb"), vec!["a", "b"]);
        assert!(wrap("   ").is_empty());
    }

    #[test]
    fn overflowing_text_is_kept_on_following_pages() {
        let mut long = invoice();
        long.seller.address = Some(format!("{}END_OF_ADDRESS", "Address line\n".repeat(100)));
        let pdf = String::from_utf8(render(&long)).unwrap();
        assert!(pdf.contains("/Count 2") || pdf.contains("/Count 3"));
        assert!(pdf.contains("END_OF_ADDRESS"));
        let streams = pdf.matches("\nstream\n").count();
        assert!(streams > 1, "expected more than one page");
        assert_eq!(streams, pdf.matches("/Type /Page /Parent").count());
    }

    #[test]
    fn the_file_is_structurally_a_pdf_whose_offsets_are_right() {
        let bytes = render(&invoice());
        let text = String::from_utf8(bytes.clone()).expect("ASCII throughout");
        assert!(text.starts_with("%PDF-1.4\n"));
        assert!(text.ends_with("%%EOF\n"));

        // startxref points at the xref table.
        let start: usize = text
            .rsplit("startxref\n")
            .next()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            text[start..].starts_with("xref\n0 7\n"),
            "{}",
            &text[start..start + 20]
        );

        // Every xref entry points at its "n 0 obj".
        let table: Vec<&str> = text[start..].lines().skip(2).take(7).collect();
        assert_eq!(table[0], "0000000000 65535 f ");
        for (number, entry) in table.iter().enumerate().skip(1) {
            let offset: usize = entry[..10].parse().unwrap();
            assert!(
                text[offset..].starts_with(&format!("{number} 0 obj\n")),
                "object {number} is not at {offset}"
            );
        }

        // The declared stream length is the real one.
        let declared: usize = text
            .split("/Length ")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let (_, after_start) = text.split_once(" >>\nstream\n").unwrap();
        let (stream, _) = after_start.split_once("endstream").unwrap();
        let actual = stream.len();
        assert_eq!(declared, actual);
    }

    #[test]
    fn the_page_shows_what_was_issued() {
        let text = String::from_utf8(render(&invoice())).unwrap();
        for expected in [
            "ZNO/2026-27/000042",
            "10 November 2026",
            "Zeno Learning Pvt Ltd",
            "Pune 411001",
            "Asha Rao",
            "asha@example.in",
            "Physics for JEE",
            "Rs 499.00",
            "Discount \\(coupon LAUNCH10\\)",
            "-Rs 50.00",
            "Rs 449.00",
        ] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
    }

    #[test]
    fn hostile_text_cannot_break_out_of_its_string() {
        let mut hostile = invoice();
        hostile.buyer.name = "x) Tj ET /F1 99 Tf (injected".into();
        hostile.lines[0].description = "\\) \\( ))) ((( \u{0}".into();
        let text = String::from_utf8(render(&hostile)).unwrap();
        // Every line of the content stream is one text-showing operation whose string literal
        // ends exactly where we ended it: whatever the text said, nothing escaped from it.
        let (_, after_start) = text.split_once(" >>\nstream\n").unwrap();
        let (stream, _) = after_start.split_once("endstream").unwrap();
        for line in stream.lines() {
            let open = line.find('(').expect("a string literal");
            let mut chars = line[open + 1..].char_indices();
            let mut end = None;
            while let Some((index, c)) = chars.next() {
                match c {
                    '\\' => {
                        chars.next();
                    }
                    '(' => panic!("an unescaped ( in {line:?}"),
                    ')' => {
                        end = Some(open + 1 + index);
                        break;
                    }
                    _ => {}
                }
            }
            let end = end.expect("the literal is closed");
            assert!(line.starts_with("BT /F"), "{line:?}");
            assert_eq!(&line[end + 1..], " Tj ET", "{line:?}");
        }
    }
}
