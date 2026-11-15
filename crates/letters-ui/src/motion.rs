//! Motion: how far along a change looks, for how far along in time it is.

use crate::tokens;

/// The longest wait between two frames of something that moves, in milliseconds.
pub(crate) const FRAME_MS: u64 = 8;

/// The curve CSS calls `ease-in-out`, which yewi gives what moves and comes back, as its skeleton's light.
pub(crate) const EASE_IN_OUT: [f32; 4] = [0.42, 0.0, 0.58, 1.0];

/// yewi's easing curve: `progress` of the time gone, from 0 to 1, gives how much of the change shows.
pub fn ease(progress: f32) -> f32 {
    curve(tokens::EASE, progress)
}

/// How much of a change shows `progress` of the way through its time, along a cubic Bézier curve as CSS
/// writes one.
pub(crate) fn curve([x1, y1, x2, y2]: [f32; 4], progress: f32) -> f32 {
    let bezier = |a: f32, b: f32, t: f32| {
        let u = 1.0 - t;
        3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
    };
    // The curve gives time and change for a parameter, so the parameter for this time is searched for.
    let x = progress.clamp(0.0, 1.0);
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..16 {
        let middle = (low + high) / 2.0;
        if bezier(x1, x2, middle) < x { low = middle } else { high = middle }
    }
    bezier(y1, y2, (low + high) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_runs_from_nothing_to_everything_without_turning_back() {
        let steps: Vec<f32> = (0..=20).map(|step| ease(step as f32 / 20.0)).collect();
        assert!(steps[0].abs() < 0.01 && (steps[20] - 1.0).abs() < 0.01, "{steps:?}");
        assert!(steps.windows(2).all(|pair| pair[0] <= pair[1]), "{steps:?}");
        assert!(steps[10] > 0.5, "most of the change shows early: {steps:?}");
        assert_eq!((ease(-1.0), ease(2.0)), (ease(0.0), ease(1.0)), "time outside the change is held at its ends");
    }

    #[test]
    fn easing_in_and_out_is_slow_at_both_ends_and_even_about_the_middle() {
        let at = |progress| curve(EASE_IN_OUT, progress);
        assert!((at(0.5) - 0.5).abs() < 0.01, "{}", at(0.5));
        assert!(at(0.1) < 0.05 && at(0.9) > 0.95, "{} {}", at(0.1), at(0.9));
        assert!((at(0.25) + at(0.75) - 1.0).abs() < 0.01, "the way out mirrors the way in");
    }
}
