//! A compressor: it brings the loud parts of a voice down so that the voice stays at about the same strength, and a limiter when its ratio is high.

use crate::gain::db_to_linear;

/// The sound it works on is interleaved: a frame is `channels` samples, and both channels get the same gain (the picture of the stereo stays).
#[derive(Clone, Debug)]
pub struct Compressor {
    threshold_db: f32,
    /// For each decibel the sound goes over the threshold, it comes out `1 / ratio` decibel over.
    ratio: f32,
    makeup: f32,
    attack: f32,
    release: f32,
    /// How strong the sound is, followed with the attack and the release (linear).
    level: f32,
}

/// The settings are kept in these limits whatever a graph says: a compressor never receives a ratio below 1 (it would make loud sounds louder).
pub const THRESHOLD_DB: (f32, f32) = (-80.0, 0.0);
pub const RATIO: (f32, f32) = (1.0, 20.0);
pub const ATTACK_MS: (f32, f32) = (0.1, 500.0);
pub const RELEASE_MS: (f32, f32) = (5.0, 5000.0);
pub const MAKEUP_DB: (f32, f32) = (0.0, 24.0);

fn crop(v: f32, (min, max): (f32, f32)) -> f32 {
    if v.is_nan() {
        min
    } else {
        v.clamp(min, max)
    }
}

/// The factor by which a level moves toward the sound in `ms` milliseconds (the smaller, the faster).
fn coefficient(ms: f32, sample_rate: u32) -> f32 {
    (-1.0 / (ms * 0.001 * sample_rate as f32)).exp()
}

impl Compressor {
    pub fn new(
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_db: f32,
        sample_rate: u32,
    ) -> Compressor {
        Compressor {
            threshold_db: crop(threshold_db, THRESHOLD_DB),
            ratio: crop(ratio, RATIO),
            makeup: db_to_linear(crop(makeup_db, MAKEUP_DB)),
            attack: coefficient(crop(attack_ms, ATTACK_MS), sample_rate),
            release: coefficient(crop(release_ms, RELEASE_MS), sample_rate),
            level: 0.0,
        }
    }

    /// Goes on from where `before` was (the graph was edited: the level it had reached is not lost, so nothing jumps).
    pub fn continue_from(&mut self, before: &Compressor) {
        self.level = before.level;
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let slope = 1.0 - 1.0 / self.ratio;
        for (frame_in, frame_out) in input
            .chunks_exact(channels)
            .zip(output.chunks_exact_mut(channels))
        {
            let peak = frame_in.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            let coefficient = if peak > self.level {
                self.attack
            } else {
                self.release
            };
            self.level = peak + coefficient * (self.level - peak);
            let gain = if self.level > 1e-6 {
                let over = 20.0 * self.level.log10() - self.threshold_db;
                if over > 0.0 {
                    db_to_linear(-over * slope) * self.makeup
                } else {
                    self.makeup
                }
            } else {
                self.makeup
            };
            for (o, s) in frame_out.iter_mut().zip(frame_in) {
                *o = s * gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn run(c: &mut Compressor, level: f32, frames: usize) -> f32 {
        let input = vec![level; frames * 2];
        let mut out = vec![0.0; frames * 2];
        c.process(&input, &mut out, 2);
        *out.last().unwrap()
    }

    #[test]
    fn a_sound_over_the_threshold_comes_out_less_over_by_the_ratio() {
        // 0.5 is -6.02 dB: 13.98 dB over a -20 dB threshold, so 3.5 dB over with a ratio of 4: about -16.5 dB
        let mut c = Compressor::new(-20.0, 4.0, 1.0, 100.0, 0.0, RATE);
        let out = run(&mut c, 0.5, RATE as usize / 2);
        let db = 20.0 * out.log10();
        assert!((db + 16.5).abs() < 0.3, "{db} dB");
    }

    #[test]
    fn a_sound_under_the_threshold_is_not_touched_and_the_makeup_raises_it() {
        let mut c = Compressor::new(-20.0, 4.0, 1.0, 100.0, 0.0, RATE);
        assert!((run(&mut c, 0.05, 4800) - 0.05).abs() < 1e-6);
        let mut c = Compressor::new(-20.0, 4.0, 1.0, 100.0, 6.0, RATE);
        assert!((run(&mut c, 0.05, 4800) - 0.05 * db_to_linear(6.0)).abs() < 1e-4);
    }

    #[test]
    fn a_ratio_of_one_changes_nothing_and_the_settings_are_kept_in_their_limits() {
        let mut c = Compressor::new(-40.0, 1.0, 1.0, 100.0, 0.0, RATE);
        assert!((run(&mut c, 0.8, 4800) - 0.8).abs() < 1e-5);
        // a ratio below 1 would make loud sounds louder: it is brought back to 1
        let mut c = Compressor::new(-40.0, 0.2, f32::NAN, -5.0, 100.0, RATE);
        assert!(run(&mut c, 0.8, 4800) <= 0.8 * db_to_linear(24.0) + 1e-3);
        assert_eq!(c.ratio, 1.0);
    }

    #[test]
    fn it_lets_go_slowly_after_a_loud_part_and_both_channels_get_the_same_gain() {
        let mut c = Compressor::new(-20.0, 8.0, 0.5, 400.0, 0.0, RATE);
        run(&mut c, 0.9, 4800);
        // 20 ms after the loud part ended, the gain has not come back yet
        let after = run(&mut c, 0.1, 960);
        assert!(after < 0.1 * 0.9, "{after}");
        // the two channels of a frame are scaled by the same factor
        let mut c = Compressor::new(-30.0, 4.0, 1.0, 100.0, 0.0, RATE);
        let input = [0.8, 0.2, 0.8, 0.2];
        let mut out = [0.0; 4];
        c.process(&input, &mut out, 2);
        assert!((out[0] / input[0] - out[1] / input[1]).abs() < 1e-5);
    }
}
