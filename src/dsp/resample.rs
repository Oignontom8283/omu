//! Streaming resampling with a ratio that can change at every call, and the controller that keeps a ring buffer
//! half full by nudging that ratio (how two devices with slightly different clocks are bridged).
//!
//! The interpolator is a 4-point cubic (Catmull-Rom): cheap, allocation-free, transparent for the tiny ratio
//! corrections of clock drift, and good enough for the common 44.1/48 kHz conversions of voice. A longer sinc
//! filter (for music-grade conversion between very different rates) can replace it behind the same interface.

/// Resamples interleaved audio. Call [`StreamResampler::process`] repeatedly with the frames available; it
/// reports how many input frames it consumed (the rest must be presented again next time).
#[derive(Clone, Debug)]
pub struct StreamResampler {
    channels: usize,
    /// The input frame just before the current position (interpolation history).
    history: Vec<f32>,
    /// Fractional read position, in input frames, relative to the first not-yet-consumed input frame.
    pos: f64,
}

impl StreamResampler {
    pub fn new(channels: usize) -> Self {
        assert!(channels > 0);
        StreamResampler {
            channels,
            history: vec![0.0; channels],
            pos: 0.0,
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Forgets the history (use after a discontinuity).
    pub fn reset(&mut self) {
        self.history.fill(0.0);
        self.pos = 0.0;
    }

    /// `step` is the number of input frames per output frame (`in_rate / out_rate`, 1.0 = copy).
    /// Returns `(input frames consumed, output frames produced)`. Needs two frames of look-ahead, so it
    /// consumes at most `input_frames - 2`.
    pub fn process(&mut self, input: &[f32], output: &mut [f32], step: f64) -> (usize, usize) {
        let ch = self.channels;
        let in_frames = input.len() / ch;
        let out_capacity = output.len() / ch;
        let mut produced = 0;
        while produced < out_capacity {
            let i = self.pos.floor() as usize;
            if i + 2 >= in_frames {
                break;
            }
            let t = (self.pos - i as f64) as f32;
            for c in 0..ch {
                let p0 = if i == 0 {
                    self.history[c]
                } else {
                    input[(i - 1) * ch + c]
                };
                let p1 = input[i * ch + c];
                let p2 = input[(i + 1) * ch + c];
                let p3 = input[(i + 2) * ch + c];
                output[produced * ch + c] = p1
                    + 0.5
                        * t
                        * (p2 - p0
                            + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3
                                + t * (3.0 * (p1 - p2) + p3 - p0)));
            }
            produced += 1;
            self.pos += step;
        }
        let consumed = (self.pos.floor() as usize).min(in_frames.saturating_sub(2));
        if consumed > 0 {
            self.history
                .copy_from_slice(&input[(consumed - 1) * ch..consumed * ch]);
            self.pos -= consumed as f64;
        }
        (consumed, produced)
    }
}

/// Turns the fill level of a ring buffer into a resampling ratio that keeps it near `target` frames.
///
/// Positive error (too full) -> consume faster (ratio above nominal); too empty -> slower. The level is smoothed
/// first so that scheduling jitter of the producer does not modulate the ratio, and the correction is clamped
/// (a real clock drift is at most a few hundred ppm; the clamp is 0.5 %).
#[derive(Clone, Debug)]
pub struct RateController {
    target: f64,
    smoothed: f64,
    /// Smoothing factor of the fill level per update (0..1, small = slow).
    alpha: f64,
    /// Relative ratio change per unit of relative fill error.
    gain: f64,
    max_correction: f64,
}

impl RateController {
    pub fn new(target_frames: usize) -> Self {
        let target = target_frames.max(1) as f64;
        RateController {
            target,
            smoothed: target,
            alpha: 0.01,
            gain: 0.02,
            max_correction: 0.005,
        }
    }

    /// Feeds the current fill level and returns the multiplier to apply to the nominal ratio.
    pub fn update(&mut self, fill_frames: usize) -> f64 {
        self.smoothed += self.alpha * (fill_frames as f64 - self.smoothed);
        let error = (self.smoothed - self.target) / self.target;
        1.0 + (self.gain * error).clamp(-self.max_correction, self.max_correction)
    }

    pub fn smoothed_fill(&self) -> f64 {
        self.smoothed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, rate: f64, frames: usize, ch: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|n| {
                let v = (std::f64::consts::TAU * freq * n as f64 / rate).sin() as f32 * 0.5;
                std::iter::repeat_n(v, ch)
            })
            .collect()
    }

    fn run_all(r: &mut StreamResampler, input: &[f32], step: f64, chunk: usize) -> Vec<f32> {
        let ch = r.channels();
        let mut out = vec![];
        let mut pending: Vec<f32> = vec![];
        let mut fed = 0;
        while fed < input.len() {
            let take = (chunk * ch).min(input.len() - fed);
            pending.extend_from_slice(&input[fed..fed + take]);
            fed += take;
            let mut buf = vec![0.0; (pending.len() / ch + 8) * ch * 2];
            let (consumed, produced) = r.process(&pending, &mut buf, step);
            out.extend_from_slice(&buf[..produced * ch]);
            pending.drain(..consumed * ch);
        }
        out
    }

