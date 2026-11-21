//! DeepFilterNet noise reduction (the model comes from the DeepFilterNet module).
//!
//! The model costs a few milliseconds of processor time every 10 ms of sound, and the graph thread has no time to
//! lose, so it runs on a thread of its own: the graph thread hands each cycle's sound to a ring, takes back what the
//! model finished a moment ago, and never waits. That costs one hop (10 ms) of delay on top of the model's own. If the
//! model ever falls behind, the cycle that finds nothing ready is silent (counted) rather than late, and sound that
//! cannot be handed over is dropped (counted): the rest of the graph is never held up.

use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Frames the model works on at a time (DeepFilterNet 3 at 48 kHz: 10 ms).
pub const HOP: usize = 480;
const CHANNELS: usize = 2;
/// Seconds of sound the rings hold.
const RING_SECONDS: f32 = 0.25;

/// Something that cleans one hop of stereo sound. It is built on the thread that runs it and never leaves it (the
/// model keeps state that cannot move between threads).
pub trait HopProcessor {
    /// `input` and `output` are `HOP` frames of interleaved stereo.
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), String>;
}

/// What the graph thread uses.
pub struct Endpoints {
    to_worker: Producer<f32>,
    from_worker: Consumer<f32>,
    stats: Arc<Stats>,
}

/// How the model is doing (read for the block's status).
#[derive(Debug, Default)]
pub struct Stats {
    /// Cycles that found nothing ready and were silent.
    pub missed: AtomicU64,
    /// Cycles whose sound could not be handed over (the model was behind).
    pub dropped: AtomicU64,
    /// The worst time the model took for one hop, in microseconds.
    pub worst_us: AtomicU64,
}

