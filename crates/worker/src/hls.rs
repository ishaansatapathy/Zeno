//! Everything about the shape of the HLS output that does not need ffmpeg to run: encoder
//! arguments, master playlist generation, and reading media playlists back.

use std::path::Path;

use crate::ladder::{self, Rung, AUDIO_ONLY_KBPS};

pub const SEGMENT_SECS: u32 = 6;
/// Written into every playlist's `#EXT-X-KEY`. The player never sees it: playback replaces it
/// with a short-lived, per-viewer key URL when the playlist is served.
pub const KEY_URI_PLACEHOLDER: &str = "zeno-key://video";
pub const PLAYLIST_NAME: &str = "index.m3u8";
pub const SEGMENT_PATTERN: &str = "seg_%05d.ts";

/// Above this, a source's frame rate is reduced so high-fps footage does not double the
/// bitrate of every rendition.
pub const FPS_CAP: f64 = 30.5;

pub struct EncodeInput<'a> {
    pub source: &'a Path,
    /// ffmpeg's `-hls_key_info_file`: key URI, key file path, optional IV.
    pub key_info: &'a Path,
    pub max_duration_secs: f64,
    pub threads: u32,
}

/// ffmpeg is always started with an argument list, never a shell string. `-protocol_whitelist`
/// comes before `-i` so the input can only be a local file, and metadata, chapters, subtitles
/// and data streams are dropped so nothing from the upload leaks into the output.
fn common_input(input: &EncodeInput<'_>) -> Vec<String> {
    vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-protocol_whitelist".into(),
        "file".into(),
        // Never encode more than the platform limit, whatever the container claims.
        "-t".into(),
        format!("{}", input.max_duration_secs.ceil() as u64 + 1),
        "-i".into(),
        format!("file:{}", slashed(input.source)),
    ]
}

fn hls_output(input: &EncodeInput<'_>) -> Vec<String> {
    vec![
        "-threads".into(),
        input.threads.to_string(),
        "-f".into(),
        "hls".into(),
        "-hls_time".into(),
        SEGMENT_SECS.to_string(),
        "-hls_playlist_type".into(),
        "vod".into(),
        "-hls_flags".into(),
        "independent_segments".into(),
        "-hls_segment_type".into(),
        "mpegts".into(),
        "-hls_segment_filename".into(),
        SEGMENT_PATTERN.into(),
        "-hls_key_info_file".into(),
        slashed(input.key_info),
        PLAYLIST_NAME.into(),
    ]
}

const STRIP: [&str; 6] = ["-map_metadata", "-1", "-map_chapters", "-1", "-sn", "-dn"];

pub fn video_args(
    input: &EncodeInput<'_>,
    rung: &Rung,
    with_audio: bool,
    reduce_fps: bool,
) -> Vec<String> {
    let mut args = common_input(input);
    args.extend(["-map".into(), "0:v:0".into()]);
    if with_audio {
        args.extend(["-map".into(), "0:a:0".into()]);
    }
    args.extend(STRIP.map(String::from));

    // Width follows the display aspect ratio (so non-square pixels come out right), rounded to
    // an even number; the pixel aspect is then reset to square.
    let mut filter = format!(
        "scale=w=trunc(oh*dar/2)*2:h={}:flags=bicubic,setsar=1",
        rung.height
    );
    if reduce_fps {
        filter.push_str(",fps=30");
    }
    filter.push_str(",format=yuv420p");

    args.extend([
        "-vf".into(),
        filter,
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-profile:v".into(),
        "main".into(),
        "-level:v".into(),
        "3.1".into(),
        "-crf".into(),
        rung.crf.to_string(),
        "-maxrate".into(),
        format!("{}k", rung.maxrate_kbps),
        "-bufsize".into(),
        format!("{}k", rung.maxrate_kbps * 2),
        // Keyframes exactly every segment on every rendition, so players can switch between
        // them at segment boundaries.
        "-force_key_frames".into(),
        format!("expr:gte(t,n_forced*{SEGMENT_SECS})"),
        "-sc_threshold".into(),
        "0".into(),
    ]);
    if with_audio {
        args.extend(audio_codec(rung.audio_kbps));
    } else {
        args.push("-an".into());
    }
    args.extend(hls_output(input));
    args
}