    #[test]
    fn unit_step_is_a_copy_delayed_by_the_lookahead() {
        let input: Vec<f32> = (0..100).map(|n| n as f32).collect();
        let mut r = StreamResampler::new(1);
        let out = run_all(&mut r, &input, 1.0, 100);
        assert_eq!(out.len(), 98);
        for (i, v) in out.iter().enumerate() {
            assert!((v - i as f32).abs() < 1e-4, "frame {i}: {v}");
        }
    }

    #[test]
    fn chunk_size_does_not_change_the_result() {
        let input = sine(1000.0, 44_100.0, 4410, 2);
        let step = 44_100.0 / 48_000.0;
        let a = run_all(&mut StreamResampler::new(2), &input, step, 4410);
        let b = run_all(&mut StreamResampler::new(2), &input, step, 97);
        let n = a.len().min(b.len());
        assert!(n > 4000);
        for i in 0..n {
            assert!((a[i] - b[i]).abs() < 1e-5, "sample {i}");
        }
    }

    #[test]
    fn resampling_44100_to_48000_keeps_the_tone() {
        let input = sine(1000.0, 44_100.0, 44_100, 1);
        let out = run_all(
            &mut StreamResampler::new(1),
            &input,
            44_100.0 / 48_000.0,
            441,
        );
        // compare with the ideal 48 kHz sine, after the start-up transient
        let mut err = 0.0f64;
        let mut sig = 0.0f64;
        for (n, v) in out.iter().enumerate().skip(100).take(40_000) {
            let ideal = (std::f64::consts::TAU * 1000.0 * n as f64 / 48_000.0).sin() * 0.5;
            err += (*v as f64 - ideal).powi(2);
            sig += ideal.powi(2);
        }
        let snr_db = 10.0 * (sig / err).log10();
        assert!(snr_db > 45.0, "SNR {snr_db:.1} dB");
    }

    #[test]
    fn stereo_channels_stay_independent() {
        let mut input = vec![];
        for n in 0..200 {
            input.push(n as f32);
            input.push(-(n as f32));
        }
        let out = run_all(&mut StreamResampler::new(2), &input, 1.0, 50);
        for f in out.chunks(2) {
            assert!((f[0] + f[1]).abs() < 1e-4);
        }
    }

    /// A producer 100 ppm fast and a consumer pulling through the controller: the fill level must stay bounded.
    #[test]
    fn controller_absorbs_clock_drift_without_underrun_or_overrun() {
        for ppm in [-300.0f64, -100.0, 0.0, 100.0, 300.0] {
            let consumer_rate = 48_000.0;
            let producer_rate = consumer_rate * (1.0 + ppm * 1e-6);
            let chunk_out = 480usize; // consumer pulls 10 ms
            let target = 960usize; // 20 ms
            let mut resampler = StreamResampler::new(1);
            let mut controller = RateController::new(target);
            let mut ring: std::collections::VecDeque<f32> = std::collections::VecDeque::new();
            let (mut min_fill, mut max_fill) = (usize::MAX, 0usize);
            let mut produced_total = 0f64;
            // prefill to the target
            ring.extend(std::iter::repeat_n(0.0, target));
            let mut out = vec![0.0f32; chunk_out];
            let cycles = 6000; // 60 s
            let mut underruns = 0;
            for cycle in 0..cycles {
                // the producer delivers what elapsed time says
                let should = producer_rate * (cycle + 1) as f64 * 0.01;
                let add = (should - produced_total).floor() as usize;
                produced_total += add as f64;
                ring.extend(std::iter::repeat_n(0.25, add));
                // the consumer pulls
                let ratio = controller.update(ring.len());
                let contiguous: Vec<f32> = ring.iter().copied().collect();
                let (consumed, produced) = resampler.process(&contiguous, &mut out, ratio);
                if produced < chunk_out {
                    underruns += 1;
                }
                ring.drain(..consumed);
                if cycle > 500 {
                    min_fill = min_fill.min(ring.len());
                    max_fill = max_fill.max(ring.len());
                }
            }
            assert_eq!(underruns, 0, "{ppm} ppm: underruns");
            assert!(
                min_fill > 100 && max_fill < 3 * target,
                "{ppm} ppm: fill {min_fill}..{max_fill}"
            );
        }
    }

    #[test]
    fn controller_nudges_the_right_way_and_is_clamped() {
        let mut c = RateController::new(1000);
        for _ in 0..2000 {
            c.update(5000);
        }
        let fast = c.update(5000);
        assert!(fast > 1.0 && (1.0..=1.0 + 0.005 + 1e-12).contains(&fast));
        let mut c = RateController::new(1000);
        for _ in 0..2000 {
            c.update(0);
        }
        let slow = c.update(0);
        assert!(slow < 1.0 && (1.0 - 0.005 - 1e-12..=1.0).contains(&slow));
    }
}
