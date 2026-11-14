//! Clock-domain bridges between a device and the graph.
//!
//! Every device runs on its own clock and in its own format; the graph runs on the engine's timer in the engine
//! format (48 kHz, stereo, `f32`). A bridge is a lock-free ring buffer plus, on the consuming side, a channel
//! remix and a resampler whose ratio is nudged by the ring's fill level, so that two clocks that differ by a few
//! hundred ppm never end in an underrun or an overrun.
//!
//! * [`source_bridge`]: device callback (producer, device format) -> graph thread ([`SourceConsumer::pull`]).
//! * [`sink_bridge`]: graph thread ([`SinkProducer::push`]) -> device callback ([`SinkConsumer::pull`]).
//!
//! Nothing here allocates or blocks after construction. Underruns produce silence and are counted; the bridge
//! then refills to its target level before resuming, which gives a clean recovery instead of stuttering.

use crate::dsp::{remix, RateController, StreamResampler};
use crate::types::{AudioFormat, ENGINE_CHANNELS, ENGINE_SAMPLE_RATE};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

const ENGINE_CH: usize = ENGINE_CHANNELS as usize;
/// Largest number of frames a bridge handles in one call (bigger buffers are processed in pieces).
const MAX_FRAMES: usize = 8192;
/// Capacity of every ring, in milliseconds of audio.
const RING_MS: usize = 500;

/// Counters shared with the supervisor and the status API.
#[derive(Debug, Default)]
pub struct BridgeStats {
    /// Times the consumer found too little audio and played silence.
    pub underruns: AtomicU64,
    /// Times the producer found the ring full and dropped audio.
    pub overruns: AtomicU64,
    /// Latest smoothed fill level, in milliseconds (x10 to keep one decimal in an integer).
    fill_ms_x10: AtomicU32,
}

impl BridgeStats {
    pub fn fill_ms(&self) -> f32 {
        self.fill_ms_x10.load(Ordering::Relaxed) as f32 / 10.0
    }
    fn set_fill_ms(&self, ms: f64) {
        self.fill_ms_x10
            .store((ms * 10.0) as u32, Ordering::Relaxed);
    }
}

fn ring_for(format: AudioFormat) -> (Producer<f32>, Consumer<f32>) {
    RingBuffer::new(format.sample_rate as usize * RING_MS / 1000 * format.channels as usize)
}

fn frames_for_ms(ms: f32, rate: u32) -> usize {
    ((ms / 1000.0) * rate as f32).round().max(1.0) as usize
}

/// Copies up to `n` slots from the consumer's readable chunk into `dst` without consuming them.
fn peek(cons: &mut Consumer<f32>, n: usize, dst: &mut [f32]) -> usize {
    let n = n.min(cons.slots()).min(dst.len());
    if n == 0 {
        return 0;
    }
    match cons.read_chunk(n) {
        Ok(chunk) => {
            let (a, b) = chunk.as_slices();
            dst[..a.len()].copy_from_slice(a);
            dst[a.len()..a.len() + b.len()].copy_from_slice(b);
            n
        }
        Err(_) => 0,
    }
}

fn advance(cons: &mut Consumer<f32>, n: usize) {
    if n > 0 {
        if let Ok(chunk) = cons.read_chunk(n) {
            chunk.commit_all();
        }
    }
}

