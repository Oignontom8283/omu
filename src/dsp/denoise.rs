//! Noise reduction: RNNoise (Xiph.Org's recurrent neural network that tells voice from noise), through `nnnoiseless`, its
//! pure Rust rewrite with the trained model built in (BSD-3-Clause, see `THIRD-PARTY-NOTICES.md`).
//!
//! RNNoise works on mono frames of 10 ms at 48 kHz, which is exactly one cycle of the engine, so each channel has its
//! own state and a cycle is one frame. For another cycle length the frames are gathered and handed back through small
//! queues. It adds 10 ms of delay (RNNoise looks at the frame it overlaps), and its first frame is thrown away because
//! it holds the fade-in of the filter. Nothing is allocated once built.

use nnnoiseless::DenoiseState;

/// Samples of one RNNoise frame.
pub const FRAME: usize = DenoiseState::FRAME_SIZE;

/// The model wants 16-bit sized values: audio here is in -1..1.
const SCALE: f32 = 32768.0;

struct Channel {
    state: Box<DenoiseState<'static>>,
    /// Input not yet processed (less than a frame after each cycle).
    pending: Vec<f32>,
    /// Output not yet handed back.
    ready: Vec<f32>,
    /// Whether the frame holding the fade-in was thrown away already.
    primed: bool,
    input: [f32; FRAME],
    output: [f32; FRAME],
}

/// The noise reduction of one stream of `channels` interleaved channels.
pub struct Denoiser {
    channels: Vec<Channel>,
    /// Frames of one engine cycle.
    cycle: usize,
}

impl Denoiser {
    /// A denoiser for cycles of `cycle_frames` frames of `channels` channels. This allocates (and warms up the
    /// transform plans the model shares), so it is made when the plan is built, never in the real-time thread.
    pub fn new(channels: usize, cycle_frames: usize) -> Denoiser {
        // the first frame of a state builds the transform plans: do it here, on a state that is thrown away
        let mut warm = DenoiseState::new();
        let (silence, mut out) = ([0.0f32; FRAME], [0.0f32; FRAME]);
        warm.process_frame(&mut out, &silence);

        let channels = (0..channels.max(1))
            .map(|_| Channel {
                state: DenoiseState::new(),
                pending: Vec::with_capacity(FRAME + cycle_frames),
                ready: Vec::with_capacity(cycle_frames + 2 * FRAME),
                primed: false,
                input: [0.0; FRAME],
                output: [0.0; FRAME],
            })
            .collect();
        Denoiser {
            channels,
            cycle: cycle_frames.max(1),
        }
    }

