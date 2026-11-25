//! A filter that takes the lows (a rumble, a breath on the microphone) or the highs out of a sound.

use std::f32::consts::{PI, SQRT_2};

/// What the filter takes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The sounds below the frequency are taken out.
    HighPass,
    /// The sounds above the frequency are taken out.
    LowPass,
}

/// The frequency is kept between these (a filter set out of the range of what is heard would be a block of nothing).
pub const HZ: (f32, f32) = (20.0, 20_000.0);

/// Two poles and two zeros (a "biquad", the usual filter of a mixing desk), the same for each channel of a frame.
#[derive(Clone, Debug)]
pub struct Filter {
    b: [f32; 3],
    a: [f32; 2],
    /// What each channel remembers of the last two samples.
    memory: Vec<[f32; 2]>,
}

impl Filter {
    pub fn new(shape: Shape, hz: f32, sample_rate: u32) -> Filter {
        // never above half the sample rate: the filter would be unstable there
        let top = (sample_rate as f32 * 0.45).min(HZ.1);
        let hz = if hz.is_nan() { HZ.0 } else { hz }.clamp(HZ.0, top);
        let w = 2.0 * PI * hz / sample_rate as f32;
        let (sin, cos) = w.sin_cos();
        // Butterworth: the flattest filter, which does not ring (the quality factor is 1 / sqrt 2)
        let alpha = sin / (2.0 * (1.0 / SQRT_2));
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = match shape {
            Shape::HighPass => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0),
            Shape::LowPass => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0),
        };
        Filter {
            b: [b0 / a0, b1 / a0, b2 / a0],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            memory: Vec::new(),
        }
    }

    /// Goes on from what `before` remembers (the graph was edited: the sound goes on without a click).
    pub fn continue_from(&mut self, before: &Filter) {
        self.memory = before.memory.clone();
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        if self.memory.len() != channels {
            self.memory = vec![[0.0; 2]; channels];
        }
        for (frame_in, frame_out) in input
            .chunks_exact(channels)
            .zip(output.chunks_exact_mut(channels))
        {
            for ((x, y), z) in frame_in.iter().zip(frame_out).zip(&mut self.memory) {
                // transposed direct form II: two numbers of memory per channel
                let out = self.b[0] * x + z[0];
                z[0] = self.b[1] * x - self.a[0] * out + z[1];
                z[1] = self.b[2] * x - self.a[1] * out;
                *y = out;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// How much of a sine of `hz` comes out of the filter, in decibels, once it has settled.
    fn response(shape: Shape, cutoff: f32, hz: f32) -> f32 {
        let mut f = Filter::new(shape, cutoff, RATE);
        let frames = RATE as usize;
        let input: Vec<f32> = (0..frames)
            .flat_map(|n| {
                let s = (2.0 * PI * hz * n as f32 / RATE as f32).sin() * 0.5;
                [s, s]
            })
            .collect();
        let mut out = vec![0.0; frames * 2];
        f.process(&input, &mut out, 2);
        let peak = out[out.len() / 2..]
            .iter()
            .fold(0.0f32, |m, s| m.max(s.abs()));
        20.0 * (peak / 0.5).log10()
    }

    #[test]
    fn a_high_pass_takes_out_the_lows_and_keeps_the_highs() {
        assert!(response(Shape::HighPass, 200.0, 50.0) < -20.0);
        assert!(response(Shape::HighPass, 200.0, 2000.0).abs() < 0.5);
        // at the cut-off frequency it is down by 3 dB
        assert!((response(Shape::HighPass, 200.0, 200.0) + 3.0).abs() < 0.7);
    }

    #[test]
    fn a_low_pass_takes_out_the_highs_and_keeps_the_lows() {
        assert!(response(Shape::LowPass, 2000.0, 12_000.0) < -30.0);
        assert!(response(Shape::LowPass, 2000.0, 200.0).abs() < 0.5);
    }

    #[test]
    fn a_frequency_out_of_range_is_kept_in_it_and_the_filter_stays_stable() {
        for hz in [f32::NAN, -5.0, 0.0, 1.0e9] {
            let mut f = Filter::new(Shape::LowPass, hz, RATE);
            let input = vec![0.5; 9600];
            let mut out = vec![0.0; 9600];
            f.process(&input, &mut out, 2);
            assert!(out.iter().all(|s| s.is_finite() && s.abs() < 2.0), "{hz}");
        }
    }

    #[test]
    fn a_new_filter_goes_on_from_the_memory_of_the_old_one() {
        let mut a = Filter::new(Shape::HighPass, 100.0, RATE);
        let input = vec![0.5; 2000];
        let mut out = vec![0.0; 2000];
        a.process(&input, &mut out, 2);
        let mut b = Filter::new(Shape::HighPass, 100.0, RATE);
        b.continue_from(&a);
        let (mut next_a, mut next_b) = (vec![0.0; 2000], vec![0.0; 2000]);
        a.process(&input, &mut next_a, 2);
        b.process(&input, &mut next_b, 2);
        assert_eq!(next_a, next_b);
    }
}
