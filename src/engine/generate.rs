//! Sounds the graph makes by itself: a steady tone and noise. They run on the graph thread, one cycle at a time, and never allocate.

use omu_dsp::db_to_linear;
use omu_types::{
    NoiseColor, Wave, ENGINE_CHANNELS, ENGINE_SAMPLE_RATE, GENERATED_MAX_DB, TONE_MAX_HZ,
    TONE_MIN_HZ,
};

const CH: usize = ENGINE_CHANNELS as usize;

/// A level in decibels brought into what a generated sound may be (never louder than `GENERATED_MAX_DB`), as a gain.
fn level(db: f32) -> f32 {
    let db = if db.is_finite() { db } else { -60.0 };
    db_to_linear(db.min(GENERATED_MAX_DB))
}

/// A tone: the same value on every channel.
#[derive(Debug, Clone)]
pub struct Oscillator {
    wave: Wave,
    /// Where it is in its period, from 0 to 1.
    phase: f32,
    /// How far the phase moves for each frame.
    step: f32,
    gain: f32,
}

/// The correction that takes the aliasing out of the jump of a square or saw wave (PolyBLEP): `t` is the phase, `dt` the step.
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

impl Oscillator {
    pub fn new(wave: Wave, hz: f32, db: f32) -> Oscillator {
        let hz = if hz.is_finite() {
            hz.clamp(TONE_MIN_HZ, TONE_MAX_HZ)
        } else {
            440.0
        };
        Oscillator {
            wave,
            phase: 0.0,
            step: hz / ENGINE_SAMPLE_RATE as f32,
            gain: level(db),
        }
    }

    /// Goes on from where the tone it replaces was, so that editing the graph does not click.
    pub fn continue_from(&mut self, before: &Oscillator) {
        self.phase = before.phase;
    }

    fn sample(&self) -> f32 {
        let (t, dt) = (self.phase, self.step);
        match self.wave {
            Wave::Sine => (t * std::f32::consts::TAU).sin(),
            Wave::Square => {
                let naive = if t < 0.5 { 1.0 } else { -1.0 };
                naive + poly_blep(t, dt) - poly_blep((t + 0.5).fract(), dt)
            }
            Wave::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
            Wave::Saw => 2.0 * t - 1.0 - poly_blep(t, dt),
        }
    }

