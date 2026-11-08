//! The rules for turning heartbeats into progress. Pure, so they can be tested exhaustively.
//!
//! A heartbeat says "I am at second N". The server never takes "I finished" on trust. Three
//! numbers are kept:
//!
//! * `last_position`, where the viewer is: resume goes here, wherever they seeked to;
//! * `furthest`, the highest position reported;
//! * `watched`, credible watch time. Moving forward earns credit only up to what the clock allows
//!   (twice real time, to permit 2x playback, plus a little slack, and never more than a couple of
//!   minutes at once). Seeking from minute 3 to minute 55 earns a few seconds, not 52 minutes.
//!
//! A lesson is *completed* once `watched` and `furthest` both reach 90% of its length. Completion
//! is sticky: watching it again later never un-completes it.

/// A lesson counts as watched at this percentage of its length.
pub const COMPLETE_PERCENT: i64 = 90;
/// Fastest believable playback, as a multiple of real time.
const MAX_RATE: i64 = 2;
/// Allowance for timer jitter and batching, in seconds.
const SLACK_SECS: i64 = 5;
/// The most credit one heartbeat can earn, however long it has been.
const MAX_CREDIT_SECS: i64 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stored {
    pub last_position: i32,
    pub furthest: i32,
    pub watched: i32,
    pub completed: bool,
}

impl Stored {
    /// Progress for a lesson seen for the first time at `position`. Nothing is credited yet: a
    /// first report proves where the viewer is, not what they watched to get there.
    pub fn first(position: i32) -> Self {
        Self {
            last_position: position,
            furthest: position,
            watched: 0,
            completed: false,
        }
    }
}

/// Folds one heartbeat into the stored state. `elapsed_secs` is server time since the previous
/// heartbeat for this lesson; `duration` is the video length when known.
pub fn advance(prev: Stored, position: i32, elapsed_secs: i64, duration: Option<i32>) -> Stored {
    let position = match duration {
        Some(d) => position.clamp(0, d.max(0)),
        None => position.max(0),
    };
    let allowed = elapsed_secs
        .max(0)
        .saturating_mul(MAX_RATE)
        .saturating_add(SLACK_SECS)
        .min(MAX_CREDIT_SECS);
    let forward = i64::from(position) - i64::from(prev.last_position);
    let credit = forward.clamp(0, allowed);
    let mut watched = i64::from(prev.watched) + credit;
    if let Some(d) = duration {
        watched = watched.min(i64::from(d.max(0)));
    }
    let watched = i32::try_from(watched).unwrap_or(i32::MAX);
    let furthest = prev.furthest.max(position);
    let completed = prev.completed
        || duration.is_some_and(|d| {
            let threshold = i64::from(d) * COMPLETE_PERCENT;
            d > 0 && i64::from(watched) * 100 >= threshold && i64::from(furthest) * 100 >= threshold
        });
    Stored {
        last_position: position,
        furthest,
        watched,
        completed,
    }
}

