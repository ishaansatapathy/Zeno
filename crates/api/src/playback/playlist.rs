//! Rewrites the playlists the worker stored into the ones a particular viewer receives.
//!
//! Stored playlists hold relative names and a placeholder key URI (`zeno-key://video`). Served
//! playlists point every rendition back at the API (carrying the viewer's token), every segment
//! at a short-lived presigned storage URL, and the key at the key server.
//!
//! The rewriting is strict. Anything the worker would never write (an unknown rendition, a URI
//! in some other tag, a segment name with a path in it, a playlist with no encryption key) is
//! refused instead of passed through, so a corrupted or tampered object in storage can't turn
//! the API into a proxy for arbitrary URLs or serve video in the clear.

use zeno_core::segment_keys::KeyRef;
#[cfg(test)]
use zeno_core::segment_keys::KEY_URI_PLACEHOLDER;

/// Each rendition's playlist inside its directory; see `zeno_worker::hls::PLAYLIST_NAME`.
pub const STORED_PLAYLIST_NAME: &str = "index.m3u8";
const MAX_SEGMENT_NAME: usize = 64;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RewriteError {
    #[error("not an HLS playlist")]
    NotAPlaylist,
    #[error("the master playlist names an unknown rendition: {0}")]
    UnknownRendition(String),
    #[error("unexpected URI in playlist: {0}")]
    UnexpectedUri(String),
    #[error("the media playlist is not encrypted with the video's key")]
    NotEncrypted,
}

fn is_playlist(text: &str) -> bool {
    text.lines().next().map(str::trim_end) == Some("#EXTM3U")
}

/// A name ffmpeg could have produced for a segment: no directories, no tricks.
fn is_safe_segment_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SEGMENT_NAME
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && name.ends_with(".ts")
}

/// Points each rendition at `{name}.m3u8?t={token}` (relative to the master's own URL).
pub fn rewrite_master(
    text: &str,
    renditions: &[String],
    token: &str,
) -> Result<String, RewriteError> {
    if !is_playlist(text) {
        return Err(RewriteError::NotAPlaylist);
    }
    let mut out = String::with_capacity(text.len() + 256);
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            // Our masters only carry variant streams; any tag with its own URI (alternate audio,
            // I-frame playlists, session keys) was not written by the worker.
            if line.contains("URI=") {
                return Err(RewriteError::UnexpectedUri(line.to_owned()));
            }
            out.push_str(line);
        } else {
            let name = line
                .strip_suffix(&format!("/{STORED_PLAYLIST_NAME}"))
                .filter(|name| renditions.iter().any(|r| r == name))
                .ok_or_else(|| RewriteError::UnknownRendition(line.to_owned()))?;
            out.push_str(&format!("{name}.m3u8?t={token}"));
        }
        out.push('\n');
    }
    Ok(out)
}

