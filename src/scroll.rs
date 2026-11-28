//! Browser-like scrolling, GUI-free: smooth wheel steps, touchpad flings and
//! the scrollbar geometry of a list whose rows are only partly measured.
//!
//! Distances are in pixels, positive toward the end of the document.

use std::{
    collections::VecDeque,
    ops::Range,
    time::{Duration, Instant},
};

/// Pixels per wheel line (GPUI's own list uses the same).
pub const LINE_HEIGHT: f32 = 20.;
/// How long a wheel step glides, in seconds.
const GLIDE_SECS: f32 = 0.15;
/// Time constant of a fling's exponential slowdown, in seconds.
const FLING_TAU: f32 = 0.35;
/// Slowest release (px/s) that starts a fling.
const FLING_MIN_START: f32 = 150.;
/// A fling stops below this speed (px/s).
const FLING_STOP: f32 = 15.;
/// Fastest fling (px/s).
const FLING_MAX: f32 = 15_000.;
/// Touchpad samples older than this before the release don't count toward
/// the fling speed, so stopping the fingers before lifting them stops dead.
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);

/// Scroll motion of one list: wheel glides and touchpad flings, advanced
/// once per frame by [`Motion::tick`].
#[derive(Debug, Default)]
pub struct Motion {
    anim: Option<Anim>,
    /// Pixels a capped `tick` held back for the next one.
    carry: f32,
    /// Recent touchpad deltas, for the release speed.
    samples: VecDeque<(Instant, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Anim {
    /// Ease-out to `distance` over `GLIDE_SECS`.
    Glide {
        distance: f32,
        start: Instant,
        done: f32,
    },
    /// Exponential slowdown from `velocity`.
    Fling {
        velocity: f32,
        start: Instant,
        done: f32,
    },
}

impl Motion {
    /// A wheel step of `dy` pixels: glides there, on top of what's left of
    /// the running glide either way, so fast spins add up and a step back
    /// returns exactly to where the first started (like GPUI's own handler,
    /// and Firefox). A fling is dropped.
    pub fn wheel(&mut self, dy: f32, now: Instant) {
        let left = match self.anim {
            Some(Anim::Glide { distance, done, .. }) => distance - done + self.carry,
            None => self.carry,
            Some(Anim::Fling { .. }) => 0.,
        };
        self.carry = 0.;
        self.samples.clear();
        self.anim = Some(Anim::Glide {
            distance: left + dy,
            start: now,
            done: 0.,
        });
    }

    /// A touchpad delta: scrolls `dy` now (the return value) and stops any
    /// motion, since the fingers are in control.
    pub fn touch(&mut self, dy: f32, now: Instant) -> f32 {
        self.anim = None;
        self.carry = 0.;
        while self
            .samples
            .front()
            .is_some_and(|&(t, _)| now.saturating_duration_since(t) > VELOCITY_WINDOW)
        {
            self.samples.pop_front();
        }
        self.samples.push_back((now, dy));
        dy
    }

    /// The fingers left the touchpad: fling on at the release speed.
    pub fn touch_end(&mut self, now: Instant) {
        let velocity = self.release_velocity(now);
        self.samples.clear();
        if velocity.abs() >= FLING_MIN_START {
            self.anim = Some(Anim::Fling {
                velocity: velocity.clamp(-FLING_MAX, FLING_MAX),
                start: now,
                done: 0.,
            });
        }
    }

    /// Speed (px/s) of the touchpad samples within `VELOCITY_WINDOW` of
    /// `now`: the distance after the first sample over the time since it.
    fn release_velocity(&self, now: Instant) -> f32 {
        let mut recent = self
            .samples
            .iter()
            .filter(|&&(t, _)| now.saturating_duration_since(t) <= VELOCITY_WINDOW);
        let Some(&(first, _)) = recent.next() else {
            return 0.;
        };
        let (last, distance) = recent.fold((first, 0.), |(_, d), &(t, dy)| (t, d + dy));
        let secs = last.duration_since(first).as_secs_f32();
        if secs < 0.001 { 0. } else { distance / secs }
    }

    /// Limits what's left of a glide to `min..=max` px from here: the room
    /// to the document's edges. A destination past an edge would otherwise
    /// absorb a following step back (5 px from the top, one step up and one
    /// down must end 20 px down, not 5). A fling ends at the edge anyway.
    ///
    /// The carry a capped `tick` held back counts too, also once the glide
    /// itself is over. What's kept of the glide is scaled down, not cut off
    /// at the end, so it never turns back on its way. It only ever shrinks:
    /// the room always includes where it is now (`min <= 0 <= max`).
    pub fn limit(&mut self, min: f32, max: f32) {
        let left = match self.anim {
            Some(Anim::Fling { .. }) => return,
            Some(Anim::Glide { distance, done, .. }) => distance - done,
            None => 0.,
        };
        let rest = left + self.carry;
        let target = rest.clamp(min.min(0.), max.max(0.));
        if target == rest {
            return;
        }
        // The glide keeps what fits of it, the carry the rest.
        let keep = if left * target > 0. {
            left.abs().min(target.abs()).copysign(left)
        } else {
            0.
        };
        self.carry = target - keep;
        if let Some(Anim::Glide { distance, done, .. }) = &mut self.anim {
            if keep == 0. {
                self.anim = None;
            } else {
                // Each later step shrinks by the same factor.
                let k = keep / left;
                *distance *= k;
                *done *= k;
            }
        }
    }

    pub fn stop(&mut self) {
        *self = Self::default();
    }

    pub fn is_active(&self) -> bool {
        self.anim.is_some() || self.carry != 0.
    }

    /// Which way the motion goes from here: -1, 0 or 1. For a glide, that's
    /// the way of what's left of it, carry included, not of its start.
    pub fn direction(&self) -> f32 {
        let d = match self.anim {
            Some(Anim::Glide { distance, done, .. }) => distance - done + self.carry,
            Some(Anim::Fling { velocity, .. }) => velocity,
            None => self.carry,
        };
        if d == 0. { 0. } else { d.signum() }
    }

    /// The pixels to scroll in the frame drawn at `now`, at most `max_step`
    /// either way; the rest waits for the next frame.
    #[cfg(test)]
    pub fn tick(&mut self, now: Instant, max_step: f32) -> f32 {
        self.tick_within(now, -max_step, max_step)
    }

    /// The pixels to scroll in the frame drawn at `now`, within
    /// `min..=max` (`min <= 0 <= max`); the rest waits for the next frame.
    pub fn tick_within(&mut self, now: Instant, min: f32, max: f32) -> f32 {
        let mut step = std::mem::take(&mut self.carry);
        match &mut self.anim {
            Some(Anim::Glide {
                distance,
                start,
                done,
            }) => {
                let t = (now.saturating_duration_since(*start).as_secs_f32() / GLIDE_SECS).min(1.);
                let target = *distance * (1. - (1. - t).powi(3));
                step += target - *done;
                *done = target;
                if t >= 1. {
                    self.anim = None;
                }
            }
            Some(Anim::Fling {
                velocity,
                start,
                done,
            }) => {
                let decay =
                    (-now.saturating_duration_since(*start).as_secs_f32() / FLING_TAU).exp();
                let target = *velocity * FLING_TAU * (1. - decay);
                step += target - *done;
                *done = target;
                if (*velocity * decay).abs() < FLING_STOP {
                    self.anim = None;
                }
            }
            None => {}
        }
        let capped = step.clamp(min, max);
        self.carry = step - capped;
        capped
    }
}

/// Row height assumed before any row is measured.
const DEFAULT_ROW_HEIGHT: f32 = 60.;

/// The rows before some index: how many, how many are measured, and the
/// measured ones' total height.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Measured {
    pub count: usize,
    pub measured: usize,
    pub height: f32,
}

impl Measured {
    /// Average measured row height, the estimate for unmeasured rows.
    pub fn average(&self) -> f32 {
        if self.measured == 0 {
            DEFAULT_ROW_HEIGHT
        } else {
            self.height / self.measured as f32
        }
    }

