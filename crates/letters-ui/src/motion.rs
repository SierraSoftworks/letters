//! Motion: how far along a change looks, for how far along in time it is.

use crate::tokens;

/// The longest wait between two frames of something that moves, in milliseconds.
pub(crate) const FRAME_MS: u64 = 8;

/// yewi's easing curve: `progress` of the time gone, from 0 to 1, gives how much of the change shows.
pub fn ease(progress: f32) -> f32 {
    let [x1, y1, x2, y2] = tokens::EASE;
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
}