/// Replaces each key placeholder with `key_url(which key)` and each segment name with `segment_url(name)`.
///
/// A video's segments are locked in periods (`zeno_core::segment_keys`), so a playlist holds a key tag at the
/// start of each period, every one naming its own key. Older videos have one tag for the whole of it. Any
/// other address in a key tag is refused.
pub fn rewrite_media(
    text: &str,
    key_url: impl Fn(KeyRef) -> String,
    segment_url: impl Fn(&str) -> String,
) -> Result<String, RewriteError> {
    if !is_playlist(text) {
        return Err(RewriteError::NotAPlaylist);
    }
    let mut keyed = false;
    let mut out = String::with_capacity(text.len() * 4);
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("#EXT-X-KEY:") {
            if line.contains("METHOD=NONE") || line.matches("URI=").count() != 1 {
                return Err(RewriteError::NotEncrypted);
            }
            const OPEN: &str = "URI=\"";
            let unexpected = || RewriteError::UnexpectedUri(line.to_owned());
            if !line.contains("METHOD=AES-128,") {
                return Err(unexpected());
            }
            let start = line.find(OPEN).ok_or_else(unexpected)? + OPEN.len();
            let end = line[start..].find('"').ok_or_else(unexpected)? + start;
            let key = KeyRef::from_placeholder(&line[start..end]).ok_or_else(unexpected)?;
            let url = key_url(key);
            debug_assert!(
                !url.contains('"'),
                "the key URL is placed in a quoted attribute"
            );
            out.push_str(&line[..start]);
            out.push_str(&url);
            out.push_str(&line[end..]);
            keyed = true;
        } else if line.starts_with('#') {
            if line.contains("URI=") {
                return Err(RewriteError::UnexpectedUri(line.to_owned()));
            }
            out.push_str(line);
        } else {
            if !keyed {
                // A segment before any key would be played unencrypted.
                return Err(RewriteError::NotEncrypted);
            }
            if !is_safe_segment_name(line) {
                return Err(RewriteError::UnexpectedUri(line.to_owned()));
            }
            out.push_str(&segment_url(line));
        }
        out.push('\n');
    }
    if !keyed {
        return Err(RewriteError::NotEncrypted);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-INDEPENDENT-SEGMENTS\n\
        #EXT-X-STREAM-INF:BANDWIDTH=250000,RESOLUTION=256x144,CODECS=\"avc1.4d401f,mp4a.40.2\"\n144p/index.m3u8\n\
        #EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=1280x720,CODECS=\"avc1.4d401f,mp4a.40.2\"\n720p/index.m3u8\n\
        #EXT-X-STREAM-INF:BANDWIDTH=70000,CODECS=\"mp4a.40.2\"\naudio/index.m3u8\n";

    // Shaped like ffmpeg's output for an AES-128 VOD rendition.
    const MEDIA: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:0\n\
        #EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video\",IV=0x00000000000000000000000000000000\n\
        #EXTINF:6.000000,\nseg_00000.ts\n#EXTINF:3.500000,\nseg_00001.ts\n#EXT-X-ENDLIST\n";

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn segment(name: &str) -> String {
        format!("https://storage/{name}?sig")
    }

    /// What `service::media` does: one address, with the period named when there is one.
    fn key(which: KeyRef) -> String {
        match which {
            KeyRef::Whole => "https://keys/v1/keys/V?t=TOKEN".to_owned(),
            KeyRef::Period(period) => format!("https://keys/v1/keys/V?t=TOKEN&k={period}"),
        }
    }

    /// Shaped like what the worker writes now: a key tag at the start of each period, no IV.
    const ROTATED: &str =
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:0\n\
        #EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video/0\"\n\
        #EXTINF:6.000000,\nseg_00000.ts\n#EXTINF:6.000000,\nseg_00001.ts\n\
        #EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video/1\"\n\
        #EXTINF:6.000000,\nseg_00002.ts\n#EXTINF:3.500000,\nseg_00003.ts\n#EXT-X-ENDLIST\n";

    #[test]
    fn a_rotated_playlist_names_each_periods_key_in_its_own_tag() {
        let out = rewrite_media(ROTATED, key, segment).unwrap();
        let tags: Vec<&str> = out
            .lines()
            .filter(|l| l.starts_with("#EXT-X-KEY"))
            .collect();
        assert_eq!(
            tags,
            [
                "#EXT-X-KEY:METHOD=AES-128,URI=\"https://keys/v1/keys/V?t=TOKEN&k=0\"",
                "#EXT-X-KEY:METHOD=AES-128,URI=\"https://keys/v1/keys/V?t=TOKEN&k=1\"",
            ]
        );
        assert!(!out.contains("zeno-key"));
        // The second key still comes before the first segment it covers.
        let lines: Vec<&str> = out.lines().collect();
        let second = lines.iter().position(|l| l.contains("&k=1")).unwrap();
        assert!(lines[second + 1].starts_with("#EXTINF"));
        assert_eq!(lines[second + 2], "https://storage/seg_00002.ts?sig");
    }

    #[test]
    fn a_key_address_that_is_not_exactly_one_the_worker_writes_is_refused() {
        for uri in [
            "zeno-key://video/",
            "zeno-key://video/-1",
            "zeno-key://video/01",
            "zeno-key://video/1/2",
            "zeno-key://video/99999999999",
            "zeno-key://video?x=1",
            "zeno-key://other/1",
            "https://evil/key",
            "",
        ] {
            let text = ROTATED.replace("zeno-key://video/1", uri);
            assert!(
                matches!(
                    rewrite_media(&text, key, segment),
                    Err(RewriteError::UnexpectedUri(_))
                ),
                "{uri:?}"
            );
        }
    }

    #[test]
    fn an_attribute_after_the_address_is_kept() {
        let with_iv = ROTATED.replace(
            "URI=\"zeno-key://video/0\"",
            "URI=\"zeno-key://video/0\",IV=0x00000000000000000000000000000001",
        );
        let out = rewrite_media(&with_iv, key, segment).unwrap();
        assert!(out.contains("&k=0\",IV=0x00000000000000000000000000000001\n"));
    }

    #[test]
    fn master_renditions_point_back_at_the_api_with_the_token() {
        let out = rewrite_master(MASTER, &names(&["144p", "720p", "audio"]), "TOKEN").unwrap();
        let uris: Vec<&str> = out.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            uris,
            [
                "144p.m3u8?t=TOKEN",
                "720p.m3u8?t=TOKEN",
                "audio.m3u8?t=TOKEN"
            ]
        );
        // Every tag line survives unchanged and in order.
        let tags = |text: &str| {
            text.lines()
                .filter(|l| l.starts_with('#'))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(tags(&out), tags(MASTER));
    }

    #[test]
    fn a_master_naming_a_rendition_the_video_does_not_have_is_refused() {
        let err = rewrite_master(MASTER, &names(&["144p", "720p"]), "T").unwrap_err();
        assert_eq!(
            err,
            RewriteError::UnknownRendition("audio/index.m3u8".into())
        );
        for line in [
            "../secret/index.m3u8",
            "https://evil/index.m3u8",
            "720p/../../x/index.m3u8",
            "720p",
        ] {
            let text = format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\n{line}\n");
            assert!(
                rewrite_master(&text, &names(&["720p"]), "T").is_err(),
                "{line}"
            );
        }
    }

    #[test]
    fn master_tags_carrying_their_own_uri_are_refused() {
        let text =
            format!("{MASTER}#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",URI=\"https://evil/a.m3u8\"\n");
        assert!(matches!(
            rewrite_master(&text, &names(&["144p", "720p", "audio"]), "T"),
            Err(RewriteError::UnexpectedUri(_))
        ));
    }

    #[test]
    fn media_playlists_get_the_key_url_and_signed_segment_urls() {
        let out = rewrite_media(MEDIA, key, segment).unwrap();
        assert!(out.contains(
            "#EXT-X-KEY:METHOD=AES-128,URI=\"https://keys/v1/keys/V?t=TOKEN\",IV=0x00000000000000000000000000000000\n"
        ));
        assert!(
            !out.contains("zeno-key"),
            "the placeholder never reaches a player"
        );
        let segments: Vec<&str> = out.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            segments,
            [
                "https://storage/seg_00000.ts?sig",
                "https://storage/seg_00001.ts?sig"
            ]
        );
        assert!(out.contains("#EXTINF:3.500000,\n"));
        assert!(out.trim_end().ends_with("#EXT-X-ENDLIST"));
    }

    #[test]
    fn a_media_playlist_without_encryption_is_never_served() {
        let clear = MEDIA.replace(
            "#EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video\",IV=0x00000000000000000000000000000000\n",
            "",
        );
        assert_eq!(
            rewrite_media(&clear, key, segment),
            Err(RewriteError::NotEncrypted)
        );
        let none = MEDIA.replace(
            "METHOD=AES-128,URI=\"zeno-key://video\",IV=0x00000000000000000000000000000000",
            "METHOD=NONE",
        );
        assert_eq!(
            rewrite_media(&none, key, segment),
            Err(RewriteError::NotEncrypted)
        );
        // A key that only starts halfway leaves the first segments in the clear.
        let late = "#EXTM3U\n#EXTINF:6,\nseg_00000.ts\n#EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video\"\n#EXTINF:6,\nseg_00001.ts\n";
        assert_eq!(
            rewrite_media(late, key, segment),
            Err(RewriteError::NotEncrypted)
        );
    }

    #[test]
    fn a_key_uri_other_than_the_placeholder_is_refused() {
        let foreign = MEDIA.replace("zeno-key://video", "https://evil/key");
        assert!(matches!(
            rewrite_media(&foreign, key, segment),
            Err(RewriteError::UnexpectedUri(_))
        ));
    }

    #[test]
    fn segment_names_with_paths_or_odd_characters_are_refused() {
        for name in [
            "../x.ts",
            "a/b.ts",
            "https://evil/x.ts",
            ".hidden.ts",
            "seg 1.ts",
            "seg.mp4",
            "",
            "seg%2F.ts",
        ] {
            let text = MEDIA.replace("seg_00001.ts", name);
            if name.is_empty() {
                // An empty line is just skipped; that is not a segment.
                assert!(rewrite_media(&text, key, segment).is_ok());
                continue;
            }
            assert!(rewrite_media(&text, key, segment).is_err(), "{name:?}");
        }
    }

    #[test]
    fn other_uri_tags_in_media_playlists_are_refused() {
        let map = MEDIA.replace(
            "#EXT-X-PLAYLIST-TYPE:VOD",
            "#EXT-X-MAP:URI=\"https://evil/init.mp4\"",
        );
        assert!(matches!(
            rewrite_media(&map, key, segment),
            Err(RewriteError::UnexpectedUri(_))
        ));
    }

    #[test]
    fn things_that_are_not_playlists_are_refused() {
        for text in ["", "hello", "<html>", "\n#EXTM3U"] {
            assert_eq!(
                rewrite_master(text, &[], "t"),
                Err(RewriteError::NotAPlaylist)
            );
            assert_eq!(
                rewrite_media(text, key, segment),
                Err(RewriteError::NotAPlaylist)
            );
        }
    }

    #[test]
    fn windows_line_endings_are_tolerated() {
        let crlf = MEDIA.replace('\n', "\r\n");
        let out = rewrite_media(&crlf, key, segment).unwrap();
        assert!(!out.contains('\r'));
    }

    #[test]
    fn the_constants_match_what_the_worker_writes() {
        assert_eq!(KEY_URI_PLACEHOLDER, "zeno-key://video");
        assert_eq!(STORED_PLAYLIST_NAME, "index.m3u8");
    }
}