    /// Height of these rows, with `avg` for each unmeasured one.
    pub fn estimate(&self, avg: f32) -> f32 {
        self.height + self.count.saturating_sub(self.measured) as f32 * avg
    }
}

/// Shortest scrollbar thumb.
pub const MIN_THUMB: f32 = 24.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thumb {
    pub top: f32,
    pub len: f32,
}

/// The thumb of a `track` pixels long scrollbar, for a `viewport` showing
/// `content` from `top` on. None when everything fits.
pub fn thumb(top: f32, content: f32, viewport: f32, track: f32) -> Option<Thumb> {
    if content <= viewport + 0.5 || track <= 0. {
        return None;
    }
    let len = (track * viewport / content).max(MIN_THUMB.min(track));
    let frac = (top / (content - viewport)).clamp(0., 1.);
    Some(Thumb {
        top: frac * (track - len),
        len,
    })
}

/// The content position (top of the viewport) when the thumb is at
/// `thumb_top`. The inverse of [`thumb`].
pub fn position_for_thumb(
    thumb_top: f32,
    len: f32,
    content: f32,
    viewport: f32,
    track: f32,
) -> f32 {
    let room = track - len;
    let frac = if room > 0. {
        (thumb_top / room).clamp(0., 1.)
    } else {
        0.
    };
    frac * (content - viewport).max(0.)
}

/// How far a list can scroll each way from its scroll top without
/// crossing a row that isn't measured, and whether that's because the
/// document ends there (rather than unmeasured rows that the next frame
/// will measure).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Room {
    pub up: f32,
    pub down: f32,
    /// Only measured rows up to the top of the document.
    pub top: bool,
    /// Only measured rows down to its end.
    pub bottom: bool,
}

