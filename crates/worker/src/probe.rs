//! Reading and vetting what ffprobe says about an uploaded file. The file is untrusted, so
//! this is where unsupported and oversized media is turned away before any encoding starts.

use serde::Deserialize;

use crate::config::Limits;
use crate::error::JobError;

/// Containers a creator's recorder or phone can plausibly produce. Anything else (playlists,
/// concat scripts, image sequences, network or device inputs) is refused outright, which also
/// closes the door on inputs that make ffmpeg open other files or URLs.
const ALLOWED_FORMATS: [&str; 13] = [
    "mov", "mp4", "m4a", "3gp", "3g2", "mj2", "matroska", "webm", "avi", "mpegts", "mpeg", "flv",
    "asf",
];
const MAX_STREAMS: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    /// Dimensions after applying any rotation, i.e. what a viewer sees.
    pub width: u32,
    pub height: u32,
    /// Width over height as displayed, including non-square pixels.
    pub display_aspect: f64,
    pub fps: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub format_names: Vec<String>,
    pub duration_secs: Option<f64>,
    pub video: Option<VideoInfo>,
    pub has_audio: bool,
    pub stream_count: usize,
}

#[derive(Deserialize)]
struct RawProbe {
    #[serde(default)]
    format: RawFormat,
    #[serde(default)]
    streams: Vec<RawStream>,
}

#[derive(Deserialize, Default)]
struct RawFormat {
    format_name: Option<String>,
    duration: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawStream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    sample_aspect_ratio: Option<String>,
    avg_frame_rate: Option<String>,
    duration: Option<String>,
    disposition: Option<RawDisposition>,
    tags: Option<RawTags>,
    side_data_list: Option<Vec<RawSideData>>,
}

#[derive(Deserialize, Default)]
struct RawDisposition {
    attached_pic: Option<u8>,
}

#[derive(Deserialize, Default)]
struct RawTags {
    rotate: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawSideData {
    rotation: Option<f64>,
}

pub fn parse(json: &str) -> Result<Probe, JobError> {
    let raw: RawProbe = serde_json::from_str(json).map_err(|e| {
        JobError::permanent("unsupported_media", format!("unreadable probe output: {e}"))
    })?;
    let format_names: Vec<String> = raw
        .format
        .format_name
        .unwrap_or_default()
        .split(',')
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect();

    // Cover art in an audio file shows up as a "video" stream; it is not one.
    let video_stream = raw.streams.iter().find(|s| {
        s.codec_type.as_deref() == Some("video")
            && s.disposition.as_ref().and_then(|d| d.attached_pic) != Some(1)
    });
    let has_audio = raw
        .streams
        .iter()
        .any(|s| s.codec_type.as_deref() == Some("audio"));

    let duration_secs = parse_duration(raw.format.duration.as_deref()).or_else(|| {
        raw.streams
            .iter()
            .filter_map(|s| parse_duration(s.duration.as_deref()))
            .fold(None, |best: Option<f64>, d| {
                Some(best.map_or(d, |b| b.max(d)))
            })
    });

    Ok(Probe {
        format_names,
        duration_secs,
        video: video_stream.and_then(video_info),
        has_audio,
        stream_count: raw.streams.len(),
    })
}

fn parse_duration(raw: Option<&str>) -> Option<f64> {
    raw?.trim()
        .parse::<f64>()
        .ok()
        .filter(|d| d.is_finite() && *d > 0.0)
}

fn video_info(stream: &RawStream) -> Option<VideoInfo> {
    let (stored_w, stored_h) = (stream.width?, stream.height?);
    if stored_w == 0 || stored_h == 0 {
        return None;
    }
    let sar = parse_ratio(stream.sample_aspect_ratio.as_deref()).unwrap_or(1.0);
    let rotation = rotation_degrees(stream).rem_euclid(180);
    let (width, height, display_aspect) = if rotation == 90 {
        (
            stored_h,
            stored_w,
            f64::from(stored_h) / (f64::from(stored_w) * sar),
        )
    } else {
        (
            stored_w,
            stored_h,
            f64::from(stored_w) * sar / f64::from(stored_h),
        )
    };
    Some(VideoInfo {
        width,
        height,
        display_aspect,
        fps: parse_ratio(stream.avg_frame_rate.as_deref()),
    })
}

/// Clockwise-normalised rotation in whole degrees, from either of the two places containers
/// record it.
fn rotation_degrees(stream: &RawStream) -> i64 {
    let from_side_data = stream
        .side_data_list
        .iter()
        .flatten()
        .find_map(|d| d.rotation)
        .map(|r| r.round() as i64);
    let from_tag = stream
        .tags
        .as_ref()
        .and_then(|t| t.rotate.as_deref())
        .and_then(|r| r.trim().parse::<f64>().ok())
        .map(|r| r.round() as i64);
    from_side_data.or(from_tag).unwrap_or(0)
}

/// "30000/1001" or "16:9" to a number; `None` for "0/0", "N/A" and nonsense.
fn parse_ratio(raw: Option<&str>) -> Option<f64> {
    let raw = raw?.trim();
    let (num, den) = raw.split_once(['/', ':'])?;
    let (num, den): (f64, f64) = (num.trim().parse().ok()?, den.trim().parse().ok()?);
    (den > 0.0 && num > 0.0).then(|| num / den)
}

/// Rejects anything the platform will not process. Every failure carries the code shown to
/// the creator.
pub fn validate(probe: &Probe, limits: &Limits) -> Result<VideoInfo, JobError> {
    if probe.format_names.is_empty()
        || !probe
            .format_names
            .iter()
            .all(|name| ALLOWED_FORMATS.contains(&name.as_str()))
    {
        return Err(JobError::permanent(
            "unsupported_media",
            format!("container {:?} is not accepted", probe.format_names),
        ));
    }
    if probe.stream_count > MAX_STREAMS {
        return Err(JobError::permanent(
            "unsupported_media",
            format!("{} streams", probe.stream_count),
        ));
    }
    let Some(video) = probe.video.clone() else {
        return Err(JobError::permanent(
            "no_video_stream",
            "no playable video stream",
        ));
    };
    if video.width > limits.max_dimension || video.height > limits.max_dimension {
        return Err(JobError::permanent(
            "resolution_too_high",
            format!(
                "{}x{} exceeds {}",
                video.width, video.height, limits.max_dimension
            ),
        ));
    }
    if probe
        .duration_secs
        .is_some_and(|d| d > limits.max_duration_secs)
    {
        return Err(JobError::permanent(
            "too_long",
            format!(
                "{:?}s exceeds {}s",
                probe.duration_secs, limits.max_duration_secs
            ),
        ));
    }
    Ok(video)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits {
        max_duration_secs: 3600.0,
        max_dimension: 4096,
        max_source_bytes: 1 << 30,
    };

    fn probe(json: &str) -> Probe {
        parse(json).unwrap()
    }

    fn code(result: Result<VideoInfo, JobError>) -> &'static str {
        match result {
            Err(JobError::Permanent { code, .. }) => code,
            other => panic!("expected a permanent error, got {other:?}"),
        }
    }