    /// Writes one cycle of interleaved frames.
    pub fn fill(&mut self, out: &mut [f32]) {
        for frame in out.as_chunks_mut::<CH>().0 {
            frame.fill(self.sample() * self.gain);
            self.phase += self.step;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

/// Noise: the same on every channel.
#[derive(Debug, Clone)]
pub struct NoiseGenerator {
    color: NoiseColor,
    gain: f32,
    seed: u32,
    /// The filters that tilt white noise into pink, and the running sum of brown.
    pink: [f32; 3],
    brown: f32,
}

impl NoiseGenerator {
    pub fn new(color: NoiseColor, db: f32) -> NoiseGenerator {
        NoiseGenerator {
            color,
            gain: level(db),
            seed: 0x9E37_79B9,
            pink: [0.0; 3],
            brown: 0.0,
        }
    }

    /// Goes on from where the noise it replaces was.
    pub fn continue_from(&mut self, before: &NoiseGenerator) {
        self.seed = before.seed;
        self.pink = before.pink;
        self.brown = before.brown;
    }

    /// White noise between -1 and 1 (xorshift: fast, and good enough for a hiss).
    fn white(&mut self) -> f32 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        x as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    fn sample(&mut self) -> f32 {
        let white = self.white();
        match self.color {
            NoiseColor::White => white,
            NoiseColor::Pink => {
                // three filters of different speeds added (Paul Kellet's economical pink filter)
                self.pink[0] = 0.997_65 * self.pink[0] + white * 0.099_046;
                self.pink[1] = 0.963 * self.pink[1] + white * 0.296_516_4;
                self.pink[2] = 0.57 * self.pink[2] + white * 1.052_691_3;
                (self.pink[0] + self.pink[1] + self.pink[2] + white * 0.1848) * 0.11
            }
            NoiseColor::Brown => {
                self.brown = (self.brown + 0.02 * white) / 1.02;
                (self.brown * 3.5).clamp(-1.0, 1.0)
            }
        }
    }

    /// Writes one cycle of interleaved frames.
    pub fn fill(&mut self, out: &mut [f32]) {
        for frame in out.as_chunks_mut::<CH>().0 {
            let v = self.sample() * self.gain;
            frame.fill(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cycle(f: impl FnOnce(&mut [f32]), frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * CH];
        f(&mut out);
        out
    }

    #[test]
    fn a_sine_has_the_pitch_and_the_level_that_were_asked_for() {
        // 480 Hz at 48 kHz: 100 frames to a period; -6 dB is about half
        let mut osc = Oscillator::new(Wave::Sine, 480.0, -6.0);
        let out = cycle(|o| osc.fill(o), 1000);
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.501).abs() < 0.01, "peak {peak}");
        let crossings = out
            .as_chunks::<CH>()
            .0
            .iter()
            .map(|f| f[0])
            .collect::<Vec<_>>()
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        assert!(
            (9..=11).contains(&crossings),
            "ten periods in 1000 frames, got {crossings}"
        );
        assert!(
            out.as_chunks::<CH>().0.iter().all(|f| f[0] == f[1]),
            "every channel carries the same sound"
        );
    }

    #[test]
    fn a_tone_goes_on_across_cycles_and_across_a_replacement() {
        let mut a = Oscillator::new(Wave::Sine, 1000.0, 0.0);
        let first = cycle(|o| a.fill(o), 64);
        let mut b = Oscillator::new(Wave::Sine, 1000.0, 0.0);
        b.continue_from(&a);
        let second = cycle(|o| b.fill(o), 64);
        let mut whole = Oscillator::new(Wave::Sine, 1000.0, 0.0);
        let both = cycle(|o| whole.fill(o), 128);
        assert_eq!(
            [first, second].concat(),
            both,
            "no break where the block was replaced"
        );
    }

    #[test]
    fn no_wave_goes_past_the_level_and_a_loud_request_is_held_back() {
        for wave in [Wave::Sine, Wave::Square, Wave::Triangle, Wave::Saw] {
            let mut osc = Oscillator::new(wave, 1234.0, 40.0);
            let out = cycle(|o| osc.fill(o), 4800);
            let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(
                peak <= db_to_linear(GENERATED_MAX_DB) * 1.1,
                "{wave:?} peaks at {peak}"
            );
            assert!(peak > 0.5, "{wave:?} makes sound");
        }
    }

    #[test]
    fn a_pitch_out_of_range_or_not_a_number_is_brought_back() {
        let mut out_of_range = Oscillator::new(Wave::Sine, 1e9, 0.0);
        let mut nan = Oscillator::new(Wave::Sine, f32::NAN, f32::NAN);
        let a = cycle(|o| out_of_range.fill(o), 480);
        let b = cycle(|o| nan.fill(o), 480);
        assert!(a.iter().chain(&b).all(|v| v.is_finite()));
    }

    #[test]
    fn each_colour_of_noise_makes_sound_that_stays_in_range_and_the_darker_ones_are_slower() {
        let mut energy = vec![];
        for color in [NoiseColor::White, NoiseColor::Pink, NoiseColor::Brown] {
            let mut noise = NoiseGenerator::new(color, 0.0);
            let out = cycle(|o| noise.fill(o), 48_000);
            let mono: Vec<f32> = out.as_chunks::<CH>().0.iter().map(|f| f[0]).collect();
            assert!(
                mono.iter().all(|v| v.abs() <= 1.0 + 1e-6),
                "{color:?} stays within range"
            );
            let rms = (mono.iter().map(|v| v * v).sum::<f32>() / mono.len() as f32).sqrt();
            assert!(rms > 0.02, "{color:?} is not silent ({rms})");
            // how much it changes from one frame to the next: a rumble changes slowly, a hiss quickly
            let jitter =
                mono.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / mono.len() as f32;
            energy.push(jitter / rms);
        }
        assert!(
            energy[0] > energy[1] && energy[1] > energy[2],
            "white, then pink, then brown: slower and slower ({energy:?})"
        );
    }
}