pub fn audio_only_args(input: &EncodeInput<'_>) -> Vec<String> {
    let mut args = common_input(input);
    args.extend(["-map".into(), "0:a:0".into(), "-vn".into()]);
    args.extend(STRIP.map(String::from));
    args.extend(audio_codec(AUDIO_ONLY_KBPS));
    args.extend(hls_output(input));
    args
}

fn audio_codec(kbps: u32) -> Vec<String> {
    vec![
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        format!("{kbps}k"),
        "-ac".into(),
        "2".into(),
        "-ar".into(),
        "48000".into(),
    ]
}

/// ffmpeg and ffprobe accept forward slashes on every platform, and a `file:` prefix stops a
/// path that starts with `-` from being read as an option.
pub fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub duration_secs: f64,
    pub uri: String,
}

/// Segment durations and names from a media playlist.
pub fn parse_media_playlist(text: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut pending: Option<f64> = None;
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            pending = rest
                .split(',')
                .next()
                .and_then(|d| d.trim().parse::<f64>().ok())
                .filter(|d| d.is_finite() && *d >= 0.0);
        } else if !line.is_empty() && !line.starts_with('#') {
            if let Some(duration_secs) = pending.take() {
                segments.push(Segment {
                    duration_secs,
                    uri: line.to_owned(),
                });
            }
        }
    }
    segments
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    /// `None` for the audio-only variant.
    pub resolution: Option<(u32, u32)>,
    pub frame_rate: Option<f64>,
    pub has_audio: bool,
    pub peak_bps: u64,
    pub average_bps: u64,
    pub playlist: String,
}

/// Lowest-quality video first (players start at the first entry), audio-only last (Apple's
/// authoring guidance: never let an audio-only variant be the default).
pub fn master_playlist(variants: &[Variant]) -> String {
    let mut ordered: Vec<&Variant> = variants.iter().collect();
    ordered.sort_by_key(|v| (v.resolution.is_none(), v.peak_bps));

    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    for variant in ordered {
        let codecs = match (variant.resolution.is_some(), variant.has_audio) {
            (true, true) => "avc1.4d401f,mp4a.40.2",
            (true, false) => "avc1.4d401f",
            (false, _) => "mp4a.40.2",
        };
        out.push_str(&format!(
            "#EXT-X-STREAM-INF:BANDWIDTH={},AVERAGE-BANDWIDTH={}",
            variant.peak_bps, variant.average_bps
        ));
        if let Some((width, height)) = variant.resolution {
            out.push_str(&format!(",RESOLUTION={width}x{height}"));
        }
        if let Some(fps) = variant.frame_rate {
            out.push_str(&format!(",FRAME-RATE={fps:.3}"));
        }
        out.push_str(&format!(",CODECS=\"{codecs}\"\n{}\n", variant.playlist));
    }
    out
}

pub fn content_type_for(path: &str) -> &'static str {
    if path.ends_with(".m3u8") {
        "application/vnd.apple.mpegurl"
    } else if path.ends_with(".ts") {
        "video/mp2t"
    } else {
        "application/octet-stream"
    }
}

/// Peak and average bitrate of an encoded rendition, from the actual segment sizes. The peak
/// never goes below what the encoder was configured to allow, since the player uses it to
/// decide whether the network can sustain the rendition.
pub fn measure(segments: &[(f64, u64)], configured_peak_bps: u64) -> (u64, u64) {
    let total_secs: f64 = segments.iter().map(|(d, _)| d).sum();
    let total_bits: f64 = segments.iter().map(|(_, bytes)| *bytes as f64 * 8.0).sum();
    let average = if total_secs > 0.0 {
        (total_bits / total_secs) as u64
    } else {
        0
    };
    let peak = segments
        .iter()
        .filter(|(d, _)| *d > 0.0)
        .map(|(d, bytes)| (*bytes as f64 * 8.0 / d) as u64)
        .max()
        .unwrap_or(0);
    (peak.max(configured_peak_bps), average)
}

/// Bandwidth ceiling for a rendition as configured: video plus audio, plus ~10% for the
/// transport stream's own overhead.
pub fn configured_peak_bps(rung: &Rung, with_audio: bool) -> u64 {
    let audio = if with_audio { rung.audio_kbps } else { 0 };
    u64::from(rung.maxrate_kbps + audio) * 1100
}

