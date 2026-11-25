//! The keys that lock a video's segments, and how often they change.
//!
//! A video has one secret (its content key, stored wrapped in `video_keys`). Older videos were encrypted
//! with it directly, so a player that had fetched it once could decrypt the whole video however long it
//! played. Newer videos are encrypted in *periods*: the segments of every `rotation_secs` of video are
//! locked with a key of their own, derived from the content key. A player is given one period's key at a
//! time, and the key server stops giving them when the viewing is no longer allowed, so a revoked viewer
//! can play on only as far as the keys they already hold (see `docs/backend.md`, "Revoking a viewing").
//!
//! The derived key is `HKDF-SHA256(content key, salt = video id, info = label || period)`. Nobody is ever
//! handed the content key itself, and a period's key does not reveal another's.

use ring::hkdf;
use uuid::Uuid;

use crate::keyvault::CONTENT_KEY_LEN;

/// What the worker writes into every `#EXT-X-KEY` of a video made before keys were rotated (the whole video
/// is one period, locked with the content key itself). The player never sees it: playback replaces it with
/// a per-viewer key address when the playlist is served.
pub const KEY_URI_PLACEHOLDER: &str = "zeno-key://video";
/// How long one period lasts for a video made now, in seconds of video. Short enough that a revoked viewer
/// gets little more than one period beyond what their player had already loaded; long enough that a viewer
/// asks for a key a couple of times a minute, not every few seconds.
pub const DEFAULT_ROTATION_SECS: i32 = 30;
/// What a stored `rotation_secs` may hold (the database checks the same).
pub const MIN_ROTATION_SECS: i32 = 6;
pub const MAX_ROTATION_SECS: i32 = 3600;
const LABEL: &[u8] = b"zeno/hls-period/v1";

/// Where a key tag's URI points, as the worker writes it: the legacy single key, or one period's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRef {
    /// The content key itself (a video made before rotation).
    Whole,
    Period(u32),
}

impl KeyRef {
    /// The placeholder the worker writes for this key.
    pub fn placeholder(self) -> String {
        match self {
            KeyRef::Whole => KEY_URI_PLACEHOLDER.to_owned(),
            KeyRef::Period(period) => format!("{KEY_URI_PLACEHOLDER}/{period}"),
        }
    }

    /// The reverse, strictly: only exactly what [`KeyRef::placeholder`] writes is accepted (no sign, no
    /// leading zeros, no spaces, nothing after the number).
    pub fn from_placeholder(uri: &str) -> Option<KeyRef> {
        if uri == KEY_URI_PLACEHOLDER {
            return Some(KeyRef::Whole);
        }
        let digits = uri.strip_prefix(KEY_URI_PLACEHOLDER)?.strip_prefix('/')?;
        let well_formed = !digits.is_empty()
            && digits.len() <= 9
            && digits.bytes().all(|b| b.is_ascii_digit())
            && (digits == "0" || !digits.starts_with('0'));
        well_formed
            .then(|| digits.parse().ok())
            .flatten()
            .map(KeyRef::Period)
    }
}

/// Which period a segment that starts `start_secs` into the video belongs to.
pub fn period_of(start_secs: f64, rotation_secs: i32) -> u32 {
    if !start_secs.is_finite() || start_secs <= 0.0 || rotation_secs <= 0 {
        return 0;
    }
    (start_secs / f64::from(rotation_secs)) as u32
}

/// The last period a video of `duration_secs` (rounded, as the database keeps it) can have. A request for a
/// later one is refused, so a viewer cannot ask for keys that lock nothing.
pub fn last_period(duration_secs: i32, rotation_secs: i32) -> u32 {
    // One second of slack: the duration is rounded and the last segment starts before the end.
    period_of(f64::from(duration_secs.max(0)) + 1.0, rotation_secs)
}

struct KeyLen;

impl hkdf::KeyType for KeyLen {
    fn len(&self) -> usize {
        CONTENT_KEY_LEN
    }
}

