//! Locking a rendition's segments.
//!
//! ffmpeg writes a rendition in the clear, and this turns it into what is published: every segment
//! encrypted with AES-128-CBC, and the playlist told which key opens which segments. It is done here, not by
//! ffmpeg, because ffmpeg can lock a whole playlist with one key and nothing more, and a video's keys have to
//! change as it plays (see `zeno_core::segment_keys`).
//!
//! The playlist carries one `#EXT-X-KEY` for each period, written before the period's first segment. A
//! segment's IV is its media sequence number, which is what HLS says when a key tag gives none, so nothing
//! about the IV is stored. A segment belongs to the period it starts in, so a segment is never split between
//! two keys.

use std::path::Path;

use aes::Aes128;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockEncryptMut, KeyIvInit};
use uuid::Uuid;
use zeno_core::keyvault::CONTENT_KEY_LEN;
use zeno_core::segment_keys::{key_for, period_of, KeyRef};

type Encryptor = cbc::Encryptor<Aes128>;

/// How a video's segments are locked.
#[derive(Clone, Copy)]
pub struct Locking<'a> {
    pub video_id: Uuid,
    pub content_key: &'a [u8; CONTENT_KEY_LEN],
    /// Seconds of video per key, or `None` for the one key of a video made before keys were rotated.
    pub rotation_secs: Option<i32>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SealError {
    /// What ffmpeg wrote is not a playlist this can read (no segments, a name with a path in it, a key already in it).
    BadPlaylist(String),
}

impl std::fmt::Display for SealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SealError::BadPlaylist(why) => write!(f, "cannot lock this playlist: {why}"),
        }
    }
}

/// One segment of the playlist, and the key that locks it.
#[derive(Debug, PartialEq, Eq)]
pub struct Locked {
    pub uri: String,
    pub key: KeyRef,
    /// The segment's media sequence number: its IV.
    pub sequence: u64,
}

/// The playlist as it is published, and which key locks each of its segments.
#[derive(Debug, PartialEq, Eq)]
pub struct Sealed {
    pub playlist: String,
    pub segments: Vec<Locked>,
}

/// Rewrites a clear media playlist: a key tag at the start of every period and, from the media sequence
/// number, the plan for each segment. Pure: nothing is read or written here.
pub fn seal_playlist(clear: &str, rotation_secs: Option<i32>) -> Result<Sealed, SealError> {
    let bad = |why: &str| SealError::BadPlaylist(why.to_owned());
    if clear.lines().next().map(str::trim_end) != Some("#EXTM3U") {
        return Err(bad("it does not start with #EXTM3U"));
    }
    let mut sequence = 0u64;
    let mut out = String::with_capacity(clear.len() + 256);
    let mut segments: Vec<Locked> = Vec::new();
    let mut start_secs = 0.0f64;
    let mut pending: Option<f64> = None;
    let mut current: Option<KeyRef> = None;
    let mut first_segment_seen = false;

    for line in clear.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("#EXT-X-KEY") || line.starts_with("#EXT-X-SESSION-KEY") {
            return Err(bad("it is already encrypted"));
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            if first_segment_seen {
                return Err(bad("a media sequence number after the first segment"));
            }
            sequence = rest
                .trim()
                .parse()
                .map_err(|_| bad("a media sequence number that is not a number"))?;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let duration = rest
                .split(',')
                .next()
                .and_then(|d| d.trim().parse::<f64>().ok())
                .filter(|d| d.is_finite() && *d >= 0.0)
                .ok_or_else(|| bad("a segment duration that is not a number"))?;
            // The key tag goes before the #EXTINF of the first segment of the period.
            let wanted = match rotation_secs {
                Some(secs) => KeyRef::Period(period_of(start_secs, secs)),
                None => KeyRef::Whole,
            };
            if current != Some(wanted) {
                out.push_str(&format!(
                    "#EXT-X-KEY:METHOD=AES-128,URI=\"{}\"\n",
                    wanted.placeholder()
                ));
                current = Some(wanted);
            }
            pending = Some(duration);
            first_segment_seen = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if !line.starts_with('#') {
            let Some(duration) = pending.take() else {
                return Err(bad("a segment with no #EXTINF"));
            };
            if line.contains(['/', '\\']) || line.contains("..") || line.starts_with('.') {
                return Err(bad("a segment name with a path in it"));
            }
            let key = current.expect("a key tag is written before the first segment");
            segments.push(Locked {
                uri: line.to_owned(),
                key,
                sequence: sequence + segments.len() as u64,
            });
            start_secs += duration;
        }
        out.push_str(line);
        out.push('\n');
    }
    if pending.is_some() {
        return Err(bad("an #EXTINF with no segment after it"));
    }
    if segments.is_empty() {
        return Err(bad("no segments"));
    }
    Ok(Sealed {
        playlist: out,
        segments,
    })
}

