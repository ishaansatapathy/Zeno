use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::comments::repo::CommentRow;
use crate::error::{ApiError, FieldError};

pub const KINDS: &[&str] = &["comment", "reaction"];
/// The reactions on offer. A short fixed list: nothing a person types is shown as a reaction.
pub const EMOJIS: &[&str] = &["👍", "❤️", "😂", "🔥", "👏", "😮"];
pub const MAX_BODY_CHARS: usize = 1000;
/// Nothing is pinned past the longest video the platform takes (6 hours), plus a second of slack.
pub const MAX_AT_MS: i64 = 6 * 3600 * 1000 + 1000;
pub const DEFAULT_PAGE: usize = 100;
pub const MAX_PAGE: usize = 200;

/// What a person leaves. A comment has `body`; a reaction has `emoji` and `at_ms`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCommentRequest {
    /// Client-generated id for retrying a post whose response was lost: the same id with the same content
    /// returns the same comment instead of a second one.
    pub id: Option<Uuid>,
    pub kind: String,
    /// The words of a comment, 1 to 1000 characters.
    #[schema(min_length = 1, max_length = 1000)]
    pub body: Option<String>,
    /// One of the reactions on offer. Only for a reaction.
    pub emoji: Option<String>,
    /// Where in the video, in milliseconds. Optional for a comment (without it, it is about the whole video),
    /// required for a reaction. Not past the end of the video.
    #[schema(minimum = 0)]
    pub at_ms: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct NewComment {
    pub id: Option<Uuid>,
    pub kind: &'static str,
    pub body: Option<String>,
    pub emoji: Option<&'static str>,
    pub at_ms: Option<i32>,
}

fn clean(body: &str) -> Option<String> {
    let body = body.trim();
    // Words and line breaks, and nothing that is not meant to be seen: no control characters, and none of the
    // ones that reorder the text around them (they can make a comment read as something else on screen).
    let hidden = |c: char| {
        (c.is_control() && c != '\n')
            || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    };
    (!body.is_empty() && !body.chars().any(hidden)).then(|| body.to_owned())
}

impl CreateCommentRequest {
    /// `duration_ms`: how long the video is, if known; nothing may be pinned past its end.
    pub fn validate(self, duration_ms: Option<i64>) -> Result<NewComment, ApiError> {
        let mut errors = Vec::new();
        let kind = KINDS
            .iter()
            .copied()
            .find(|k| *k == self.kind.trim().to_ascii_lowercase());
        if kind.is_none() {
            errors.push(FieldError::new(
                "kind",
                format!("Choose one of: {}.", KINDS.join(", ")),
            ));
        }
        let limit = duration_ms.map_or(MAX_AT_MS, |d| (d + 1000).min(MAX_AT_MS));
        let at_ms = match self.at_ms {
            Some(at) if !(0..=limit).contains(&at) => {
                errors.push(FieldError::new("at_ms", "That moment is not in the video."));
                None
            }
            other => other.and_then(|at| i32::try_from(at).ok()),
        };

        let (mut body, mut emoji) = (None, None);
        match kind {
            Some("comment") => {
                match self.body.as_deref().map(|raw| (raw, clean(raw))) {
                    Some((_, Some(text))) if text.chars().count() <= MAX_BODY_CHARS => {
                        body = Some(text)
                    }
                    Some((_, Some(_))) => errors.push(FieldError::new(
                        "body",
                        format!("At most {MAX_BODY_CHARS} characters."),
                    )),
                    Some((_, None)) => errors.push(FieldError::new(
                        "body",
                        "Write something (no control characters).",
                    )),
                    None => errors.push(FieldError::new("body", "A comment needs words.")),
                }
                if self.emoji.is_some() {
                    errors.push(FieldError::new("emoji", "Only a reaction has an emoji."));
                }
            }
            Some("reaction") => {
                match self
                    .emoji
                    .as_deref()
                    .and_then(|raw| EMOJIS.iter().copied().find(|e| *e == raw.trim()))
                {
                    Some(found) => emoji = Some(found),
                    None => errors.push(FieldError::new(
                        "emoji",
                        format!("Choose one of: {}.", EMOJIS.join(" ")),
                    )),
                }
                if self.body.is_some() {
                    errors.push(FieldError::new(
                        "body",
                        "A reaction has no words. Leave a comment for that.",
                    ));
                }
                if self.at_ms.is_none() {
                    errors.push(FieldError::new(
                        "at_ms",
                        "A reaction belongs to a moment of the video.",
                    ));
                }
            }
            _ => {}
        }
        if !errors.is_empty() {
            return Err(ApiError::validation(errors));
        }
        Ok(NewComment {
            id: self.id,
            kind: kind.unwrap_or("comment"),
            body,
            emoji,
            at_ms,
        })
    }
}