/// Whole percent, 0 to 100, of `part` out of `total`; 0 when the length is unknown.
pub fn percent(part: i32, total: Option<i32>) -> u8 {
    match total {
        Some(total) if total > 0 => {
            u8::try_from((i64::from(part.max(0)) * 100 / i64::from(total)).min(100)).unwrap_or(100)
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i32 = 3600;

    fn watch_honestly(duration: i32, rate: i32, heartbeat_secs: i64) -> Stored {
        let mut state = Stored::first(0);
        let mut position = 0;
        while position < duration {
            position = (position + (heartbeat_secs as i32) * rate).min(duration);
            state = advance(state, position, heartbeat_secs, Some(duration));
        }
        state
    }

    #[test]
    fn watching_straight_through_completes_at_any_normal_speed() {
        for rate in [1, 2] {
            let state = watch_honestly(HOUR, rate, 30);
            assert!(state.completed, "rate {rate}: {state:?}");
            assert_eq!(state.watched, HOUR);
            assert_eq!(state.last_position, HOUR);
        }
    }

    #[test]
    fn a_first_report_credits_nothing_and_resuming_late_does_not_either() {
        let state = Stored::first(1500);
        assert_eq!(
            state,
            Stored {
                last_position: 1500,
                furthest: 1500,
                watched: 0,
                completed: false
            }
        );
        // Back the next day: the first heartbeat reports where they resumed.
        let next = advance(state, 1500, 86_400, Some(HOUR));
        assert_eq!((next.watched, next.last_position), (0, 1500));
    }

    #[test]
    fn dragging_the_slider_to_the_end_earns_almost_nothing() {
        let state = advance(Stored::first(0), 30, 30, Some(HOUR));
        let skipped = advance(state, HOUR, 30, Some(HOUR));
        assert!(skipped.watched <= 30 + 65, "{skipped:?}");
        assert_eq!(skipped.furthest, HOUR);
        assert!(!skipped.completed, "furthest alone is not completion");
        assert_eq!(skipped.last_position, HOUR, "but resume follows them");
    }

    #[test]
    fn a_heartbeat_burst_cannot_manufacture_watch_time() {
        let mut state = Stored::first(0);
        for step in 1..=200 {
            state = advance(state, step * 60, 1, Some(10 * HOUR));
        }
        // 200 heartbeats one second apart can account for at most ~7 seconds each.
        assert!(state.watched <= 200 * 7, "{state:?}");
    }

    #[test]
    fn rewinding_never_loses_credit_and_position_follows() {
        let mut state = watch_honestly(600, 1, 30);
        assert!(state.completed);
        state = advance(state, 100, 30, Some(600));
        assert_eq!(state.last_position, 100);
        assert_eq!(state.watched, 600);
        assert!(state.completed, "completion is sticky");
    }

    #[test]
    fn positions_are_clamped_to_the_video() {
        let state = advance(Stored::first(10), 9_999_999, 30, Some(100));
        assert_eq!(state.last_position, 100);
        assert_eq!(state.furthest, 100);
        let state = advance(Stored::first(10), -50, 30, Some(100));
        assert_eq!(state.last_position, 0);
    }

    #[test]
    fn watched_never_exceeds_the_length_even_when_looping_a_clip() {
        let mut state = Stored::first(0);
        for _ in 0..500 {
            state = advance(state, 25, 30, Some(60));
            state = advance(state, 0, 30, Some(60));
        }
        assert!(state.watched <= 60, "{state:?}");
    }

    #[test]
    fn completion_needs_both_watching_and_reaching_the_end() {
        // Watched 95% by looping the first half over and over, never got near the end.
        let mut state = Stored::first(0);
        for _ in 0..40 {
            state = advance(state, 0, 30, Some(1000));
            state = advance(state, 40, 30, Some(1000));
        }
        assert!(state.watched >= 900, "plenty of credit: {state:?}");
        assert!(state.furthest <= 40, "never went anywhere: {state:?}");
        assert!(!state.completed, "{state:?}");
    }

    #[test]
    fn unknown_length_never_completes_and_never_panics() {
        let mut state = Stored::first(0);
        for step in 1..50 {
            state = advance(state, step * 30, 30, None);
        }
        assert!(!state.completed);
        assert!(state.watched > 0);
    }

    #[test]
    fn weird_clocks_are_harmless() {
        let negative = advance(Stored::first(0), 30, -1_000_000, Some(HOUR));
        assert!(negative.watched <= SLACK_SECS as i32);
        let huge = advance(Stored::first(0), 3000, i64::MAX, Some(HOUR));
        assert!(i64::from(huge.watched) <= MAX_CREDIT_SECS);
    }

    #[test]
    fn percentages_are_whole_and_bounded() {
        assert_eq!(percent(0, Some(100)), 0);
        assert_eq!(percent(45, Some(100)), 45);
        assert_eq!(percent(999, Some(100)), 100);
        assert_eq!(percent(-5, Some(100)), 0);
        assert_eq!(percent(50, None), 0);
        assert_eq!(percent(50, Some(0)), 0);
    }
}
