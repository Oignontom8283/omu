//! Gain in decibels with click-free changes.

/// Silence floor: gains at or below this many decibels are treated as zero.
pub const MIN_DB: f32 = -120.0;

pub fn db_to_linear(db: f32) -> f32 {
    if db <= MIN_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        MIN_DB
    } else {
        (20.0 * linear.log10()).max(MIN_DB)
    }
}

/// A gain that glides to a new target over a few milliseconds instead of jumping (no clicks).
#[derive(Clone, Debug)]
pub struct SmoothedGain {
    current: f32,
    target: f32,
    /// Linear change per frame while moving.
    step: f32,
    /// Frames a full 0 -> 1 move takes.
    ramp_frames: f32,
}

impl SmoothedGain {
    /// `ramp_ms` is the time a change of 1.0 (linear) takes at `sample_rate`.
    pub fn new(db: f32, sample_rate: u32, ramp_ms: f32) -> Self {
        let g = db_to_linear(db);
        SmoothedGain {
            current: g,
            target: g,
            step: 0.0,
            ramp_frames: (ramp_ms / 1000.0 * sample_rate as f32).max(1.0),
        }
    }

    pub fn set_db(&mut self, db: f32) {
        self.target = db_to_linear(db);
        // Constant speed: a full 0 -> 1 move takes `ramp_frames`, a shorter move proportionally less.
        self.step = if self.target > self.current {
            1.0 / self.ramp_frames
        } else if self.target < self.current {
            -1.0 / self.ramp_frames
        } else {
            0.0
        };
    }

    /// Continues from another gain's current value (used when a graph is replaced: no jump at the swap).
    pub fn start_from(&mut self, current_linear: f32) {
        self.current = current_linear;
        let target = self.target;
        self.step = if target > current_linear {
            1.0 / self.ramp_frames
        } else if target < current_linear {
            -1.0 / self.ramp_frames
        } else {
            0.0
        };
    }

    pub fn current(&self) -> f32 {
        self.current
    }

    /// Applies the gain in place to `frames` interleaved frames of `channels` channels.
    pub fn apply(&mut self, samples: &mut [f32], channels: usize) {
        if channels == 0 {
            return;
        }
        for frame in samples.chunks_mut(channels) {
            if self.current != self.target {
                self.current += self.step;
                if (self.step > 0.0 && self.current >= self.target)
                    || (self.step < 0.0 && self.current <= self.target)
                {
                    self.current = self.target;
                }
            }
            for s in frame {
                *s *= self.current;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_conversions() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_linear(-6.0206) - 0.5).abs() < 1e-3);
        assert_eq!(db_to_linear(-200.0), 0.0);
        assert!((linear_to_db(db_to_linear(-12.0)) + 12.0).abs() < 1e-3);
        assert_eq!(linear_to_db(0.0), MIN_DB);
    }

    #[test]
    fn steady_gain_scales_every_channel() {
        let mut g = SmoothedGain::new(-6.0206, 48_000, 5.0);
        let mut buf = vec![1.0f32; 8];
        g.apply(&mut buf, 2);
        assert!(buf.iter().all(|s| (s - 0.5).abs() < 1e-3));
    }

    #[test]
    fn a_change_glides_and_ends_exactly_on_target() {
        let mut g = SmoothedGain::new(0.0, 48_000, 5.0);
        g.set_db(-120.0); // to silence
        let mut buf = vec![1.0f32; 2 * 480];
        g.apply(&mut buf, 2);
        // monotonic decrease, no jump larger than one step, ends at 0
        let frames: Vec<f32> = buf.chunks(2).map(|f| f[0]).collect();
        assert!(frames.windows(2).all(|w| w[1] <= w[0] + 1e-6));
        assert!(frames.windows(2).all(|w| (w[0] - w[1]).abs() < 0.01));
        assert_eq!(*frames.last().unwrap(), 0.0);
        assert_eq!(g.current(), 0.0);
    }

    #[test]
    fn setting_the_same_gain_changes_nothing() {
        let mut g = SmoothedGain::new(-3.0, 48_000, 5.0);
        let before = g.current();
        g.set_db(-3.0);
        let mut buf = vec![1.0f32; 16];
        g.apply(&mut buf, 2);
        assert_eq!(g.current(), before);
    }
}