/// The key that locks `period`'s segments.
pub fn period_key(
    content_key: &[u8; CONTENT_KEY_LEN],
    video_id: Uuid,
    period: u32,
) -> [u8; CONTENT_KEY_LEN] {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, video_id.as_bytes()).extract(content_key);
    let period = period.to_be_bytes();
    let info = [LABEL, &period[..]];
    let mut out = [0u8; CONTENT_KEY_LEN];
    prk.expand(&info, KeyLen)
        .and_then(|okm| okm.fill(&mut out))
        .expect("16 bytes is within what HKDF-SHA256 can give");
    out
}

/// The key a playlist's `key` refers to, for a video with this content key.
pub fn key_for(
    content_key: &[u8; CONTENT_KEY_LEN],
    video_id: Uuid,
    key: KeyRef,
) -> [u8; CONTENT_KEY_LEN] {
    match key {
        KeyRef::Whole => *content_key,
        KeyRef::Period(period) => period_key(content_key, video_id, period),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 16] = *b"0123456789abcdef";

    #[test]
    fn a_period_key_is_not_the_content_key_and_differs_by_period_and_by_video() {
        let video = Uuid::new_v4();
        let k0 = period_key(&KEY, video, 0);
        let k1 = period_key(&KEY, video, 1);
        assert_ne!(k0, KEY);
        assert_ne!(k0, k1);
        assert_eq!(
            k0,
            period_key(&KEY, video, 0),
            "the same inputs, the same key"
        );
        assert_ne!(k0, period_key(&KEY, Uuid::new_v4(), 0));
        assert_ne!(k0, period_key(b"another content ", video, 0));
    }

    #[test]
    fn the_derivation_is_pinned() {
        // Changing it would make every video already made unplayable, so it is written down here.
        let video = Uuid::from_u128(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
        let key = period_key(&KEY, video, 7);
        assert_eq!(key.len(), 16);
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, PINNED_PERIOD_7);
    }

    #[test]
    fn placeholders_round_trip_and_nothing_else_is_accepted() {
        for key in [
            KeyRef::Whole,
            KeyRef::Period(0),
            KeyRef::Period(7),
            KeyRef::Period(999_999_999),
        ] {
            assert_eq!(KeyRef::from_placeholder(&key.placeholder()), Some(key));
        }
        for bad in [
            "",
            "zeno-key://video/",
            "zeno-key://video/-1",
            "zeno-key://video/+1",
            "zeno-key://video/01",
            "zeno-key://video/1 ",
            "zeno-key://video/1/2",
            "zeno-key://video/abc",
            "zeno-key://video/1000000000",
            "zeno-key://video/99999999999999999999",
            "zeno-key://videox",
            "https://evil/key",
            "zeno-key://other/1",
        ] {
            assert_eq!(KeyRef::from_placeholder(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_segment_belongs_to_the_period_it_starts_in() {
        assert_eq!(period_of(0.0, 30), 0);
        assert_eq!(period_of(29.9, 30), 0);
        assert_eq!(period_of(30.0, 30), 1);
        assert_eq!(period_of(61.0, 30), 2);
        assert_eq!(period_of(f64::NAN, 30), 0);
        assert_eq!(period_of(-5.0, 30), 0);
        assert_eq!(
            period_of(100.0, 0),
            0,
            "a rotation of nothing is one period"
        );
    }

    #[test]
    fn the_last_period_leaves_room_for_the_rounded_duration() {
        // 90 s of video in 6 s segments: the last segment starts at 84 s, in period 2.
        assert_eq!(last_period(90, 30), 3);
        assert!(last_period(90, 30) >= period_of(84.0, 30));
        assert_eq!(last_period(0, 30), 0);
        assert_eq!(last_period(-4, 30), 0);
    }

    /// Worked out separately with Python's `hmac` (HKDF-SHA256 extract and one expand block), not with this code.
    const PINNED_PERIOD_7: &str = "057fdfad6c87af73cf8ed1224e86e5d1";
}