    const MP4: &str = r#"{"format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"12.480000"},
        "streams":[
          {"codec_type":"video","width":1920,"height":1080,"sample_aspect_ratio":"1:1","avg_frame_rate":"30000/1001"},
          {"codec_type":"audio"}]}"#;

    #[test]
    fn a_normal_phone_video_is_understood() {
        let p = probe(MP4);
        assert_eq!(p.duration_secs, Some(12.48));
        assert!(p.has_audio);
        let v = validate(&p, &LIMITS).unwrap();
        assert_eq!((v.width, v.height), (1920, 1080));
        assert!((v.display_aspect - 16.0 / 9.0).abs() < 1e-9);
        assert!((v.fps.unwrap() - 29.97).abs() < 0.01);
    }

    #[test]
    fn rotation_swaps_the_displayed_dimensions_from_either_source() {
        let side = r#"{"format":{"format_name":"mov,mp4","duration":"5"},"streams":[
            {"codec_type":"video","width":1920,"height":1080,"side_data_list":[{"rotation":-90}]}]}"#;
        let tag = r#"{"format":{"format_name":"mov,mp4","duration":"5"},"streams":[
            {"codec_type":"video","width":1920,"height":1080,"tags":{"rotate":"270"}}]}"#;
        let upside_down = r#"{"format":{"format_name":"mov,mp4","duration":"5"},"streams":[
            {"codec_type":"video","width":1920,"height":1080,"side_data_list":[{"rotation":180}]}]}"#;
        for json in [side, tag] {
            let v = validate(&probe(json), &LIMITS).unwrap();
            assert_eq!((v.width, v.height), (1080, 1920), "{json}");
            assert!((v.display_aspect - 9.0 / 16.0).abs() < 1e-9);
        }
        let v = validate(&probe(upside_down), &LIMITS).unwrap();
        assert_eq!(
            (v.width, v.height),
            (1920, 1080),
            "180 degrees keeps the shape"
        );
    }

    #[test]
    fn non_square_pixels_change_the_display_aspect_not_the_stored_size() {
        let anamorphic = r#"{"format":{"format_name":"avi","duration":"5"},"streams":[
            {"codec_type":"video","width":720,"height":576,"sample_aspect_ratio":"64:45"}]}"#;
        let v = validate(&probe(anamorphic), &LIMITS).unwrap();
        assert_eq!((v.width, v.height), (720, 576));
        assert!((v.display_aspect - 720.0 * 64.0 / 45.0 / 576.0).abs() < 1e-9);
    }

    #[test]
    fn cover_art_is_not_a_video_stream() {
        let mp3 = r#"{"format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"200"},"streams":[
            {"codec_type":"audio"},
            {"codec_type":"video","width":500,"height":500,"disposition":{"attached_pic":1}}]}"#;
        assert_eq!(code(validate(&probe(mp3), &LIMITS)), "no_video_stream");
    }

    #[test]
    fn containers_that_can_reference_other_resources_are_refused() {
        for name in [
            "hls,applehttp",
            "concat",
            "image2",
            "lavfi",
            "data",
            "sdp",
            "rtsp",
            "mov,mp4,m4a,3gp,3g2,mj2,evil",
            "",
        ] {
            let json = format!(
                r#"{{"format":{{"format_name":"{name}","duration":"5"}},"streams":[{{"codec_type":"video","width":640,"height":360}}]}}"#
            );
            assert_eq!(
                code(validate(&probe(&json), &LIMITS)),
                "unsupported_media",
                "{name:?}"
            );
        }
        for name in ["matroska,webm", "mpegts", "avi", "flv", "asf"] {
            let json = format!(
                r#"{{"format":{{"format_name":"{name}","duration":"5"}},"streams":[{{"codec_type":"video","width":640,"height":360}}]}}"#
            );
            assert!(validate(&probe(&json), &LIMITS).is_ok(), "{name}");
        }
    }

    #[test]
    fn limits_on_duration_and_resolution_are_enforced() {
        let long = MP4.replace("12.480000", "3600.5");
        assert_eq!(code(validate(&probe(&long), &LIMITS)), "too_long");
        let ok_at_limit = MP4.replace("12.480000", "3600.0");
        assert!(validate(&probe(&ok_at_limit), &LIMITS).is_ok());

        let wide = MP4.replace("1920", "8192");
        assert_eq!(
            code(validate(&probe(&wide), &LIMITS)),
            "resolution_too_high"
        );
        let tall_when_rotated = r#"{"format":{"format_name":"mov,mp4","duration":"5"},"streams":[
            {"codec_type":"video","width":4097,"height":1080,"side_data_list":[{"rotation":90}]}]}"#;
        assert_eq!(
            code(validate(&probe(tall_when_rotated), &LIMITS)),
            "resolution_too_high"
        );
    }

    #[test]
    fn absurd_stream_counts_and_empty_dimensions_are_refused() {
        let streams = vec![r#"{"codec_type":"data"}"#; 40].join(",");
        let json = format!(
            r#"{{"format":{{"format_name":"matroska,webm","duration":"5"}},"streams":[{{"codec_type":"video","width":640,"height":360}},{streams}]}}"#
        );
        assert_eq!(code(validate(&probe(&json), &LIMITS)), "unsupported_media");
        let zero = r#"{"format":{"format_name":"mov,mp4","duration":"5"},"streams":[{"codec_type":"video","width":0,"height":0}]}"#;
        assert_eq!(code(validate(&probe(zero), &LIMITS)), "no_video_stream");
    }

    #[test]
    fn a_missing_duration_falls_back_to_streams_and_is_otherwise_tolerated() {
        let from_stream = r#"{"format":{"format_name":"matroska,webm"},"streams":[
            {"codec_type":"video","width":640,"height":360,"duration":"7.5"}]}"#;
        assert_eq!(probe(from_stream).duration_secs, Some(7.5));
        let none = r#"{"format":{"format_name":"matroska,webm","duration":"N/A"},"streams":[
            {"codec_type":"video","width":640,"height":360}]}"#;
        let p = probe(none);
        assert_eq!(p.duration_secs, None);
        assert!(
            validate(&p, &LIMITS).is_ok(),
            "the encode itself is capped instead"
        );
    }

    #[test]
    fn garbage_probe_output_is_an_unsupported_file_not_a_crash() {
        for junk in ["", "not json", "[]", "null", "{\"format\":5}"] {
            match parse(junk) {
                Err(JobError::Permanent { code, .. }) => assert_eq!(code, "unsupported_media"),
                Ok(p) => assert_eq!(code(validate(&p, &LIMITS)), "unsupported_media", "{junk}"),
                Err(other) => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn frame_rates_and_ratios_parse_defensively() {
        assert_eq!(parse_ratio(Some("30/1")), Some(30.0));
        assert_eq!(parse_ratio(Some("0/0")), None);
        assert_eq!(parse_ratio(Some("1:0")), None);
        assert_eq!(parse_ratio(Some("N/A")), None);
        assert_eq!(parse_ratio(Some("")), None);
        assert_eq!(parse_ratio(None), None);
    }
}