impl Room {
    /// Already at the top (`dir < 0`) or the bottom (`dir > 0`).
    pub fn at_edge(&self, dir: f32) -> bool {
        if dir < 0. {
            self.top && self.up <= 0.
        } else {
            dir > 0. && self.bottom && self.down <= 0.
        }
    }

    /// Splits a scroll of `dy` px into what can go now and what must wait
    /// for the next frame to measure more rows. Nothing waits to go past an
    /// edge of the document.
    pub fn split(&self, dy: f32) -> (f32, f32) {
        let now = dy.clamp(-self.up, self.down);
        let rest = dy - now;
        let edge = if rest < 0. { self.top } else { self.bottom };
        (now, if edge { 0. } else { rest })
    }
}

/// The rows around row `ix` that are all measured: `start..end` with
/// `start <= ix <= end`, as far as it goes both ways. `measured(i)` is how
/// many of the first `i` rows are measured, for `i` in `0..=count`. It ends
/// at `ix` when row `ix` itself isn't measured, or is past the end.
pub fn measured_run(ix: usize, count: usize, measured: impl Fn(usize) -> usize) -> Range<usize> {
    let ix = ix.min(count);
    let all = |a: usize, b: usize| measured(b) - measured(a) == b - a;
    // Smallest start with rows start..ix all measured.
    let (mut lo, mut hi) = (0, ix);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if all(mid, ix) { hi = mid } else { lo = mid + 1 }
    }
    let start = lo;
    // Largest end with rows ix..end all measured.
    let (mut lo, mut hi) = (ix, count);
    while lo < hi {
        let mid = hi - (hi - lo) / 2;
        if all(ix, mid) { lo = mid } else { hi = mid - 1 }
    }
    start..lo
}