    /// Denoises one cycle: `input` and `output` are interleaved, `cycle` frames long.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let n = self.channels.len();
        debug_assert_eq!(input.len(), self.cycle * n);
        debug_assert_eq!(output.len(), self.cycle * n);
        for (c, ch) in self.channels.iter_mut().enumerate() {
            ch.pending
                .extend(input.iter().skip(c).step_by(n).map(|x| x * SCALE));
            while ch.pending.len() >= FRAME {
                ch.input.copy_from_slice(&ch.pending[..FRAME]);
                ch.pending.drain(..FRAME);
                ch.state.process_frame(&mut ch.output, &ch.input);
                if ch.primed {
                    ch.ready.extend(ch.output.iter().map(|x| x / SCALE));
                } else {
                    // the very first frame holds the fade-in of the filter: silence instead
                    ch.primed = true;
                    ch.ready.extend(std::iter::repeat_n(0.0, FRAME));
                }
            }
            if ch.ready.len() >= self.cycle {
                for (frame, x) in ch.ready.drain(..self.cycle).enumerate() {
                    output[frame * n + c] = x.clamp(-1.0, 1.0);
                }
            } else {
                // not enough yet (start-up, or a cycle shorter than a frame): silence, keep what is ready
                for frame in 0..self.cycle {
                    output[frame * n + c] = 0.0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic noise (xorshift), `amplitude` at most.
    fn noise(len: usize, amplitude: f32, seed: u32) -> Vec<f32> {
        let mut x = seed.max(1);
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32 * 2.0 - 1.0) * amplitude
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// Runs `input` (interleaved) through a denoiser in cycles of `cycle` frames.
    fn run(channels: usize, cycle: usize, input: &[f32]) -> Vec<f32> {
        let mut d = Denoiser::new(channels, cycle);
        let mut out = vec![0.0; input.len()];
        for (i, o) in input
            .chunks_exact(cycle * channels)
            .zip(out.chunks_exact_mut(cycle * channels))
        {
            d.process(i, o);
        }
        out
    }

    /// A low rumble like a fan or an air conditioner: noise that is low-passed.
    fn rumble(len: usize, level: f32, seed: u32) -> Vec<f32> {
        let mut low = 0.0;
        noise(len, 1.0, seed)
            .into_iter()
            .map(|n| {
                low = 0.98 * low + 0.02 * n;
                low * level
            })
            .collect()
    }

    /// A voiced sound: harmonics of `f0` with two formant humps and a syllable-like envelope, 3 per second.
    fn voice(len: usize, f0: f32, level: f32) -> Vec<f32> {
        let tau = 2.0 * std::f32::consts::PI;
        (0..len)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                let envelope = 0.5 + 0.5 * (tau * 3.0 * t).sin().max(0.0);
                let sum: f32 = (1..=30)
                    .map(|h| {
                        let f = f0 * h as f32;
                        let hump = (-((f - 700.0) / 500.0).powi(2)).exp()
                            + 0.6 * (-((f - 1800.0) / 700.0).powi(2)).exp();
                        hump * (tau * f * t).sin()
                    })
                    .sum();
                sum * envelope * level / 4.0
            })
            .collect()
    }

    fn mono_to_stereo(m: &[f32]) -> Vec<f32> {
        m.iter().flat_map(|x| [*x, *x]).collect()
    }

    fn left(out: &[f32]) -> Vec<f32> {
        out.iter().step_by(2).copied().collect()
    }

    fn db(ratio: f32) -> f32 {
        20.0 * ratio.log10()
    }

    #[test]
    fn a_steady_rumble_is_taken_out() {
        let n = 48_000 * 4;
        let input = rumble(n, 3.0, 7);
        let out = left(&run(2, FRAME, &mono_to_stereo(&input)));
        let change = db(rms(&out[48_000..]) / rms(&input[48_000..]));
        assert!(
            change < -20.0,
            "the rumble only went down by {change:.1} dB"
        );
    }

    #[test]
    fn a_voice_goes_through_as_it_is_and_the_noise_around_it_goes_down() {
        let n = 48_000 * 4;
        let clean = voice(n, 120.0, 0.3);
        // the voice alone is left alone (same level, within a dB)
        let alone = left(&run(2, FRAME, &mono_to_stereo(&clean)));
        let kept = db(rms(&alone[48_000..]) / rms(&clean[48_000..]));
        assert!(kept.abs() < 1.0, "a voice alone changed by {kept:.1} dB");
        // with white noise on top: the noise is lower than before and the voice is still there
        let hiss = noise(n, 0.05, 21);
        let mixed: Vec<f32> = clean.iter().zip(&hiss).map(|(a, b)| a + b).collect();
        let out = left(&run(2, FRAME, &mono_to_stereo(&mixed)));
        // the block delays the sound by one frame: compare with the clean voice a frame earlier
        let error: Vec<f32> = (48_000..n - FRAME)
            .map(|i| out[i + FRAME] - clean[i])
            .collect();
        let before: Vec<f32> = (48_000..n).map(|i| mixed[i] - clean[i]).collect();
        let gain = db(rms(&error) / rms(&before));
        assert!(gain < -3.0, "the noise only went down by {gain:.1} dB");
        let level = db(rms(&out[48_000..]) / rms(&clean[48_000..]));
        assert!(level.abs() < 1.5, "the voice changed by {level:.1} dB");
    }

    #[test]
    fn silence_stays_silent_and_nothing_is_ever_out_of_range() {
        let out = run(2, FRAME, &vec![0.0; FRAME * 2 * 50]);
        assert!(out.iter().all(|s| *s == 0.0));
        // full-scale square waves and noise: finite, and inside the range the engine expects
        let loud: Vec<f32> = (0..FRAME * 2 * 100)
            .map(|i| if (i / 40) % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        for input in [loud, noise(FRAME * 2 * 100, 1.0, 3)] {
            let out = run(2, FRAME, &input);
            assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        }
    }

    #[test]
    fn the_first_cycle_is_silent_not_the_fade_in_of_the_filter() {
        let out = run(2, FRAME, &noise(FRAME * 2 * 3, 0.5, 11));
        assert!(out[..FRAME * 2].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn channels_do_not_leak_into_each_other() {
        // noise on the left, silence on the right
        let n = noise(48_000 * 2, 0.3, 5);
        let stereo: Vec<f32> = n.iter().flat_map(|l| [*l, 0.0]).collect();
        let out = run(2, FRAME, &stereo);
        let right: Vec<f32> = out.iter().skip(1).step_by(2).copied().collect();
        assert!(
            right.iter().all(|s| *s == 0.0),
            "the silent channel stayed silent"
        );
    }

    #[test]
    fn another_cycle_length_gives_the_same_sound_a_little_later() {
        // the frames are the same, only the queues differ: half a frame of cycles delays the sound by one cycle
        let input = noise(FRAME * 2 * 20, 0.2, 9);
        let whole = run(2, FRAME, &input);
        let half = run(2, FRAME / 2, &input);
        // 240-frame cycles hand back the same samples, 240 frames later (480 interleaved samples... one cycle)
        let shift = FRAME / 2 * 2;
        assert_eq!(&half[shift..], &whole[..whole.len() - shift]);
        // a cycle that is not a multiple of a frame works too
        let odd = run(2, 1000, &noise(1000 * 2 * 30, 0.2, 13));
        assert!(odd.iter().all(|s| s.is_finite()));
        assert!(rms(&odd[1000 * 2 * 5..]) < 0.2);
    }

    #[test]
    fn a_cycle_costs_far_less_than_the_cycle_lasts() {
        let input = noise(FRAME * 2, 0.2, 1);
        let mut out = vec![0.0; FRAME * 2];
        let mut d = Denoiser::new(2, FRAME);
        for _ in 0..50 {
            d.process(&input, &mut out);
        }
        let t = std::time::Instant::now();
        for _ in 0..400 {
            d.process(&input, &mut out);
        }
        let per_cycle = t.elapsed().as_secs_f64() / 400.0;
        println!(
            "one stereo cycle of noise reduction: {:.0} us",
            per_cycle * 1e6
        );
        // 10 ms per cycle: even an unoptimised test build must stay well below it
        assert!(per_cycle < 0.008, "{per_cycle} s per cycle");
    }
}