fn write(prod: &mut Producer<f32>, data: &[f32]) -> bool {
    match prod.write_chunk(data.len()) {
        Ok(mut chunk) => {
            let (a, b) = chunk.as_mut_slices();
            a.copy_from_slice(&data[..a.len()]);
            b.copy_from_slice(&data[a.len()..]);
            chunk.commit_all();
            true
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------------------------------------
// device -> graph

/// The capture side: what the HAL callback calls with each captured buffer.
pub struct SourceProducer {
    prod: Producer<f32>,
    stats: Arc<BridgeStats>,
}

impl SourceProducer {
    pub fn push(&mut self, data: &[f32]) {
        if !write(&mut self.prod, data) {
            self.stats.overruns.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The graph side of a source: yields engine-format frames.
pub struct SourceConsumer {
    cons: Consumer<f32>,
    dev_ch: usize,
    dev_rate: u32,
    resampler: StreamResampler,
    controller: RateController,
    target_frames: usize,
    primed: bool,
    raw: Vec<f32>,
    stereo: Vec<f32>,
    stats: Arc<BridgeStats>,
}

/// Builds the pair for a capture device of format `dev`; `target_ms` is the fill level kept in the ring.
pub fn source_bridge(
    dev: AudioFormat,
    target_ms: f32,
) -> (SourceProducer, SourceConsumer, Arc<BridgeStats>) {
    let (prod, cons) = ring_for(dev);
    let stats = Arc::new(BridgeStats::default());
    let target_frames = frames_for_ms(target_ms, dev.sample_rate);
    let ch = dev.channels as usize;
    (
        SourceProducer {
            prod,
            stats: stats.clone(),
        },
        SourceConsumer {
            cons,
            dev_ch: ch,
            dev_rate: dev.sample_rate,
            resampler: StreamResampler::new(ENGINE_CH),
            controller: RateController::new(target_frames),
            target_frames,
            primed: false,
            raw: vec![0.0; MAX_FRAMES * ch],
            stereo: vec![0.0; MAX_FRAMES * ENGINE_CH],
            stats: stats.clone(),
        },
        stats,
    )
}

impl SourceConsumer {
    /// Fills `out` (engine format) with the next frames; silence while priming or after an underrun.
    pub fn pull(&mut self, out: &mut [f32]) {
        let n_out = out.len() / ENGINE_CH;
        let avail = self.cons.slots() / self.dev_ch;
        if !self.primed {
            if avail < self.target_frames {
                out.fill(0.0);
                return;
            }
            self.primed = true;
        }
        let ratio = self.controller.update(avail);
        self.stats
            .set_fill_ms(self.controller.smoothed_fill() * 1000.0 / self.dev_rate as f64);
        let step = self.dev_rate as f64 / ENGINE_SAMPLE_RATE as f64 * ratio;
        let need = (n_out as f64 * step).ceil() as usize + 4;
        let take = need.min(avail).min(MAX_FRAMES);
        let got = peek(&mut self.cons, take * self.dev_ch, &mut self.raw) / self.dev_ch;
        remix(
            &self.raw[..got * self.dev_ch],
            self.dev_ch,
            &mut self.stereo[..got * ENGINE_CH],
            ENGINE_CH,
        );
        let (consumed, produced) =
            self.resampler
                .process(&self.stereo[..got * ENGINE_CH], out, step);
        advance(&mut self.cons, consumed * self.dev_ch);
        if produced < n_out {
            out[produced * ENGINE_CH..].fill(0.0);
            self.stats.underruns.fetch_add(1, Ordering::Relaxed);
            self.primed = false;
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// graph -> device

/// The graph side of a sink: where the graph thread puts engine-format frames.
pub struct SinkProducer {
    prod: Producer<f32>,
    stats: Arc<BridgeStats>,
}

impl SinkProducer {
    pub fn push(&mut self, data: &[f32]) {
        if !write(&mut self.prod, data) {
            self.stats.overruns.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The device side of a sink: what the HAL render callback calls to get the frames to play.
pub struct SinkConsumer {
    cons: Consumer<f32>,
    dev_ch: usize,
    dev_rate: u32,
    resampler: StreamResampler,
    controller: RateController,
    target_frames: usize,
    primed: bool,
    stereo_in: Vec<f32>,
    stereo_out: Vec<f32>,
    stats: Arc<BridgeStats>,
}

/// Builds the pair for a playback device of format `dev`; `target_ms` is the fill level kept in the ring.
pub fn sink_bridge(
    dev: AudioFormat,
    target_ms: f32,
) -> (SinkProducer, SinkConsumer, Arc<BridgeStats>) {
    let (prod, cons) = ring_for(AudioFormat::ENGINE);
    let stats = Arc::new(BridgeStats::default());
    let target_frames = frames_for_ms(target_ms, ENGINE_SAMPLE_RATE);
    (
        SinkProducer {
            prod,
            stats: stats.clone(),
        },
        SinkConsumer {
            cons,
            dev_ch: dev.channels as usize,
            dev_rate: dev.sample_rate,
            resampler: StreamResampler::new(ENGINE_CH),
            controller: RateController::new(target_frames),
            target_frames,
            primed: false,
            stereo_in: vec![0.0; (MAX_FRAMES * 2) * ENGINE_CH],
            stereo_out: vec![0.0; MAX_FRAMES * ENGINE_CH],
            stats: stats.clone(),
        },
        stats,
    )
}

impl SinkConsumer {
    /// Fills `out` (device format) with the next frames to play; silence while priming or after an underrun.
    pub fn pull(&mut self, out: &mut [f32]) {
        let total = out.len() / self.dev_ch;
        let mut done = 0;
        while done < total {
            let piece = (total - done).min(MAX_FRAMES);
            let dst = &mut out[done * self.dev_ch..(done + piece) * self.dev_ch];
            self.pull_piece(dst, piece);
            done += piece;
        }
    }

    fn pull_piece(&mut self, out: &mut [f32], n_dev: usize) {
        let avail = self.cons.slots() / ENGINE_CH;
        if !self.primed {
            if avail < self.target_frames {
                out.fill(0.0);
                return;
            }
            self.primed = true;
        }
        let ratio = self.controller.update(avail);
        self.stats
            .set_fill_ms(self.controller.smoothed_fill() * 1000.0 / ENGINE_SAMPLE_RATE as f64);
        let step = ENGINE_SAMPLE_RATE as f64 / self.dev_rate as f64 * ratio;
        let need = (n_dev as f64 * step).ceil() as usize + 4;
        let take = need.min(avail).min(self.stereo_in.len() / ENGINE_CH);
        let got = peek(&mut self.cons, take * ENGINE_CH, &mut self.stereo_in) / ENGINE_CH;
        let (consumed, produced) = self.resampler.process(
            &self.stereo_in[..got * ENGINE_CH],
            &mut self.stereo_out[..n_dev * ENGINE_CH],
            step,
        );
        advance(&mut self.cons, consumed * ENGINE_CH);
        out.fill(0.0);
        remix(
            &self.stereo_out[..produced * ENGINE_CH],
            ENGINE_CH,
            out,
            self.dev_ch,
        );
        if produced < n_dev {
            self.stats.underruns.fetch_add(1, Ordering::Relaxed);
            self.primed = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CYCLE: usize = 480;

    fn mono_dev(rate: u32) -> AudioFormat {
        AudioFormat {
            sample_rate: rate,
            channels: 1,
        }
    }

    #[test]
    fn source_primes_then_delivers_the_device_signal() {
        let (mut prod, mut cons, stats) = source_bridge(AudioFormat::ENGINE, 20.0);
        let mut out = vec![1.0f32; CYCLE * 2];
        cons.pull(&mut out);
        assert!(out.iter().all(|s| *s == 0.0), "silent while priming");
        for _ in 0..3 {
            prod.push(&vec![0.5f32; CYCLE * 2]);
        }
        cons.pull(&mut out);
        assert!(
            out.iter().all(|s| (*s - 0.5).abs() < 1e-4),
            "signal after priming"
        );
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn source_converts_mono_44100_to_engine_stereo() {
        let (mut prod, mut cons, _) = source_bridge(mono_dev(44_100), 20.0);
        // 100 ms of a constant, in 10 ms device chunks, consumed in 10 ms engine cycles
        let mut got = vec![];
        for _ in 0..30 {
            prod.push(&vec![0.25f32; 441]);
            let mut out = vec![0.0f32; CYCLE * 2];
            cons.pull(&mut out);
            got.extend_from_slice(&out);
        }
        let tail = &got[got.len() - CYCLE * 2..];
        assert!(
            tail.iter().all(|s| (*s - 0.25).abs() < 1e-3),
            "constant preserved on both channels"
        );
    }

    #[test]
    fn source_underrun_gives_silence_and_is_counted() {
        let (mut prod, mut cons, stats) = source_bridge(AudioFormat::ENGINE, 20.0);
        for _ in 0..3 {
            prod.push(&vec![0.5f32; CYCLE * 2]);
        }
        let mut out = vec![0.0f32; CYCLE * 2];
        for _ in 0..6 {
            cons.pull(&mut out); // drains faster than it is fed
        }
        assert!(stats.underruns.load(Ordering::Relaxed) >= 1);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn source_overrun_drops_audio_and_is_counted() {
        let (mut prod, _cons, stats) = source_bridge(AudioFormat::ENGINE, 20.0);
        for _ in 0..200 {
            prod.push(&vec![0.1f32; CYCLE * 2]); // 2 s into a 0.5 s ring
        }
        assert!(stats.overruns.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn sink_plays_the_graph_signal_in_device_format() {
        let (mut prod, mut cons, stats) = sink_bridge(mono_dev(48_000), 20.0);
        for _ in 0..3 {
            prod.push(&vec![0.5f32; CYCLE * 2]);
        }
        let mut out = vec![9.0f32; CYCLE];
        cons.pull(&mut out);
        assert!(out.iter().all(|s| (*s - 0.5).abs() < 1e-3));
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn sink_with_no_audio_plays_silence() {
        let (_prod, mut cons, _) = sink_bridge(AudioFormat::ENGINE, 20.0);
        let mut out = vec![1.0f32; CYCLE * 2];
        cons.pull(&mut out);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    /// Simulated clocks: graph at exactly 48 kHz against a device `ppm` off, for 60 s: no underrun, no overrun.
    #[test]
    fn sink_bridge_absorbs_device_clock_drift() {
        for ppm in [-300.0f64, -100.0, 100.0, 300.0] {
            let (mut prod, mut cons, stats) = sink_bridge(AudioFormat::ENGINE, 25.0);
            let dev_rate = 48_000.0 * (1.0 + ppm * 1e-6);
            let dev_chunk = 441usize; // the device pulls ~9.2 ms at a time
            let mut dev_time = 0.0f64; // seconds of device audio pulled
            let mut graph_time = 0.0f64;
            let mut out = vec![0.0f32; dev_chunk * 2];
            let mut elapsed = 0.0f64; // wall clock
            let step = 0.0005; // 0.5 ms simulation step
            while elapsed < 60.0 {
                elapsed += step;
                while graph_time + 0.01 <= elapsed {
                    prod.push(&vec![0.3f32; CYCLE * 2]);
                    graph_time += 0.01;
                }
                while dev_time + dev_chunk as f64 / dev_rate <= elapsed {
                    cons.pull(&mut out);
                    dev_time += dev_chunk as f64 / dev_rate;
                }
            }
            let under = stats.underruns.load(Ordering::Relaxed);
            let over = stats.overruns.load(Ordering::Relaxed);
            // the first priming may cost one underrun; sustained drift must not cause more
            assert!(
                under <= 1 && over == 0,
                "{ppm} ppm: {under} underruns, {over} overruns"
            );
        }
    }

    #[test]
    fn large_device_buffers_are_processed_in_pieces() {
        let (mut prod, mut cons, stats) = sink_bridge(AudioFormat::ENGINE, 20.0);
        for _ in 0..40 {
            prod.push(&vec![0.5f32; CYCLE * 2]);
        }
        let mut out = vec![0.0f32; 20_000 * 2]; // 20 000 frames > MAX_FRAMES
        cons.pull(&mut out);
        assert!(out[..1000].iter().all(|s| (*s - 0.5).abs() < 1e-3));
        let _ = stats;
    }
}
