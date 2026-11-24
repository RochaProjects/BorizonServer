//! Server-owned runtime primitives that do not depend on client presentation.
//!
//! The engine's current dedicated runner is still hosted by `horizon-client`,
//! but timing and pause policy used by its live loop live here so they can be
//! exercised without compiling or constructing a client runtime.
use std::time::{Duration, Instant};

pub mod chunk_jobs;
mod preflight;
pub mod work_queue;
pub use preflight::{ServerSetup, preflight};

/// The action the server loop takes after a tick to preserve vanilla pacing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickPacing {
    /// Tick sprinting bypasses the normal 20 TPS deadline.
    RunImmediately,
    /// The server is on schedule (or less than two seconds behind).
    Continue,
    /// Keep workers fed, then sleep only if this deadline is still in the future.
    WaitUntil(Instant),
    /// More than two seconds behind; skip the accumulated ticks and report this lag.
    Overloaded(Duration),
}

/// Advance the next tick deadline using the rules shared by the dedicated and
/// integrated server loops. `now` is injected so boundary behavior is testable.
/// A tick exactly two seconds behind is retained; only greater lag is skipped.
pub fn advance_tick_schedule(
    next: &mut Instant,
    now: Instant,
    tick_duration: Duration,
    paused: bool,
    sprinting: bool,
) -> TickPacing {
    if !paused && sprinting {
        *next = now;
        return TickPacing::RunImmediately;
    }

    *next += tick_duration;
    if *next > now {
        TickPacing::WaitUntil(*next)
    } else if now.duration_since(*next) > Duration::from_secs(2) {
        let behind = now.duration_since(*next);
        *next = now;
        TickPacing::Overloaded(behind)
    } else {
        TickPacing::Continue
    }
}

/// `MinecraftServer.tickServer` pause-when-empty accounting. A player, a
/// pre-generation task, or `/tick sprint` resets the counter; a zero/negative
/// threshold disables pausing and leaves the counter unchanged, matching the
/// engine's existing dedicated-server behavior.
pub fn count_empty_tick(
    empty_ticks: &mut i32,
    pause_after: i32,
    players: usize,
    pregen: bool,
    sprinting: bool,
) -> bool {
    if pause_after > 0 {
        if players == 0 && !pregen && !sprinting {
            *empty_ticks += 1;
        } else {
            *empty_ticks = 0;
        }
    }
    pause_after > 0 && *empty_ticks >= pause_after
}

/// Vanilla's overload warning text, including completed 50-ms ticks behind.
pub fn overload_warning(behind: Duration, tick_duration: Duration) -> String {
    format!(
        "Can't keep up! Is the server overloaded? Running {}ms or {} ticks behind",
        behind.as_millis(),
        behind.as_nanos() / tick_duration.as_nanos().max(1)
    )
}

#[cfg(test)]
mod tests {
    use super::{TickPacing, advance_tick_schedule, count_empty_tick, overload_warning};
    use std::time::{Duration, Instant};

    #[test]
    fn regular_ticks_advance_by_the_tick_duration() {
        let now = Instant::now();
        let mut next = now;
        assert_eq!(
            advance_tick_schedule(&mut next, now, Duration::from_millis(50), false, false),
            TickPacing::WaitUntil(now + Duration::from_millis(50))
        );
        assert_eq!(next, now + Duration::from_millis(50));
    }

    #[test]
    fn sprint_resets_deadline_but_does_not_bypass_pause() {
        let now = Instant::now();
        let mut next = now - Duration::from_secs(1);
        assert_eq!(
            advance_tick_schedule(&mut next, now, Duration::from_millis(50), false, true),
            TickPacing::RunImmediately
        );
        assert_eq!(next, now);

        assert_eq!(
            advance_tick_schedule(&mut next, now, Duration::from_millis(50), true, true),
            TickPacing::WaitUntil(now + Duration::from_millis(50))
        );
    }

    #[test]
    fn exactly_two_seconds_late_is_not_skipped_but_more_is() {
        let epoch = Instant::now();
        let mut next = epoch;
        assert_eq!(
            advance_tick_schedule(
                &mut next,
                epoch + Duration::from_millis(2_050),
                Duration::from_millis(50),
                false,
                false
            ),
            TickPacing::Continue
        );
        assert_eq!(next, epoch + Duration::from_millis(50));

        next = epoch;
        assert_eq!(
            advance_tick_schedule(
                &mut next,
                epoch + Duration::from_millis(2_051),
                Duration::from_millis(50),
                false,
                false
            ),
            TickPacing::Overloaded(Duration::from_millis(2_001))
        );
        assert_eq!(next, epoch + Duration::from_millis(2_051));
    }

    #[test]
    fn pause_when_empty_preserves_sprint_and_pregen_semantics() {
        let pause_after = 60 * 20;
        let mut empty_ticks = 0;
        for _ in 0..2000 {
            assert!(!count_empty_tick(
                &mut empty_ticks,
                pause_after,
                0,
                false,
                true
            ));
        }
        assert_eq!(empty_ticks, 0);
        for n in 1..=pause_after {
            assert_eq!(
                count_empty_tick(&mut empty_ticks, pause_after, 0, false, false),
                n == pause_after,
                "tick {n}"
            );
        }
        assert!(!count_empty_tick(
            &mut empty_ticks,
            pause_after,
            0,
            false,
            true
        ));
        assert_eq!(empty_ticks, 0);
        empty_ticks = 5;
        assert!(!count_empty_tick(
            &mut empty_ticks,
            pause_after,
            1,
            false,
            false
        ));
        assert_eq!(empty_ticks, 0);
        empty_ticks = 5;
        assert!(!count_empty_tick(
            &mut empty_ticks,
            pause_after,
            0,
            true,
            false
        ));
        assert_eq!(empty_ticks, 0);
        empty_ticks = 5;
        assert!(!count_empty_tick(&mut empty_ticks, 0, 0, false, false));
        assert_eq!(empty_ticks, 5);
    }

    #[test]
    fn overload_warning_keeps_vanilla_text() {
        assert_eq!(
            overload_warning(Duration::from_millis(13_027), Duration::from_millis(50)),
            "Can't keep up! Is the server overloaded? Running 13027ms or 260 ticks behind"
        );
    }
}
