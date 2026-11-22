//! The set of renditions produced for a source: 144p to 720p, never above the source.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rung {
    pub name: String,
    pub height: u32,
    pub crf: u8,
    pub maxrate_kbps: u32,
    pub audio_kbps: u32,
}

/// (height, crf, video max rate kbps, audio kbps), lowest first.
const LADDER: [(u32, u8, u32, u32); 5] = [
    (144, 28, 200, 48),
    (240, 27, 400, 64),
    (360, 26, 800, 96),
    (480, 25, 1400, 96),
    (720, 24, 2800, 128),
];

pub const AUDIO_ONLY_KBPS: u32 = 64;

/// Rungs whose height does not exceed the source's, lowest first. A source smaller than the
/// lowest rung gets a single rendition at its own (even) height instead of being upscaled.
pub fn choose(source_height: u32) -> Vec<Rung> {
    let mut rungs: Vec<Rung> = LADDER
        .iter()
        .filter(|(height, ..)| *height <= source_height)
        .map(|&(height, crf, maxrate_kbps, audio_kbps)| Rung {
            name: format!("{height}p"),
            height,
            crf,
            maxrate_kbps,
            audio_kbps,
        })
        .collect();
    if rungs.is_empty() {
        let height = (source_height & !1).max(2);
        let (_, crf, maxrate_kbps, audio_kbps) = LADDER[0];
        rungs.push(Rung {
            name: format!("{height}p"),
            height,
            crf,
            maxrate_kbps,
            audio_kbps,
        });
    }
    rungs
}

/// The height the first rendition aims at: sharp enough to be watched properly on a laptop, small enough to
/// be encoded in a fraction of the video's length. People can watch as soon as this one is published.
pub const FIRST_PLAY_HEIGHT: u32 = 360;

/// Which rung is encoded and published first: the largest that does not exceed `FIRST_PLAY_HEIGHT`, or the
/// smallest there is when the source is smaller than that is. `rungs` is lowest first and not empty.
pub fn first_play_index(rungs: &[Rung]) -> usize {
    rungs
        .iter()
        .rposition(|rung| rung.height <= FIRST_PLAY_HEIGHT)
        .unwrap_or(0)
}

/// The rungs left after the first one is out, in the order they are worth waiting for: the ones above it
/// first, nearest first (people on a good connection see a better picture sooner), then the ones below it,
/// highest first (they only matter to a viewer whose connection is poor).
pub fn rest_in_order(rungs: &[Rung], first: usize) -> Vec<Rung> {
    let above = rungs.iter().skip(first + 1).cloned();
    let below = rungs.iter().take(first).rev().cloned();
    above.chain(below).collect()
}

/// Output width for a given height and display aspect ratio, rounded down to an even number
/// as H.264 4:2:0 requires.
pub fn display_width(height: u32, display_aspect: f64) -> u32 {
    // The epsilon keeps products like 720 * 16/9 = 1279.9999999999998 from rounding down.
    let width = ((f64::from(height) * display_aspect / 2.0) + 1e-9).floor() as u32 * 2;
    width.max(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heights(source: u32) -> Vec<u32> {
        choose(source).iter().map(|r| r.height).collect()
    }

    #[test]
    fn rungs_never_exceed_the_source_and_cap_at_720() {
        assert_eq!(heights(2160), [144, 240, 360, 480, 720]);
        assert_eq!(heights(1080), [144, 240, 360, 480, 720]);
        assert_eq!(heights(720), [144, 240, 360, 480, 720]);
        assert_eq!(heights(719), [144, 240, 360, 480]);
        assert_eq!(heights(360), [144, 240, 360]);
        assert_eq!(heights(144), [144]);
        assert_eq!(heights(143), [142]);
    }

    #[test]
    fn the_first_rendition_is_the_largest_up_to_360p_and_the_rest_follow_in_order_of_use() {
        let names = |rungs: &[Rung]| rungs.iter().map(|r| r.height).collect::<Vec<_>>();
        for (source, first, rest) in [
            (1080, 360, vec![480, 720, 240, 144]),
            (720, 360, vec![480, 720, 240, 144]),
            (480, 360, vec![480, 240, 144]),
            (360, 360, vec![240, 144]),
            (240, 240, vec![144]),
            (144, 144, vec![]),
            (100, 100, vec![]),
        ] {
            let rungs = choose(source);
            let at = first_play_index(&rungs);
            assert_eq!(rungs[at].height, first, "source {source}");
            assert_eq!(names(&rest_in_order(&rungs, at)), rest, "source {source}");
        }
    }

    #[test]
    fn tiny_sources_get_one_even_height_rendition_and_never_zero() {
        assert_eq!(heights(100), [100]);
        assert_eq!(heights(101), [100]);
        assert_eq!(heights(1), [2]);
        assert_eq!(heights(0), [2]);
        assert_eq!(choose(100)[0].name, "100p");
    }

    #[test]
    fn bitrates_rise_with_the_ladder() {
        let rungs = choose(1080);
        assert!(rungs
            .windows(2)
            .all(|w| w[0].maxrate_kbps < w[1].maxrate_kbps));
        assert!(rungs.windows(2).all(|w| w[0].crf > w[1].crf));
    }

    #[test]
    fn widths_follow_the_display_aspect_ratio_and_stay_even() {
        assert_eq!(display_width(720, 16.0 / 9.0), 1280);
        assert_eq!(display_width(144, 16.0 / 9.0), 256);
        assert_eq!(display_width(480, 4.0 / 3.0), 640);
        assert_eq!(display_width(640, 9.0 / 16.0), 360);
        assert_eq!(display_width(100, 0.001), 2);
        for h in [144, 240, 360, 480, 720] {
            assert_eq!(display_width(h, 1.7777) % 2, 0);
        }
    }
}
