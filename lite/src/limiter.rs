//! How often one person may ping: spamming is the point, but only so much of it.

use std::collections::VecDeque;

/// At most `max` events in any `window_ms`, and never two closer than `min_gap_ms`.
#[derive(Debug, Clone)]
pub struct Limiter {
    min_gap_ms: u64,
    max: usize,
    window_ms: u64,
    times: VecDeque<u64>,
}

/// What a sender allows itself: one every 100 ms, twenty in any 5 s.
pub const SEND: (u64, usize, u64) = (100, 20, 5000);
/// What a receiver lets through from one origin: the same, with slack for network jitter bunching packets up.
pub const RECEIVE: (u64, usize, u64) = (50, 20, 4500);

impl Limiter {
    pub fn new((min_gap_ms, max, window_ms): (u64, usize, u64)) -> Self {
        Self { min_gap_ms, max, window_ms, times: VecDeque::with_capacity(max) }
    }

    /// Counts the event and says yes, or says no and counts nothing.
    pub fn allow(&mut self, now: u64) -> bool {
        if self.times.back().is_some_and(|&last| now < last + self.min_gap_ms) {
            return false;
        }
        while self.times.front().is_some_and(|&t| t + self.window_ms <= now) {
            self.times.pop_front();
        }
        if self.times.len() >= self.max {
            return false;
        }
        self.times.push_back(now);
        true
    }

    /// Nothing counted any more: the limiter can be forgotten.
    pub fn idle(&self, now: u64) -> bool {
        self.times.back().is_none_or(|&t| t + self.window_ms <= now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_events_by_the_gap() {
        let mut l = Limiter::new(SEND);
        assert!(l.allow(1000));
        assert!(!l.allow(1050));
        assert!(!l.allow(1099));
        assert!(l.allow(1100));
    }

    #[test]
    fn allows_twenty_in_any_five_seconds() {
        let mut l = Limiter::new(SEND);
        for i in 0..20 {
            assert!(l.allow(10_000 + i * 100), "{i}");
        }
        assert!(!l.allow(12_000));
        assert!(!l.allow(14_999));
        // The first one leaves the window at 15 000; the next at 15 100.
        assert!(l.allow(15_000));
        assert!(!l.allow(15_050));
        assert!(!l.allow(15_099));
        assert!(l.allow(15_100));
    }

    #[test]
    fn a_refused_event_does_not_count() {
        let mut l = Limiter::new((100, 1, 1000));
        assert!(l.allow(0));
        for t in (100..1000).step_by(100) {
            assert!(!l.allow(t));
        }
        assert!(l.allow(1000));
    }

    #[test]
    fn steady_pinging_settles_at_four_a_second() {
        let mut l = Limiter::new(SEND);
        let sent = (0..60_000).step_by(100).filter(|&t| l.allow(t)).count();
        assert_eq!(sent, 240);
    }

    #[test]
    fn idles_once_the_window_passes() {
        let mut l = Limiter::new(SEND);
        assert!(l.idle(0));
        l.allow(0);
        assert!(!l.idle(4999));
        assert!(l.idle(5000));
    }
}