impl Endpoints {
    /// Hands over this cycle's sound and fills `out` with what the model finished. Never waits, never allocates.
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) {
        match self.to_worker.write_chunk(input.len()) {
            Ok(mut chunk) => {
                let (a, b) = chunk.as_mut_slices();
                a.copy_from_slice(&input[..a.len()]);
                b.copy_from_slice(&input[a.len()..]);
                chunk.commit_all();
            }
            Err(_) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        match self.from_worker.read_chunk(out.len()) {
            Ok(chunk) => {
                let (a, b) = chunk.as_slices();
                out[..a.len()].copy_from_slice(a);
                out[a.len()..].copy_from_slice(b);
                chunk.commit_all();
            }
            Err(_) => {
                out.fill(0.0);
                self.stats.missed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn stats(&self) -> &Arc<Stats> {
        &self.stats
    }
}

impl omu_hal::Running for Worker {}

/// The thread that runs the model. Dropping it stops the thread.
pub struct Worker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Starts a worker around the processor `make` builds (on the worker's own thread); returns the graph thread's end and the
/// worker's guard once the processor is ready, or why it could not be made. A panic of the processor (the model is third-party code
/// that reads what the network sent) ends the worker and fails `health`: the engine then opens the block again, instead of the whole
/// program ending.
pub fn start_with<P, F>(
    make: F,
    health: Arc<omu_hal::StreamHealth>,
) -> Result<(Arc<Mutex<Endpoints>>, Worker), String>
where
    P: HopProcessor + 'static,
    F: FnOnce() -> Result<P, String> + Send + 'static,
{
    let ring = (RING_SECONDS * 48_000.0) as usize * CHANNELS;
    let (to_worker, mut from_graph) = RingBuffer::<f32>::new(ring);
    let (mut to_graph, from_worker) = RingBuffer::<f32>::new(ring);
    // the hop of silence that sets the delay: the graph always has a finished hop to take
    {
        let chunk = to_graph
            .write_chunk(HOP * CHANNELS)
            .map_err(|_| "no room for the first hop".to_string())?;
        // (the chunk is zeroed memory: commit it as it is)
        let mut chunk = chunk;
        let (a, b) = chunk.as_mut_slices();
        a.fill(0.0);
        b.fill(0.0);
        chunk.commit_all();
    }
    let stats = Arc::new(Stats::default());
    let stop = Arc::new(AtomicBool::new(false));
    let (flag, s2) = (stop.clone(), stats.clone());
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let thread = std::thread::Builder::new()
        .name("deepfilter".into())
        .spawn(move || {
            let on_panic = health;
            let outcome = omu_diagnostics::crash::contained(move || {
                let mut processor = match make() {
                    Ok(p) => {
                        let _ = ready_tx.send(Ok(()));
                        p
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let mut input = vec![0.0f32; HOP * CHANNELS];
                let mut output = vec![0.0f32; HOP * CHANNELS];
                while !flag.load(Ordering::Relaxed) {
                    let Ok(chunk) = from_graph.read_chunk(HOP * CHANNELS) else {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    };
                    let (a, b) = chunk.as_slices();
                    input[..a.len()].copy_from_slice(a);
                    input[a.len()..].copy_from_slice(b);
                    chunk.commit_all();
                    let started = std::time::Instant::now();
                    if let Err(e) = processor.process(&input, &mut output) {
                        // a model that fails lets the sound through as it is rather than going silent
                        tracing::warn!("DeepFilterNet: {e}");
                        output.copy_from_slice(&input);
                    }
                    s2.worst_us
                        .fetch_max(started.elapsed().as_micros() as u64, Ordering::Relaxed);
                    // wait for room rather than lose a finished hop: the graph thread takes one every cycle
                    while !flag.load(Ordering::Relaxed) {
                        match to_graph.write_chunk(HOP * CHANNELS) {
                            Ok(mut out) => {
                                let (a, b) = out.as_mut_slices();
                                a.copy_from_slice(&output[..a.len()]);
                                b.copy_from_slice(&output[a.len()..]);
                                out.commit_all();
                                break;
                            }
                            Err(_) => std::thread::sleep(Duration::from_millis(1)),
                        }
                    }
                }
            });
            if outcome.is_err() {
                on_panic.fail("the noise reduction stopped unexpectedly (see the logs)");
            }
        })
        .map_err(|e| format!("cannot start the noise reduction thread: {e}"))?;
    // loading the model takes a moment (a fraction of a second; much longer in a debug build)
    match ready_rx.recv_timeout(Duration::from_secs(120)) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = thread.join();
            return Err(e);
        }
        Err(_) => {
            return Err("the noise reduction model did not start".into());
        }
    }
    Ok((
        Arc::new(Mutex::new(Endpoints {
            to_worker,
            from_worker,
            stats,
        })),
        Worker {
            stop,
            thread: Some(thread),
        },
    ))
}

#[cfg(feature = "model")]
mod model {
    use super::{HopProcessor, CHANNELS, HOP};
    use df::tract::{DfParams, DfTract, RuntimeParams};
    use ndarray::Array2;
    use std::path::Path;

    /// The DeepFilterNet model, running.
    pub struct Model {
        df: DfTract,
        noisy: Array2<f32>,
        clean: Array2<f32>,
    }

    impl Model {
        /// Loads the model from its archive. `limit_db` is the most the noise is reduced by (100 or more: no limit).
        pub fn load(archive: &Path, limit_db: f32) -> Result<Model, String> {
            let params = DfParams::new(archive.to_owned())
                .map_err(|e| format!("cannot read the model: {e}"))?;
            let mut runtime = RuntimeParams::default_with_ch(CHANNELS);
            if limit_db < 100.0 {
                runtime = runtime.with_atten_lim(limit_db.max(0.0));
            }
            let df = DfTract::new(params, &runtime)
                .map_err(|e| format!("cannot start the model: {e}"))?;
            if df.hop_size != HOP || df.sr != 48_000 || df.ch != CHANNELS {
                return Err(format!(
                    "this model works on {} frames at {} Hz in {} channels, not 480 at 48000 in 2",
                    df.hop_size, df.sr, df.ch
                ));
            }
            Ok(Model {
                df,
                noisy: Array2::zeros((CHANNELS, HOP)),
                clean: Array2::zeros((CHANNELS, HOP)),
            })
        }
    }

    impl HopProcessor for Model {
        fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), String> {
            for (i, frame) in input.as_chunks::<2>().0.iter().enumerate() {
                self.noisy[[0, i]] = frame[0];
                self.noisy[[1, i]] = frame[1];
            }
            self.df
                .process(self.noisy.view(), self.clean.view_mut())
                .map_err(|e| e.to_string())?;
            for (i, frame) in output.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                frame[0] = self.clean[[0, i]];
                frame[1] = self.clean[[1, i]];
            }
            Ok(())
        }
    }
}

#[cfg(feature = "model")]
pub use model::Model;

/// Loads the model at `archive` and starts it on its own thread.
#[cfg(feature = "model")]
pub fn start(
    archive: &std::path::Path,
    limit_db: f32,
    health: Arc<omu_hal::StreamHealth>,
) -> Result<(Arc<Mutex<Endpoints>>, Worker), String> {
    let archive = archive.to_owned();
    start_with(move || Model::load(&archive, limit_db), health)
}

/// Without the model's code in the program there is nothing to start.
#[cfg(not(feature = "model"))]
pub fn start(
    _archive: &std::path::Path,
    _limit_db: f32,
    _health: Arc<omu_hal::StreamHealth>,
) -> Result<(Arc<Mutex<Endpoints>>, Worker), String> {
    Err("this build of the program does not include DeepFilterNet".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Halves the sound, like a model that took half of it away.
    struct Half;
    impl HopProcessor for Half {
        fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), String> {
            for (o, i) in output.iter_mut().zip(input) {
                *o = *i * 0.5;
            }
            Ok(())
        }
    }

    struct Broken;
    impl HopProcessor for Broken {
        fn process(&mut self, _: &[f32], _: &mut [f32]) -> Result<(), String> {
            Err("the model failed".into())
        }
    }

    /// Plays `cycles` cycles of a counting signal (left 0.001*n, right the negative) through the endpoints, pacing like
    /// the graph thread: one cycle of `frames` frames every `pace`.
    fn run(
        endpoints: &Arc<Mutex<Endpoints>>,
        frames: usize,
        cycles: usize,
        pace: Duration,
    ) -> Vec<f32> {
        let mut heard = Vec::new();
        let mut n = 0u32;
        for _ in 0..cycles {
            let mut input = vec![0.0f32; frames * 2];
            for f in input.as_chunks_mut::<2>().0.iter_mut() {
                f[0] = n as f32 * 1e-5;
                f[1] = -(n as f32) * 1e-5;
                n += 1;
            }
            let mut out = vec![0.0f32; frames * 2];
            endpoints.lock().unwrap().process(&input, &mut out);
            heard.extend_from_slice(&out);
            std::thread::sleep(pace);
        }
        heard
    }

    #[test]
    fn sound_comes_back_processed_in_order_after_one_hop_of_delay_whatever_the_cycle_length() {
        for frames in [480usize, 240, 960] {
            let (endpoints, worker) =
                start_with(|| Ok(Half), omu_hal::StreamHealth::new()).unwrap();
            let heard = run(
                &endpoints,
                frames,
                4000 / frames,
                Duration::from_millis(frames as u64 / 48 + 1),
            );
            // the first hop is the silence that sets the delay; after it, the signal halved, in order, without gaps
            let left: Vec<f32> = heard.iter().step_by(2).copied().collect();
            let first = left
                .iter()
                .position(|s| *s != 0.0)
                .expect("sound came back");
            assert!(
                first >= HOP.saturating_sub(1) && first <= HOP + 3 * frames,
                "{frames}: starts at {first}"
            );
            // consecutive frames of the output are consecutive frames of the input (halved): a step of 0.5e-5
            let body = &left[first + 1..];
            let mut broken = 0;
            let mut count = 0;
            for w in body.windows(2) {
                if w[0] == 0.0 || w[1] == 0.0 {
                    continue; // a cycle that found nothing ready
                }
                count += 1;
                if ((w[1] - w[0]) - 0.5e-5).abs() > 1e-7 {
                    broken += 1;
                }
            }
            assert!(count > 1000, "{frames}: {count} frames heard");
            assert!(broken <= 2, "{frames}: {broken} breaks in the order");
            // the right channel is the left one, negated: the channels stay together
            let right: Vec<f32> = heard.iter().skip(1).step_by(2).copied().collect();
            assert!(left.iter().zip(&right).all(|(l, r)| (l + r).abs() < 1e-7));
            drop(worker);
        }
    }

    /// A model that panics (third-party code, fed by whatever came in).
    struct Panics;
    impl HopProcessor for Panics {
        fn process(&mut self, _: &[f32], _: &mut [f32]) -> Result<(), String> {
            panic!("a bug in the model");
        }
    }

    #[test]
    fn a_model_that_panics_fails_its_block_and_does_not_end_the_program() {
        let health = omu_hal::StreamHealth::new();
        let (endpoints, worker) = start_with(|| Ok(Panics), health.clone()).unwrap();
        let _ = run(&endpoints, 480, 20, Duration::from_millis(11));
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while !health.has_failed() && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            health.has_failed(),
            "the block is reported as failed, so that the engine opens it again"
        );
        drop(worker);
    }

    #[test]
    fn a_model_that_fails_lets_the_sound_through_and_a_model_that_is_too_slow_costs_silence_not_time(
    ) {
        let (endpoints, worker) = start_with(|| Ok(Broken), omu_hal::StreamHealth::new()).unwrap();
        let heard = run(&endpoints, 480, 20, Duration::from_millis(11));
        let left: Vec<f32> = heard.iter().step_by(2).copied().collect();
        // unprocessed, but there: the sound at its original level
        let tail = &left[left.len() - 480..];
        assert!(
            tail.iter().any(|s| *s > 0.0)
                && tail.windows(2).all(|w| (w[1] - w[0] - 1e-5).abs() < 1e-7),
            "{:?}",
            &tail[..4]
        );
        drop(worker);

        // nobody takes the sound: the graph side must never block, whatever happens
        struct Stuck;
        impl HopProcessor for Stuck {
            fn process(&mut self, _: &[f32], _: &mut [f32]) -> Result<(), String> {
                std::thread::sleep(Duration::from_millis(40));
                Ok(())
            }
        }
        let (endpoints, worker) = start_with(|| Ok(Stuck), omu_hal::StreamHealth::new()).unwrap();
        let started = std::time::Instant::now();
        let mut out = vec![0.0f32; 960];
        for _ in 0..300 {
            endpoints
                .lock()
                .unwrap()
                .process(&vec![0.1f32; 960], &mut out);
        }
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "the graph thread waited: {:?}",
            started.elapsed()
        );
        let stats = endpoints.lock().unwrap().stats().clone();
        assert!(
            stats.dropped.load(Ordering::Relaxed) > 0,
            "the overflow is counted"
        );
        assert!(
            stats.missed.load(Ordering::Relaxed) > 0,
            "the silent cycles are counted"
        );
        drop(worker);
    }
}