#[derive(Debug, Deserialize, Default, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct ListCommentsQuery {
    /// From the previous page's `next_cursor`.
    pub cursor: Option<String>,
    /// How many, 1 to 200 (default 100).
    pub limit: Option<usize>,
    /// For a link with a password: the token of a viewing that unlocked it (the `t` of its playlist address).
    pub t: Option<String>,
}

impl ListCommentsQuery {
    pub fn page_size(&self) -> Result<usize, ApiError> {
        match self.limit {
            None => Ok(DEFAULT_PAGE),
            Some(limit) if (1..=MAX_PAGE).contains(&limit) => Ok(limit),
            Some(_) => Err(ApiError::validation(vec![FieldError::new(
                "limit",
                format!("Between 1 and {MAX_PAGE}."),
            )])),
        }
    }
}

/// Only for the write routes: the token of a viewing, for a link with a password.
#[derive(Debug, Deserialize, Default, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct TokenQuery {
    /// For a link with a password: the token of a viewing that unlocked it.
    pub t: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommentAuthor {
    /// The name on their account.
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommentResponse {
    pub id: Uuid,
    pub kind: String,
    pub body: Option<String>,
    pub emoji: Option<String>,
    /// Where in the video, in milliseconds.
    pub at_ms: Option<i32>,
    pub author: CommentAuthor,
    pub created_at: DateTime<Utc>,
    /// Left by the person asking.
    pub mine: bool,
    /// Whether the person asking may remove it: its author, or the owner of the recording.
    pub can_delete: bool,
}

impl CommentResponse {
    pub fn of(row: CommentRow, viewer: Option<Uuid>, owner: Uuid) -> Self {
        let mine = viewer == Some(row.author_id);
        Self {
            can_delete: mine || viewer == Some(owner),
            mine,
            id: row.id,
            kind: row.kind,
            body: row.body,
            emoji: row.emoji,
            at_ms: row.at_ms,
            author: CommentAuthor {
                name: row.author_name,
            },
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommentEnvelope {
    pub comment: CommentResponse,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommentListEnvelope {
    /// Newest first.
    pub comments: Vec<CommentResponse>,
    /// Pass as `cursor` for the next page; absent on the last.
    pub next_cursor: Option<String>,
    /// When off, the owner has closed the conversation: `comments` is empty and nobody can add to it.
    pub comments_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        kind: &str,
        body: Option<&str>,
        emoji: Option<&str>,
        at_ms: Option<i64>,
    ) -> CreateCommentRequest {
        CreateCommentRequest {
            id: None,
            kind: kind.into(),
            body: body.map(str::to_owned),
            emoji: emoji.map(str::to_owned),
            at_ms,
        }
    }

    fn fields(result: Result<NewComment, ApiError>) -> Vec<&'static str> {
        match result.unwrap_err() {
            ApiError::Validation(list) => list.iter().map(|f| f.field).collect(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_comment_is_trimmed_words_with_an_optional_moment() {
        let ok = request(
            "Comment",
            Some("  Nice part here \n and here  "),
            None,
            Some(12_500),
        )
        .validate(Some(60_000))
        .unwrap();
        assert_eq!(
            (ok.kind, ok.body.as_deref(), ok.emoji, ok.at_ms),
            (
                "comment",
                Some("Nice part here \n and here"),
                None,
                Some(12_500)
            )
        );
        assert_eq!(
            request("comment", Some("About all of it"), None, None)
                .validate(None)
                .unwrap()
                .at_ms,
            None
        );
    }

    #[test]
    fn a_reaction_is_one_listed_emoji_pinned_to_a_moment() {
        for emoji in EMOJIS {
            assert!(
                request("reaction", None, Some(emoji), Some(0))
                    .validate(Some(1000))
                    .is_ok(),
                "{emoji}"
            );
        }
        assert_eq!(
            fields(request("reaction", None, Some("🦄"), Some(5)).validate(None)),
            ["emoji"]
        );
        assert_eq!(
            fields(request("reaction", None, Some("👍"), None).validate(None)),
            ["at_ms"]
        );
        assert_eq!(
            fields(request("reaction", Some("words"), Some("👍"), Some(1)).validate(None)),
            ["body"]
        );
        assert_eq!(
            fields(request("comment", Some("hi"), Some("👍"), None).validate(None)),
            ["emoji"]
        );
    }

    #[test]
    fn words_must_be_something_and_not_too_long_and_not_control_characters() {
        assert_eq!(
            fields(request("comment", Some("   "), None, None).validate(None)),
            ["body"]
        );
        assert_eq!(
            fields(request("comment", None, None, None).validate(None)),
            ["body"]
        );
        assert_eq!(
            fields(request("comment", Some("a\u{0}b"), None, None).validate(None)),
            ["body"]
        );
        assert_eq!(
            fields(request("comment", Some("a\u{202e}b"), None, None).validate(None)),
            ["body"],
            "text direction overrides"
        );
        assert_eq!(
            fields(request("comment", Some("a\u{2067}b"), None, None).validate(None)),
            ["body"]
        );
        assert!(request("comment", Some("a family \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} and a heart \u{2764}\u{FE0F}"), None, None).validate(None).is_ok(), "joined emoji are words too");
        assert!(
            request("comment", Some(&"x".repeat(MAX_BODY_CHARS)), None, None)
                .validate(None)
                .is_ok()
        );
        assert_eq!(
            fields(
                request("comment", Some(&"x".repeat(MAX_BODY_CHARS + 1)), None, None)
                    .validate(None)
            ),
            ["body"]
        );
        // Characters, not bytes: a thousand of a multi-byte one is fine.
        assert!(
            request("comment", Some(&"é".repeat(MAX_BODY_CHARS)), None, None)
                .validate(None)
                .is_ok()
        );
    }

    #[test]
    fn nothing_is_pinned_before_the_start_or_past_the_end() {
        assert_eq!(
            fields(request("comment", Some("x"), None, Some(-1)).validate(Some(5_000))),
            ["at_ms"]
        );
        assert!(
            request("comment", Some("x"), None, Some(6_000))
                .validate(Some(5_000))
                .is_ok(),
            "a second of slack"
        );
        assert_eq!(
            fields(request("comment", Some("x"), None, Some(6_001)).validate(Some(5_000))),
            ["at_ms"]
        );
        assert_eq!(
            fields(request("comment", Some("x"), None, Some(MAX_AT_MS + 1)).validate(None)),
            ["at_ms"]
        );
    }

    #[test]
    fn an_unknown_kind_is_named_with_the_other_problems() {
        assert_eq!(
            fields(request("shout", Some("x"), None, Some(-5)).validate(None)),
            ["kind", "at_ms"]
        );
    }

    #[test]
    fn a_delete_button_is_for_the_author_and_the_owner_only() {
        let author = Uuid::new_v4();
        let owner = Uuid::new_v4();
        let row = || CommentRow {
            id: Uuid::new_v4(),
            recording_id: Uuid::new_v4(),
            author_id: author,
            author_name: "A".into(),
            kind: "comment".into(),
            body: Some("x".into()),
            emoji: None,
            at_ms: None,
            created_at: Utc::now(),
        };
        let flags = |viewer| {
            let r = CommentResponse::of(row(), viewer, owner);
            (r.mine, r.can_delete)
        };
        assert_eq!(flags(Some(author)), (true, true));
        assert_eq!(flags(Some(owner)), (false, true));
        assert_eq!(flags(Some(Uuid::new_v4())), (false, false));
        assert_eq!(flags(None), (false, false));
    }
}