/// The row containing position `pos` and the offset into it, given the
/// (non-decreasing) height of the first `ix` rows, `before(ix)`, for
/// `ix` in `0..=count`.
pub fn locate(pos: f32, count: usize, before: impl Fn(usize) -> f32) -> (usize, f32) {
    // Largest ix in 0..count with before(ix) <= pos.
    let (mut lo, mut hi) = (0, count);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if before(mid) <= pos {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo, (pos - before(lo)).max(0.))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    /// Ticks every 16 ms from `from` until the motion stops; returns the total.
    fn run(m: &mut Motion, t0: Instant, from: u64) -> f32 {
        let mut total = 0.;
        let mut t = from;
        while m.is_active() {
            t += 16;
            total += m.tick(ms(t0, t), f32::INFINITY);
            assert!(t < 60_000, "motion never stops");
        }
        total
    }

    #[test]
    fn wheel_glides_the_whole_distance_easing_out() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.wheel(60., t0);
        assert!(m.is_active());
        assert_eq!(m.direction(), 1.);
        let first = m.tick(ms(t0, 16), f32::INFINITY);
        let second = m.tick(ms(t0, 32), f32::INFINITY);
        assert!(first > second && second > 0., "{first} {second}");
        assert!(first < 60.);
        let rest = run(&mut m, t0, 32);
        assert!((first + second + rest - 60.).abs() < 1e-3);
        // Done after the glide time.
        let mut m = Motion::default();
        m.wheel(60., t0);
        assert_eq!(m.tick(ms(t0, 150), f32::INFINITY), 60.);
        assert!(!m.is_active());
    }

    #[test]
    fn wheel_steps_in_a_row_add_up() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.wheel(60., t0);
        let a = m.tick(ms(t0, 16), f32::INFINITY);
        m.wheel(60., ms(t0, 20));
        let rest = run(&mut m, t0, 20);
        assert!((a + rest - 120.).abs() < 1e-3);
    }

    #[test]
    fn wheel_back_returns_to_the_start() {
        let t0 = Instant::now();
        // Before any frame: the steps cancel out.
        let mut m = Motion::default();
        m.wheel(60., t0);
        m.wheel(-60., t0);
        assert!(run(&mut m, t0, 0).abs() < 1e-3);
        // Mid-glide: back to the start, not 60 px past it.
        let mut m = Motion::default();
        m.wheel(60., t0);
        let a = m.tick(ms(t0, 16), f32::INFINITY);
        m.wheel(-60., ms(t0, 20));
        assert_eq!(m.direction(), -1.);
        assert!((a + run(&mut m, t0, 20)).abs() < 1e-3);
    }

    #[test]
    fn touch_scrolls_directly_and_stops_motion() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.wheel(60., t0);
        assert_eq!(m.touch(7., t0), 7.);
        assert!(!m.is_active());
        assert_eq!(m.tick(ms(t0, 16), f32::INFINITY), 0.);
    }

    /// 20 px every 10 ms = 2000 px/s, then release.
    fn swipe(m: &mut Motion, t0: Instant, dy: f32) {
        for i in 0..10 {
            m.touch(dy, ms(t0, i * 10));
        }
    }

    #[test]
    fn fast_release_flings_and_slows_down() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        swipe(&mut m, t0, 20.);
        assert!((m.release_velocity(ms(t0, 90)) - 2000.).abs() < 1.);
        m.touch_end(ms(t0, 92));
        assert!(m.is_active());
        assert_eq!(m.direction(), 1.);
        let first = m.tick(ms(t0, 108), f32::INFINITY);
        let second = m.tick(ms(t0, 124), f32::INFINITY);
        assert!(first > second && second > 0., "{first} {second}");
        // Total distance tends to v·τ = 700 px.
        let total = first + second + run(&mut m, t0, 124);
        assert!((total - 2000. * FLING_TAU).abs() < 15., "{total}");
    }

    #[test]
    fn flings_go_both_ways() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        swipe(&mut m, t0, -20.);
        m.touch_end(ms(t0, 92));
        assert_eq!(m.direction(), -1.);
        assert!(run(&mut m, t0, 92) < -600.);
    }

    #[test]
    fn pausing_before_release_does_not_fling() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        swipe(&mut m, t0, 20.);
        m.touch_end(ms(t0, 300));
        assert!(!m.is_active());
    }

    #[test]
    fn slow_release_does_not_fling() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        swipe(&mut m, t0, 1.); // 100 px/s
        m.touch_end(ms(t0, 92));
        assert!(!m.is_active());
        // A single sample has no speed.
        let mut m = Motion::default();
        m.touch(50., t0);
        m.touch_end(t0);
        assert!(!m.is_active());
    }

    #[test]
    fn fling_speed_is_capped() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.touch(1000., t0);
        m.touch(1000., ms(t0, 10)); // 100 000 px/s
        m.touch_end(ms(t0, 10));
        let total = run(&mut m, t0, 10);
        assert!((total - FLING_MAX * FLING_TAU).abs() < 15., "{total}");
    }

    #[test]
    fn old_samples_are_dropped() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.touch(500., t0);
        m.touch(20., ms(t0, 200));
        m.touch(20., ms(t0, 210));
        assert_eq!(m.samples.len(), 2);
        assert!((m.release_velocity(ms(t0, 210)) - 2000.).abs() < 1.);
    }

    #[test]
    fn wheel_cancels_a_fling() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        swipe(&mut m, t0, 20.);
        m.touch_end(ms(t0, 92));
        m.wheel(-60., ms(t0, 100));
        assert!((run(&mut m, t0, 100) + 60.).abs() < 1e-3);
    }

    #[test]
    fn capped_ticks_carry_the_rest() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.wheel(-2000., t0);
        assert_eq!(m.tick(ms(t0, 150), 760.), -760.);
        assert!(m.is_active());
        assert_eq!(m.direction(), -1.);
        assert_eq!(m.tick(ms(t0, 166), 760.), -760.);
        assert_eq!(m.tick(ms(t0, 182), 760.), -480.);
        assert!(!m.is_active());
        // A new wheel step keeps the carried rest.
        let mut m = Motion::default();
        m.wheel(2000., t0);
        m.tick(ms(t0, 150), 760.);
        m.wheel(10., ms(t0, 160));
        assert!((run(&mut m, t0, 160) - 1250.).abs() < 1e-3);
    }

    #[test]
    fn limit_keeps_a_glide_within_the_edges() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.wheel(-20., t0);
        let a = m.tick(ms(t0, 16), f32::INFINITY);
        // Started 10 px from the top: the rest shrinks to the room left,
        // so a step back down ends 10 px below where it started.
        assert!(a > -10.);
        m.limit(-(10. + a), 100.);
        m.wheel(20., ms(t0, 20));
        assert!((a + run(&mut m, t0, 20) - 10.).abs() < 1e-3);
        // Within the room it's untouched, carry included.
        let mut m = Motion::default();
        m.wheel(2000., t0);
        m.tick(ms(t0, 50), 100.);
        m.limit(-10., 1900.);
        assert!((100. + run(&mut m, t0, 50) - 2000.).abs() < 1e-3);
        // Down too.
        let mut m = Motion::default();
        m.wheel(300., t0);
        m.limit(-10., 40.);
        assert!((run(&mut m, t0, 0) - 40.).abs() < 1e-3);
        // A fling is left alone.
        let mut m = Motion::default();
        swipe(&mut m, t0, 20.);
        m.touch_end(ms(t0, 92));
        m.limit(0., 0.);
        assert!(run(&mut m, t0, 92) > 600.);
    }

    /// Ticks every 16 ms from `from` until the motion stops; returns each step.
    fn steps(m: &mut Motion, t0: Instant, from: u64, max_step: f32) -> Vec<f32> {
        let mut steps = Vec::new();
        let mut t = from;
        while m.is_active() {
            t += 16;
            steps.push(m.tick(ms(t0, t), max_step));
            assert!(t < 60_000, "motion never stops");
        }
        steps
    }

    #[test]
    fn tick_within_caps_each_way_on_its_own() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.wheel(-500., t0);
        // Room for 300 px up, none down: 300 now, the rest later.
        assert_eq!(m.tick_within(ms(t0, 150), -300., 0.), -300.);
        assert_eq!(m.carry, -200.);
        // No room at all: everything waits.
        assert_eq!(m.tick_within(ms(t0, 166), 0., 1000.), 0.);
        assert_eq!(m.carry, -200.);
        assert!(m.is_active());
        assert_eq!(m.tick_within(ms(t0, 182), -1000., 0.), -200.);
        assert!(!m.is_active());
        // Down the same.
        let mut m = Motion::default();
        m.wheel(500., t0);
        assert_eq!(m.tick_within(ms(t0, 150), -1000., 120.), 120.);
        assert_eq!(m.carry, 380.);
    }

    #[test]
    fn limit_trims_the_carry_of_a_finished_glide() {
        let t0 = Instant::now();
        // The glide is over, but a capped tick held back 1240 px.
        let mut m = Motion::default();
        m.wheel(-2000., t0);
        assert_eq!(m.tick(ms(t0, 150), 760.), -760.);
        assert_eq!(m.anim, None);
        assert_eq!(m.carry, -1240.);
        // Only 100 px are left to the top.
        m.limit(-100., 500.);
        assert_eq!(m.carry, -100.);
        assert_eq!(m.direction(), -1.);
        assert_eq!(run(&mut m, t0, 150), -100.);
        // Down too, and a step back isn't absorbed by the unreachable rest.
        let mut m = Motion::default();
        m.wheel(2000., t0);
        m.tick(ms(t0, 150), 760.);
        m.limit(-500., 40.);
        m.wheel(-60., ms(t0, 160));
        assert!((run(&mut m, t0, 160) + 20.).abs() < 1e-3);
        // At the edge: nothing is left, and the motion is over.
        let mut m = Motion::default();
        m.wheel(2000., t0);
        m.tick(ms(t0, 150), 760.);
        m.limit(-500., 0.);
        assert!(!m.is_active());
        assert_eq!(m.direction(), 0.);
        // A carry within the room is untouched.
        let mut m = Motion::default();
        m.wheel(2000., t0);
        m.tick(ms(t0, 150), 760.);
        m.limit(-500., 5000.);
        assert_eq!(m.carry, 1240.);
    }

    #[test]
    fn a_limited_glide_never_turns_back() {
        let t0 = Instant::now();
        for dir in [1., -1.] {
            // Mid-glide and capped: 1307 px carried, 593 px of glide left.
            let mut m = Motion::default();
            m.wheel(2000. * dir, t0);
            assert_eq!(m.tick(ms(t0, 50), 100.), 100. * dir);
            m.limit(-5., 5.);
            assert_eq!(m.direction(), dir);
            let steps = steps(&mut m, t0, 50, f32::INFINITY);
            assert!(
                steps.iter().all(|s| s * dir >= 0.),
                "a step goes the wrong way: {steps:?}"
            );
            assert!((steps.iter().sum::<f32>() - 5. * dir).abs() < 1e-3);
        }
        // With little carry, the glide keeps easing out to the edge.
        let mut m = Motion::default();
        m.wheel(300., t0);
        let a = m.tick(ms(t0, 16), f32::INFINITY);
        m.limit(-10., 40.);
        assert_eq!(m.carry, 0.);
        let steps = steps(&mut m, t0, 16, f32::INFINITY);
        assert!(steps.len() > 3, "{steps:?}");
        assert!(steps.windows(2).all(|w| w[0] >= w[1]), "{steps:?}");
        assert!(
            (steps.iter().sum::<f32>() - 40.).abs() < 1e-3,
            "{a} {steps:?}"
        );
    }

    #[test]
    fn a_limited_glide_and_carry_keep_their_direction() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.wheel(2000., t0);
        m.tick(ms(t0, 50), 100.);
        m.limit(-5., 5.);
        // What's left goes down, whatever the glide started with.
        let (left, carry) = match m.anim {
            Some(Anim::Glide { distance, done, .. }) => (distance - done, m.carry),
            a => panic!("{a:?}"),
        };
        assert!(left >= 0. && carry >= 0., "{left} {carry}");
        assert!((left + carry - 5.).abs() < 1e-3);
    }

    #[test]
    fn direction_is_that_of_the_rest() {
        let t0 = Instant::now();
        // A glide that started down but has 30 px left to go up.
        let m = Motion {
            anim: Some(Anim::Glide {
                distance: 100.,
                start: t0,
                done: 30.,
            }),
            carry: -100.,
            ..Default::default()
        };
        assert_eq!(m.direction(), -1.);
        // Nothing left of it at all.
        let m = Motion {
            anim: Some(Anim::Glide {
                distance: 100.,
                start: t0,
                done: 100.,
            }),
            carry: 0.,
            ..Default::default()
        };
        assert_eq!(m.direction(), 0.);
        // Only a carry.
        let m = Motion {
            carry: 12.,
            ..Default::default()
        };
        assert_eq!(m.direction(), 1.);
    }

    #[test]
    fn limit_with_no_motion_does_nothing() {
        let mut m = Motion::default();
        m.limit(-10., 10.);
        assert!(!m.is_active());
        m.limit(5., 10.);
        assert!(!m.is_active(), "no motion to stretch");
        // A room that leaves out where it is now just stops the motion,
        // never turns it around.
        let t0 = Instant::now();
        m.wheel(-100., t0);
        m.limit(5., 10.);
        assert!(!m.is_active());
        m.wheel(100., t0);
        m.limit(-10., -5.);
        assert!(!m.is_active());
    }

    #[test]
    fn stop_ends_everything() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.wheel(-2000., t0);
        m.tick(ms(t0, 150), 760.);
        m.stop();
        assert!(!m.is_active());
        assert_eq!(m.direction(), 0.);
        assert_eq!(m.tick(ms(t0, 166), 760.), 0.);
    }

    #[test]
    fn estimates_unmeasured_rows_with_the_average() {
        let m = Measured {
            count: 10,
            measured: 4,
            height: 200.,
        };
        assert_eq!(m.average(), 50.);
        assert_eq!(m.estimate(30.), 380.);
        assert_eq!(Measured::default().average(), DEFAULT_ROW_HEIGHT);
        assert_eq!(Measured::default().estimate(30.), 0.);
    }

    #[test]
    fn thumb_geometry() {
        assert_eq!(thumb(0., 500., 500., 500.), None);
        assert_eq!(thumb(0., 1000., 500., 0.), None);
        assert_eq!(
            thumb(0., 1000., 500., 500.),
            Some(Thumb { top: 0., len: 250. })
        );
        assert_eq!(
            thumb(500., 1000., 500., 500.),
            Some(Thumb {
                top: 250.,
                len: 250.
            })
        );
        assert_eq!(
            thumb(250., 1000., 500., 500.),
            Some(Thumb {
                top: 125.,
                len: 250.
            })
        );
        // Past the end clamps; a huge document keeps a grabbable thumb.
        assert_eq!(thumb(900., 1000., 500., 500.).unwrap().top, 250.);
        assert_eq!(
            thumb(0., 1_000_000., 500., 500.),
            Some(Thumb {
                top: 0.,
                len: MIN_THUMB
            })
        );
        assert_eq!(thumb(0., 1_000_000., 500., 10.).unwrap().len, 10.);
    }

    #[test]
    fn position_for_thumb_inverts_thumb() {
        for top in [0., 100., 250., 499.] {
            let t = thumb(top, 2000., 500., 400.).unwrap();
            let pos = position_for_thumb(t.top, t.len, 2000., 500., 400.);
            assert!((pos - top).abs() < 1e-3, "{top} → {pos}");
        }
        assert_eq!(position_for_thumb(-5., 100., 2000., 500., 400.), 0.);
        assert_eq!(position_for_thumb(900., 100., 2000., 500., 400.), 1500.);
        assert_eq!(position_for_thumb(10., 400., 2000., 500., 400.), 0.);
    }

    #[test]
    fn room_splits_a_scroll_at_unmeasured_rows() {
        let room = Room {
            up: 300.,
            down: 500.,
            top: false,
            bottom: false,
        };
        assert_eq!(room.split(200.), (200., 0.));
        assert_eq!(room.split(-200.), (-200., 0.));
        assert_eq!(room.split(500.), (500., 0.));
        assert_eq!(room.split(-300.), (-300., 0.));
        // The rest waits for the rows past the measured ones.
        assert_eq!(room.split(1400.), (500., 900.));
        assert_eq!(room.split(-1400.), (-300., -1100.));
        assert_eq!(room.split(0.), (0., 0.));
        // Nothing measured at all yet: it all waits.
        assert_eq!(Room::default().split(-60.), (0., -60.));
        assert_eq!(Room::default().split(60.), (0., 60.));
    }

    #[test]
    fn room_drops_what_goes_past_an_edge() {
        let top = Room {
            up: 300.,
            down: 500.,
            top: true,
            bottom: false,
        };
        assert_eq!(top.split(-1400.), (-300., 0.));
        assert_eq!(top.split(1400.), (500., 900.));
        let bottom = Room {
            top: false,
            bottom: true,
            ..top
        };
        assert_eq!(bottom.split(1400.), (500., 0.));
        assert_eq!(bottom.split(-1400.), (-300., -1100.));
    }

    #[test]
    fn room_knows_when_it_is_at_an_edge() {
        let mut room = Room {
            up: 0.,
            down: 0.,
            top: true,
            bottom: true,
        };
        assert!(room.at_edge(-1.));
        assert!(room.at_edge(1.));
        assert!(!room.at_edge(0.));
        // Room left, or unmeasured rows that the next frame measures.
        room.up = 0.25;
        room.down = 0.25;
        assert!(!room.at_edge(-1.));
        assert!(!room.at_edge(1.));
        let gap = Room::default();
        assert!(!gap.at_edge(-1.));
        assert!(!gap.at_edge(1.));
    }

    /// `measured_run` over rows measured (`true`) or not.
    fn run_of(rows: &[bool], ix: usize) -> Range<usize> {
        measured_run(ix, rows.len(), |i| rows[..i].iter().filter(|&&m| m).count())
    }

    #[test]
    fn measured_run_finds_the_measured_rows_around_a_row() {
        let rows = [true, true, false, true, true, true, false, false, true];
        assert_eq!(run_of(&rows, 0), 0..2);
        assert_eq!(run_of(&rows, 1), 0..2);
        assert_eq!(run_of(&rows, 3), 3..6);
        assert_eq!(run_of(&rows, 4), 3..6);
        assert_eq!(run_of(&rows, 5), 3..6);
        assert_eq!(run_of(&rows, 8), 8..9);
        // An unmeasured row: only the measured ones right above it.
        assert_eq!(run_of(&rows, 2), 0..2);
        assert_eq!(run_of(&rows, 6), 3..6);
        assert_eq!(run_of(&rows, 7), 7..7);
        // Past the end: the measured rows right above the end.
        assert_eq!(run_of(&rows, 9), 8..9);
        assert_eq!(run_of(&rows, 50), 8..9);
        assert_eq!(run_of(&[false, false], 2), 2..2);
        // All or nothing.
        assert_eq!(run_of(&[true; 5], 2), 0..5);
        assert_eq!(run_of(&[false; 5], 2), 2..2);
        assert_eq!(run_of(&[], 0), 0..0);
    }

    #[test]
    fn measured_run_matches_a_linear_scan() {
        // Every pattern of 7 rows, from every row.
        for bits in 0u32..128 {
            let rows: Vec<bool> = (0..7).map(|i| bits & (1 << i) != 0).collect();
            for ix in 0..=7 {
                let mut start = ix;
                while start > 0 && rows[start - 1] {
                    start -= 1;
                }
                let mut end = ix;
                while end < 7 && rows[end] {
                    end += 1;
                }
                assert_eq!(run_of(&rows, ix), start..end, "{rows:?} from {ix}");
            }
        }
    }

    #[test]
    fn locate_finds_the_row_and_offset() {
        // Rows of 10, 0, 30, 20 px.
        let tops = [0., 10., 10., 40., 60.];
        let before = |ix: usize| tops[ix];
        assert_eq!(locate(0., 4, before), (0, 0.));
        assert_eq!(locate(5., 4, before), (0, 5.));
        // The 0-height row 1 is skipped.
        assert_eq!(locate(10., 4, before), (2, 0.));
        assert_eq!(locate(39., 4, before), (2, 29.));
        assert_eq!(locate(45., 4, before), (3, 5.));
        // Past the end stays in the last row.
        assert_eq!(locate(100., 4, before), (3, 60.));
        assert_eq!(locate(-3., 4, before), (0, 0.));
        assert_eq!(locate(10., 0, |_| 0.), (0, 10.));
    }

    #[test]
    fn wheel_keeps_a_running_glide_and_its_carry() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.wheel(-2000., t0);
        // Mid-glide and capped: both a glide rest and a carry are pending.
        let a = m.tick(ms(t0, 50), 100.);
        assert_eq!(a, -100.);
        m.wheel(-10., ms(t0, 60));
        assert!((a + run(&mut m, t0, 60) + 2010.).abs() < 1e-3);
    }

    #[test]
    fn wheel_against_a_long_glide_shortens_it() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.wheel(200., t0);
        let a = m.tick(ms(t0, 16), f32::INFINITY);
        m.wheel(-10., ms(t0, 20));
        assert_eq!(m.direction(), 1.);
        assert!((a + run(&mut m, t0, 20) - 190.).abs() < 1e-3);
    }

    #[test]
    fn samples_exactly_at_the_window_edge_count() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.touch(20., t0);
        m.touch(20., t0 + VELOCITY_WINDOW);
        assert_eq!(m.samples.len(), 2);
    }

    #[test]
    fn samples_need_a_millisecond_between_them() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        m.touch(50., t0);
        m.touch(50., t0);
        m.touch_end(t0);
        assert!(!m.is_active(), "no speed from samples at one instant");
        let mut m = Motion::default();
        m.touch(1., t0);
        m.touch(1., ms(t0, 1)); // 1000 px/s
        m.touch_end(ms(t0, 1));
        assert!(m.is_active());
    }

    #[test]
    fn a_fling_starts_at_the_release_speed() {
        let t0 = Instant::now();
        let mut m = Motion::default();
        swipe(&mut m, t0, 20.);
        m.touch_end(ms(t0, 92));
        // 16 ms at just under 2000 px/s.
        let first = m.tick(ms(t0, 108), f32::INFINITY);
        assert!(first > 28. && first < 32., "{first}");
    }
}