pub fn audio_only_configured_peak_bps() -> u64 {
    u64::from(AUDIO_ONLY_KBPS) * 1100
}

pub fn variant_width(rung: &Rung, display_aspect: f64) -> u32 {
    ladder::display_width(rung.height, display_aspect)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn input() -> (PathBuf, PathBuf) {
        (
            PathBuf::from("C:\\work\\job 1\\source"),
            PathBuf::from("/tmp/work/keyinfo.txt"),
        )
    }

    fn args_for(rung_height: u32, audio: bool, fps: bool) -> Vec<String> {
        let (source, key_info) = input();
        let rung = ladder::choose(rung_height).pop().unwrap();
        video_args(
            &EncodeInput {
                source: &source,
                key_info: &key_info,
                max_duration_secs: 3600.0,
                threads: 2,
            },
            &rung,
            audio,
            fps,
        )
    }

    fn pos(args: &[String], flag: &str) -> usize {
        args.iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("{flag} missing in {args:?}"))
    }

    #[test]
    fn the_input_is_locked_to_local_files_before_it_is_opened() {
        let args = args_for(720, true, false);
        assert!(pos(&args, "-protocol_whitelist") < pos(&args, "-i"));
        assert_eq!(args[pos(&args, "-protocol_whitelist") + 1], "file");
        assert_eq!(args[pos(&args, "-i") + 1], "file:C:/work/job 1/source");
        assert!(args.contains(&"-nostdin".to_string()));
    }

    #[test]
    fn nothing_from_the_upload_is_carried_into_the_output() {
        for args in [args_for(720, true, false), audio_only()] {
            assert_eq!(args[pos(&args, "-map_metadata") + 1], "-1");
            assert_eq!(args[pos(&args, "-map_chapters") + 1], "-1");
            assert!(args.contains(&"-sn".to_string()) && args.contains(&"-dn".to_string()));
        }
    }

    fn audio_only() -> Vec<String> {
        let (source, key_info) = input();
        audio_only_args(&EncodeInput {
            source: &source,
            key_info: &key_info,
            max_duration_secs: 60.0,
            threads: 0,
        })
    }

    #[test]
    fn encoding_is_capped_at_the_platform_duration_limit() {
        let args = args_for(720, true, false);
        assert_eq!(args[pos(&args, "-t") + 1], "3601");
        assert!(
            pos(&args, "-t") < pos(&args, "-i"),
            "-t before -i limits the input"
        );
    }

    #[test]
    fn video_output_is_aes_encrypted_vod_hls_with_aligned_keyframes() {
        let args = args_for(480, true, false);
        let joined = args.join(" ");
        for needle in [
            "-hls_playlist_type vod",
            "-hls_flags independent_segments",
            "-hls_segment_type mpegts",
            "-hls_segment_filename seg_%05d.ts",
            "-hls_key_info_file /tmp/work/keyinfo.txt",
            "-force_key_frames expr:gte(t,n_forced*6)",
            "-sc_threshold 0",
            "-hls_time 6",
            "-c:v libx264",
            "-maxrate 1400k",
        ] {
            assert!(joined.contains(needle), "missing {needle:?} in {joined}");
        }
        assert_eq!(args.last().unwrap(), "index.m3u8");
    }

    #[test]
    fn the_scale_filter_uses_the_display_aspect_and_resets_pixel_shape() {
        let args = args_for(240, true, false);
        let filter = &args[pos(&args, "-vf") + 1];
        assert_eq!(
            filter,
            "scale=w=trunc(oh*dar/2)*2:h=240:flags=bicubic,setsar=1,format=yuv420p"
        );
        let capped = args_for(240, true, true);
        assert!(capped[pos(&capped, "-vf") + 1].contains("fps=30"));
    }

    #[test]
    fn silent_sources_get_no_audio_stream_and_others_get_stereo_aac() {
        let silent = args_for(360, false, false);
        assert!(silent.contains(&"-an".to_string()) && !silent.contains(&"0:a:0".to_string()));
        let voiced = args_for(360, true, false);
        let joined = voiced.join(" ");
        assert!(joined.contains("-map 0:a:0") && joined.contains("-c:a aac -b:a 96k -ac 2"));
    }

    #[test]
    fn audio_only_renditions_drop_video_entirely() {
        let joined = audio_only().join(" ");
        assert!(
            joined.contains("-vn") && joined.contains("-b:a 64k") && !joined.contains("libx264")
        );
    }

    #[test]
    fn every_argument_is_a_separate_element_so_no_shell_can_reinterpret_them() {
        let (source, key_info) = (
            PathBuf::from("/tmp/a b; rm -rf ~/source"),
            PathBuf::from("/tmp/k"),
        );
        let rung = ladder::choose(720).pop().unwrap();
        let args = video_args(
            &EncodeInput {
                source: &source,
                key_info: &key_info,
                max_duration_secs: 10.0,
                threads: 1,
            },
            &rung,
            true,
            false,
        );
        assert!(args.contains(&"file:/tmp/a b; rm -rf ~/source".to_string()));
    }

    #[test]
    fn media_playlists_are_read_back_into_segments() {
        let text =
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video\"\n\
                    #EXTINF:6.000000,\nseg_00000.ts\n#EXTINF:5.5,\nseg_00001.ts\n#EXT-X-ENDLIST\n";
        assert_eq!(
            parse_media_playlist(text),
            vec![
                Segment {
                    duration_secs: 6.0,
                    uri: "seg_00000.ts".into()
                },
                Segment {
                    duration_secs: 5.5,
                    uri: "seg_00001.ts".into()
                },
            ]
        );
        assert!(parse_media_playlist("").is_empty());
        assert!(parse_media_playlist("seg.ts\n#EXTINF:abc,\nx.ts").is_empty());
    }

    fn variant(name: &str, res: Option<(u32, u32)>, peak: u64) -> Variant {
        Variant {
            name: name.into(),
            resolution: res,
            frame_rate: res.map(|_| 25.0),
            has_audio: true,
            peak_bps: peak,
            average_bps: peak / 2,
            playlist: format!("{name}/index.m3u8"),
        }
    }

    #[test]
    fn the_master_playlist_starts_low_and_puts_audio_only_last() {
        let playlist = master_playlist(&[
            variant("720p", Some((1280, 720)), 3_000_000),
            variant("audio", None, 70_000),
            variant("144p", Some((256, 144)), 250_000),
            variant("360p", Some((640, 360)), 900_000),
        ]);
        let uris: Vec<&str> = playlist
            .lines()
            .filter(|l| l.ends_with("index.m3u8"))
            .collect();
        assert_eq!(
            uris,
            [
                "144p/index.m3u8",
                "360p/index.m3u8",
                "720p/index.m3u8",
                "audio/index.m3u8"
            ]
        );
        assert!(playlist.starts_with("#EXTM3U\n"));
        assert!(playlist.contains("BANDWIDTH=250000,AVERAGE-BANDWIDTH=125000,RESOLUTION=256x144,FRAME-RATE=25.000,CODECS=\"avc1.4d401f,mp4a.40.2\""));
        assert!(playlist.contains(
            "BANDWIDTH=70000,AVERAGE-BANDWIDTH=35000,CODECS=\"mp4a.40.2\"\naudio/index.m3u8"
        ));
    }

    #[test]
    fn bandwidth_is_measured_from_segments_and_never_below_the_configured_ceiling() {
        let (peak, average) = measure(&[(6.0, 600_000), (6.0, 300_000)], 100_000);
        assert_eq!(average, 600_000);
        assert_eq!(peak, 800_000);
        let (floor, _) = measure(&[(6.0, 1000)], 500_000);
        assert_eq!(floor, 500_000);
        assert_eq!(measure(&[], 42), (42, 0));
        let rung = ladder::choose(720).pop().unwrap();
        assert_eq!(configured_peak_bps(&rung, true), (2800 + 128) * 1100);
        assert_eq!(configured_peak_bps(&rung, false), 2800 * 1100);
    }

    #[test]
    fn content_types_are_set_for_playlists_and_segments() {
        assert_eq!(
            content_type_for("a/index.m3u8"),
            "application/vnd.apple.mpegurl"
        );
        assert_eq!(content_type_for("a/seg_00001.ts"), "video/mp2t");
        assert_eq!(content_type_for("x"), "application/octet-stream");
    }
}