/// AES-128-CBC with PKCS#7 padding, as HLS specifies; the IV is the sequence number, big-endian.
pub fn encrypt_segment(key: &[u8; 16], sequence: u64, plain: &[u8]) -> Vec<u8> {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&sequence.to_be_bytes());
    Encryptor::new(key.into(), &iv.into()).encrypt_padded_vec_mut::<Pkcs7>(plain)
}

/// Locks the rendition ffmpeg wrote into `dir` in place: each segment is replaced by its encrypted
/// self and the playlist by the sealed one. Segments are written before the playlist, so a rendition that
/// is found half done still has its clear playlist, which `publish` would never have been given.
pub async fn seal_rendition(
    dir: &Path,
    playlist_name: &str,
    locking: Locking<'_>,
) -> Result<(), SealFailure> {
    let path = dir.join(playlist_name);
    let clear = tokio::fs::read_to_string(&path)
        .await
        .map_err(SealFailure::Io)?;
    let sealed = seal_playlist(&clear, locking.rotation_secs).map_err(SealFailure::Playlist)?;
    // The keys of a rendition are the same few again and again: work each out once.
    let mut keys: Vec<(KeyRef, [u8; 16])> = Vec::new();
    for segment in &sealed.segments {
        let key = match keys.iter().find(|(known, _)| *known == segment.key) {
            Some((_, key)) => *key,
            None => {
                let key = key_for(locking.content_key, locking.video_id, segment.key);
                keys.push((segment.key, key));
                key
            }
        };
        let file = dir.join(&segment.uri);
        let plain = tokio::fs::read(&file).await.map_err(SealFailure::Io)?;
        let locked = encrypt_segment(&key, segment.sequence, &plain);
        tokio::fs::write(&file, locked)
            .await
            .map_err(SealFailure::Io)?;
    }
    tokio::fs::write(&path, sealed.playlist)
        .await
        .map_err(SealFailure::Io)?;
    Ok(())
}

#[derive(Debug)]
pub enum SealFailure {
    Io(std::io::Error),
    Playlist(SealError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use cbc::cipher::BlockDecryptMut;

    type Decryptor = cbc::Decryptor<Aes128>;

    /// What ffmpeg writes for `-hls_time 6` and no key: 6 s segments, the last one short.
    fn clear(segments: usize) -> String {
        let mut text = String::from(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n",
        );
        for n in 0..segments {
            let last = n + 1 == segments;
            text.push_str(&format!(
                "#EXTINF:{},\nseg_{n:05}.ts\n",
                if last { "3.500000" } else { "6.000000" }
            ));
        }
        text.push_str("#EXT-X-ENDLIST\n");
        text
    }

    fn key_lines(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|l| l.starts_with("#EXT-X-KEY"))
            .collect()
    }

    #[test]
    fn a_key_tag_opens_every_period_and_not_before_its_first_segment() {
        // 12 segments: starts at 0, 6, ..., 66 s. A 30 s period starts at 0, 30 (segment 5) and 60 (segment 10).
        let sealed = seal_playlist(&clear(12), Some(30)).unwrap();
        assert_eq!(
            key_lines(&sealed.playlist),
            [
                "#EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video/0\"",
                "#EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video/1\"",
                "#EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video/2\"",
            ]
        );
        let periods: Vec<KeyRef> = sealed.segments.iter().map(|s| s.key).collect();
        assert_eq!(periods[0], KeyRef::Period(0));
        assert_eq!(periods[4], KeyRef::Period(0));
        assert_eq!(periods[5], KeyRef::Period(1));
        assert_eq!(periods[9], KeyRef::Period(1));
        assert_eq!(periods[10], KeyRef::Period(2));
        assert_eq!(periods[11], KeyRef::Period(2));
        // A key tag always comes right before the #EXTINF of the first segment it covers.
        let lines: Vec<&str> = sealed.playlist.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            if line.starts_with("#EXT-X-KEY") {
                assert!(
                    lines[n + 1].starts_with("#EXTINF"),
                    "{line} is followed by {}",
                    lines[n + 1]
                );
            }
        }
        // Nothing else is changed.
        let without_keys: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|l| !l.starts_with("#EXT-X-KEY"))
            .collect();
        assert_eq!(without_keys.join("\n") + "\n", clear(12));
    }

    #[test]
    fn a_video_made_before_rotation_gets_one_key_for_all_of_it() {
        let sealed = seal_playlist(&clear(12), None).unwrap();
        assert_eq!(
            key_lines(&sealed.playlist),
            ["#EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video\""]
        );
        assert!(sealed.segments.iter().all(|s| s.key == KeyRef::Whole));
    }

    #[test]
    fn a_short_video_is_one_period() {
        let sealed = seal_playlist(&clear(3), Some(30)).unwrap();
        assert_eq!(key_lines(&sealed.playlist).len(), 1);
    }

    #[test]
    fn a_period_that_does_not_line_up_with_the_segments_still_never_splits_one() {
        // 6 s segments and a 10 s period: starts 0, 6 (period 0), 12 (1), 18 (1), 24 (2), 30 (3).
        let sealed = seal_playlist(&clear(6), Some(10)).unwrap();
        let periods: Vec<KeyRef> = sealed.segments.iter().map(|s| s.key).collect();
        assert_eq!(
            periods,
            [
                KeyRef::Period(0),
                KeyRef::Period(0),
                KeyRef::Period(1),
                KeyRef::Period(1),
                KeyRef::Period(2),
                KeyRef::Period(3)
            ]
        );
    }

    #[test]
    fn the_iv_is_the_media_sequence_number() {
        let text = clear(3).replace("MEDIA-SEQUENCE:0", "MEDIA-SEQUENCE:40");
        let sealed = seal_playlist(&text, Some(30)).unwrap();
        let sequences: Vec<u64> = sealed.segments.iter().map(|s| s.sequence).collect();
        assert_eq!(sequences, [40, 41, 42]);
    }

    #[test]
    fn what_is_locked_opens_with_the_key_and_the_iv_a_player_works_out() {
        let key = [7u8; 16];
        let plain: Vec<u8> = (0..=255u8).cycle().take(188 * 40 + 5).collect();
        let locked = encrypt_segment(&key, 42, &plain);
        assert_ne!(&locked[..plain.len().min(locked.len())], &plain[..]);
        assert_eq!(locked.len() % 16, 0);
        assert!(locked.len() > plain.len(), "PKCS#7 always pads");
        // What a player does: the IV is the sequence number as 128 bits, big-endian.
        let mut iv = [0u8; 16];
        iv[15] = 42;
        let opened = Decryptor::new(&key.into(), &iv.into())
            .decrypt_padded_vec_mut::<Pkcs7>(&locked)
            .unwrap();
        assert_eq!(opened, plain);
        // The wrong IV or the wrong key does not give it back.
        assert_ne!(encrypt_segment(&key, 43, &plain), locked);
        assert_ne!(encrypt_segment(&[8u8; 16], 42, &plain), locked);
    }

    #[test]
    fn playlists_that_are_not_what_ffmpeg_writes_are_refused() {
        for (why, text) in [
            ("not a playlist", "hello".to_owned()),
            (
                "already encrypted",
                clear(2).replace(
                    "#EXT-X-PLAYLIST-TYPE:VOD",
                    "#EXT-X-KEY:METHOD=AES-128,URI=\"x\"",
                ),
            ),
            ("no segments", "#EXTM3U\n#EXT-X-ENDLIST\n".to_owned()),
            ("a path", clear(2).replace("seg_00001.ts", "../x.ts")),
            ("a path", clear(2).replace("seg_00001.ts", "a/b.ts")),
            ("a bad duration", clear(2).replace("6.000000", "soon")),
            (
                "a segment with no duration",
                "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:0\nseg_00000.ts\n".to_owned(),
            ),
            (
                "a duration with no segment",
                "#EXTM3U\n#EXTINF:6,\n".to_owned(),
            ),
            (
                "a bad sequence",
                clear(2).replace("MEDIA-SEQUENCE:0", "MEDIA-SEQUENCE:x"),
            ),
        ] {
            assert!(seal_playlist(&text, Some(30)).is_err(), "{why}");
        }
    }
}
