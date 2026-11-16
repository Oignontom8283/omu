//! What plays: the sounds in memory, the voices that read them and the mixer the graph thread runs.
//!
//! This file is the real-time side. Nothing here allocates, waits on a lock (it only tries), reads the disk or frees
//! memory that a sound uses: the blocks of a sound are given back to the pool by the threads that are allowed to wait.
//!
//! A sound short enough to keep whole is in memory from its first sample to its last. A longer one keeps its first
//! segment in memory (so it starts at once) and the rest arrives block by block from a thread that reads the disk
//! ahead of the voice (see `stream` in `engine`).

use super::pool::BlockRef;
use super::{BLOCK_SAMPLES, MAX_VOICES};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A sound in memory: all of it, or its first segment when it is longer than it is worth keeping whole.
#[derive(Debug)]
pub struct LoadedSound {
    pub id: String,
    pub channels: usize,
    /// Frames of the whole sound.
    pub frames: u64,
    /// Frames held in `blocks`: all of them, or the first segment.
    pub loaded_frames: u64,
    pub blocks: Vec<BlockRef>,
    /// The stored file (the rest of a long sound is read from it).
    pub path: PathBuf,
}

impl LoadedSound {
    /// Whether most of the sound is on the disk.
    pub fn streamed(&self) -> bool {
        self.loaded_frames < self.frames
    }

    /// The (left, right) samples of frame `frame` of the part in memory.
    #[inline]
    fn frame(&self, frame: u64) -> (i16, i16) {
        let s = frame as usize * self.channels;
        let block = &self.blocks[s / BLOCK_SAMPLES].data;
        let l = block[s % BLOCK_SAMPLES];
        let r = if self.channels == 2 {
            block[(s + 1) % BLOCK_SAMPLES]
        } else {
            l
        };
        (l, r)
    }
}

/// What a long sound's reader and its voice share: blocks on their way to the voice, and blocks on their way back.
#[derive(Debug, Default)]
pub struct StreamState {
    /// Blocks read from the disk, in order, not yet played.
    pub queue: VecDeque<BlockRef>,
    /// Blocks the voice has finished with, for the reader to take back.
    pub done: Vec<BlockRef>,
    /// The reader has put the last block of the sound in the queue.
    pub eof: bool,
    /// The sound was stopped: the reader gives everything back and forgets it.
    pub stop: bool,
}

#[derive(Debug, Default)]
pub struct StreamShared {
    pub state: Mutex<StreamState>,
}

/// Starts a sound.
#[derive(Debug)]
pub struct PlayCommand {
    /// What a stop of this pad addresses.
    pub pad: u32,
    pub sound: Arc<LoadedSound>,
    /// Linear gain.
    pub gain: f32,
    pub looped: bool,
    /// The part of the sound that plays, in frames: from `start` to `end` (clamped to the sound's length).
    pub start: u64,
    pub end: u64,
    /// For a long sound: where the rest of its blocks come from.
    pub stream: Option<Arc<StreamShared>>,
}

#[derive(Debug)]
pub enum Command {
    Play(PlayCommand),
    /// Stops one pad (all of its voices), or everything, fading out over `fade` frames.
    Stop {
        pad: Option<u32>,
        fade: u32,
    },
}

impl Command {
    /// A stop with the short fade that keeps it from clicking.
    pub fn stop(pad: Option<u32>) -> Command {
        Command::Stop {
            pad,
            fade: FADE_FRAMES,
        }
    }
}

/// Frames a stopping voice fades out over (10 ms), so that nothing clicks. A pad can ask for a longer fade.
pub const FADE_FRAMES: u32 = 480;
/// Finished blocks a voice keeps before it can hand them back (a lost lock for a few cycles).
const RETURN_ROOM: usize = 32;

#[derive(Debug)]
struct Voice {
    pad: u32,
    gain: f32,
    sound: Arc<LoadedSound>,
    /// Next frame to read in the part in memory.
    frame: u64,
    /// Where the played part starts and ends, and the position in the whole sound (for the end).
    start: u64,
    end: u64,
    pos: u64,
    looped: bool,
    stream: Option<Arc<StreamShared>>,
    current: Option<BlockRef>,
    current_at: usize,
    returned: Vec<BlockRef>,
    /// Frames of fade left and the frames the whole fade takes, when the voice is being stopped.
    fading: Option<(u32, u32)>,
    finished: bool,
    age: u64,
}

impl Voice {
    fn new(c: PlayCommand, age: u64) -> Voice {
        let end = c.end.min(c.sound.frames);
        let start = if c.start < end { c.start } else { 0 };
        // a start beyond what is in memory goes straight to the part the reader brings
        let frame = start.min(c.sound.loaded_frames);
        Voice {
            pad: c.pad,
            gain: c.gain,
            frame,
            start,
            end,
            pos: start,
            sound: c.sound,
            looped: c.looped,
            stream: c.stream,
            current: None,
            current_at: 0,
            returned: Vec::with_capacity(RETURN_ROOM),
            fading: None,
            finished: false,
            age,
        }
    }

    /// Hands finished blocks back and takes the next one, when the lock is free. Never waits.
    fn exchange(&mut self, want_next: bool) {
        let Some(shared) = &self.stream else { return };
        if let Ok(mut st) = shared.state.try_lock() {
            st.done.append(&mut self.returned);
            if want_next {
                self.current = st.queue.pop_front();
                self.current_at = 0;
            }
        }
    }

    /// The next frame, `Ok(None)` = silence for now (the disk is late), `Err(())` = the sound is over.
    fn next(&mut self) -> Result<Option<(i16, i16)>, ()> {
        loop {
            if self.pos >= self.end {
                if self.looped && !self.sound.streamed() && self.start < self.end {
                    self.frame = self.start;
                    self.pos = self.start;
                } else {
                    // the block just finished goes back to the reader like any other
                    if let Some(done) = self.current.take() {
                        if self.returned.len() < RETURN_ROOM {
                            self.returned.push(done);
                        }
                    }
                    return Err(());
                }
            }
            if self.frame < self.sound.loaded_frames {
                let f = self.sound.frame(self.frame);
                self.frame += 1;
                self.pos += 1;
                return Ok(Some(f));
            }
            if !self.sound.streamed() {
                return Err(());
            }
            // the rest of a long sound comes from the reader
            if let Some(b) = &self.current {
                if self.current_at < b.len {
                    let s = self.current_at;
                    let l = b.data[s];
                    let r = if self.sound.channels == 2 {
                        b.data[s + 1]
                    } else {
                        l
                    };
                    self.current_at += self.sound.channels;
                    self.pos += 1;
                    return Ok(Some((l, r)));
                }
                let last = b.last;
                if let Some(done) = self.current.take() {
                    if self.returned.len() < RETURN_ROOM {
                        self.returned.push(done);
                    } else {
                        // cannot be kept (the lock was lost for a long time): at worst it is freed here, once
                        drop(done);
                    }
                }
                if last {
                    return Err(());
                }
            }
            self.exchange(true);
            if self.current.is_none() {
                let ended = self
                    .stream
                    .as_ref()
                    .and_then(|s| s.state.try_lock().ok())
                    .is_some_and(|st| st.eof && st.queue.is_empty());
                return if ended { Err(()) } else { Ok(None) };
            }
        }
    }

    /// Adds `frames` frames of this voice to `out` (stereo, interleaved); returns the frames that were late.
    fn mix(&mut self, out: &mut [f32]) -> u64 {
        let mut late = 0;
        for frame in out.as_chunks_mut::<2>().0.iter_mut() {
            let fade = match &mut self.fading {
                Some((0, _)) => {
                    self.finished = true;
                    break;
                }
                Some((n, total)) => {
                    let g = *n as f32 / *total as f32;
                    *n -= 1;
                    g
                }
                None => 1.0,
            };
            match self.next() {
                Ok(Some((l, r))) => {
                    let g = self.gain * fade / 32768.0;
                    frame[0] += l as f32 * g;
                    frame[1] += r as f32 * g;
                }
                Ok(None) => late += 1,
                Err(()) => {
                    self.finished = true;
                    break;
                }
            }
        }
        // give finished blocks back now and then, not only when the next one is wanted
        if !self.returned.is_empty() {
            self.exchange(false);
        }
        late
    }
}

/// The voices of one soundboard.
#[derive(Debug)]
pub struct Voices {
    voices: Vec<Voice>,
    age: u64,
}

impl Default for Voices {
    fn default() -> Self {
        Voices {
            voices: Vec::with_capacity(MAX_VOICES),
            age: 0,
        }
    }
}

impl Voices {
    /// How many voices are playing.
    pub fn count(&self) -> usize {
        self.voices.len()
    }

    /// The pads that have a voice playing (and are not being stopped).
    pub fn playing(&self) -> impl Iterator<Item = u32> + '_ {
        self.voices
            .iter()
            .filter(|v| v.fading.is_none())
            .map(|v| v.pad)
    }

    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Play(c) => {
                self.age += 1;
                let voice = Voice::new(c, self.age);
                if self.voices.len() < MAX_VOICES {
                    self.voices.push(voice);
                } else if let Some(oldest) = self.voices.iter_mut().min_by_key(|v| v.age) {
                    // all the voices are busy: the oldest one makes way (a hard cut, rare, only for a crowd of sounds)
                    *oldest = voice;
                }
            }
            Command::Stop { pad, fade } => {
                let fade = fade.max(1);
                for v in self
                    .voices
                    .iter_mut()
                    .filter(|v| pad.is_none_or(|p| v.pad == p) && v.fading.is_none())
                {
                    v.fading = Some((fade, fade));
                }
                // stopping a long sound also tells its reader to give its blocks back
                for v in self
                    .voices
                    .iter()
                    .filter(|v| pad.is_none_or(|p| v.pad == p))
                {
                    if let Some(s) = &v.stream {
                        if let Ok(mut st) = s.state.try_lock() {
                            st.stop = true;
                        }
                    }
                }
            }
        }
    }

    /// Mixes every voice into `out` (stereo, interleaved; it is cleared first); `late` counts frames the disk was late for.
    pub fn mix(&mut self, out: &mut [f32], late: &AtomicU64) {
        out.fill(0.0);
        let mut frames_late = 0;
        for v in &mut self.voices {
            frames_late += v.mix(out);
        }
        if frames_late > 0 {
            late.fetch_add(frames_late, Ordering::Relaxed);
        }
        // finished voices leave (their blocks were handed back above; a long sound's reader notices on its own)
        let mut i = 0;
        while i < self.voices.len() {
            if self.voices[i].finished {
                if let Some(s) = &self.voices[i].stream {
                    if let Ok(mut st) = s.state.try_lock() {
                        st.stop = true;
                    }
                }
                self.voices.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }
}

/// What the graph thread and the control side share about one soundboard that is in the running graph.
#[derive(Debug)]
pub struct BoardRuntime {
    pub id: String,
    commands: Mutex<Vec<Command>>,
    voices: Mutex<Voices>,
    /// Frames of long sounds the disk was too late for.
    pub late_frames: AtomicU64,
    /// The pads the graph can play (`play1`...): the graph thread counts the actions it gets, the control side plays.
    trigger_pads: Mutex<Vec<String>>,
    plays: [AtomicU32; MAX_TRIGGERS],
    stops: AtomicU32,
}

/// The most trigger pads a soundboard block can have.
pub const MAX_TRIGGERS: usize = 16;

impl BoardRuntime {
    pub fn new(id: &str) -> BoardRuntime {
        BoardRuntime {
            id: id.to_owned(),
            commands: Mutex::new(Vec::with_capacity(64)),
            voices: Mutex::new(Voices::default()),
            late_frames: AtomicU64::new(0),
            trigger_pads: Mutex::new(vec![]),
            plays: std::array::from_fn(|_| AtomicU32::new(0)),
            stops: AtomicU32::new(0),
        }
    }

    /// Which pads the block's action inputs play, in order (set whenever a plan is built).
    pub fn set_triggers(&self, pads: Vec<String>) {
        *self.trigger_pads.lock().unwrap_or_else(|e| e.into_inner()) = pads;
    }

    /// Called by the graph thread: the action input `k` fired. Counting is all it does.
    pub fn request_play(&self, k: usize) {
        if let Some(c) = self.plays.get(k) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Called by the graph thread: the `stop` input fired.
    pub fn request_stop(&self) {
        self.stops.fetch_add(1, Ordering::Relaxed);
    }

    /// For the control side: the pads that were asked for since the last call (each at most once), and whether the
    /// soundboard was asked to stop.
    pub fn take_requests(&self) -> (Vec<String>, bool) {
        let stop = self.stops.swap(0, Ordering::Relaxed) > 0;
        let pads = self.trigger_pads.lock().unwrap_or_else(|e| e.into_inner());
        let mut asked = vec![];
        for (k, counter) in self.plays.iter().enumerate() {
            if counter.swap(0, Ordering::Relaxed) > 0 {
                if let Some(p) = pads.get(k) {
                    asked.push(p.clone());
                }
            }
        }
        (asked, stop)
    }

    /// Hands a command to the graph thread (it runs it at the start of its next cycle).
    pub fn send(&self, command: Command) {
        self.commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(command);
    }

    /// Runs the commands and mixes the voices into `out`: what the graph thread calls each cycle. Never waits: if
    /// the control side holds a lock at this very moment, this cycle is silent (or the commands wait for the next).
    pub fn mix(&self, out: &mut [f32]) {
        let Ok(mut voices) = self.voices.try_lock() else {
            out.fill(0.0);
            return;
        };
        if let Ok(mut commands) = self.commands.try_lock() {
            for c in commands.drain(..) {
                voices.apply(c);
            }
        }
        voices.mix(out, &self.late_frames);
    }

    /// The pads playing now (for the panel).
    pub fn playing_pads(&self) -> Vec<u32> {
        self.voices
            .lock()
            .map(|v| v.playing().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::BlockPool;

    /// A sound of `samples` (interleaved) made of pool blocks.
    pub(crate) fn sound_of(
        pool: &mut BlockPool,
        channels: usize,
        samples: &[i16],
    ) -> Arc<LoadedSound> {
        let mut blocks = vec![];
        for chunk in samples.chunks(BLOCK_SAMPLES) {
            let mut b = pool.take().unwrap();
            let inner = Arc::get_mut(&mut b).unwrap();
            inner.data[..chunk.len()].copy_from_slice(chunk);
            inner.len = chunk.len();
            blocks.push(b);
        }
        Arc::new(LoadedSound {
            id: "s".into(),
            channels,
            frames: (samples.len() / channels) as u64,
            loaded_frames: (samples.len() / channels) as u64,
            blocks,
            path: PathBuf::new(),
        })
    }

    fn play(sound: &Arc<LoadedSound>, pad: u32) -> Command {
        Command::Play(PlayCommand {
            start: 0,
            end: u64::MAX,
            pad,
            sound: sound.clone(),
            gain: 1.0,
            looped: false,
            stream: None,
        })
    }

    fn cycle(v: &mut Voices, frames: usize) -> Vec<f32> {
        let mut out = vec![9.0; frames * 2];
        v.mix(&mut out, &AtomicU64::new(0));
        out
    }

    #[test]
    fn a_sound_comes_out_as_it_is_then_ends_and_its_voice_goes() {
        let mut pool = BlockPool::new(1 << 20);
        let samples: Vec<i16> = (0..400).map(|i| (i * 50 - 10_000) as i16).collect(); // 200 stereo frames
        let sound = sound_of(&mut pool, 2, &samples);
        let mut v = Voices::default();
        v.apply(play(&sound, 1));
        let out = cycle(&mut v, 480);
        for i in 0..400 {
            assert!(
                (out[i] - samples[i] as f32 / 32768.0).abs() < 1e-6,
                "sample {i}"
            );
        }
        assert!(
            out[400..].iter().all(|s| *s == 0.0),
            "silence once it is over"
        );
        assert_eq!(v.count(), 0, "the voice left");
    }

    #[test]
    fn a_mono_sound_plays_on_both_sides_and_the_gain_applies() {
        let mut pool = BlockPool::new(1 << 20);
        let sound = sound_of(&mut pool, 1, &[16_384, -16_384, 8192]);
        let mut v = Voices::default();
        v.apply(Command::Play(PlayCommand {
            start: 0,
            end: u64::MAX,
            pad: 1,
            sound,
            gain: 0.5,
            looped: false,
            stream: None,
        }));
        let out = cycle(&mut v, 4);
        assert_eq!(&out[..6], &[0.25, 0.25, -0.25, -0.25, 0.125, 0.125]);
    }

    #[test]
    fn sounds_overlap_and_are_added() {
        let mut pool = BlockPool::new(1 << 20);
        let a = sound_of(&mut pool, 1, &[8192; 100]);
        let b = sound_of(&mut pool, 1, &[16_384; 100]);
        let mut v = Voices::default();
        v.apply(play(&a, 1));
        v.apply(play(&b, 2));
        let out = cycle(&mut v, 10);
        assert!((out[0] - 0.75).abs() < 1e-6, "{}", out[0]);
        assert_eq!(v.count(), 2);
    }

    #[test]
    fn a_stop_fades_out_instead_of_cutting_and_only_stops_its_pad() {
        let mut pool = BlockPool::new(1 << 20);
        let long = sound_of(&mut pool, 1, &vec![16_384; 48_000]);
        let mut v = Voices::default();
        v.apply(play(&long, 1));
        v.apply(play(&long, 2));
        cycle(&mut v, 100);
        v.apply(Command::stop(Some(1)));
        assert_eq!(
            v.playing().collect::<Vec<_>>(),
            vec![2],
            "pad 1 is on its way out"
        );
        let out = cycle(&mut v, 480);
        // two voices at first (0.5 each), one fading to nothing: the sum goes from 1.0 down to 0.5
        assert!(out[0] > 0.9 && out[0] <= 1.0, "{}", out[0]);
        assert!((out[2 * 479] - 0.5).abs() < 0.01, "{}", out[2 * 479]);
        assert!(
            out.windows(2).all(|w| w[0] >= w[1] - 1e-6),
            "smooth, never a jump up"
        );
        let _ = cycle(&mut v, 10);
        assert_eq!(v.count(), 1, "the stopped voice is gone");
        v.apply(Command::stop(None));
        cycle(&mut v, 480);
        cycle(&mut v, 10);
        assert_eq!(v.count(), 0);
    }

    #[test]
    fn a_looping_sound_goes_on_and_too_many_sounds_make_the_oldest_give_way() {
        let mut pool = BlockPool::new(1 << 20);
        let s = sound_of(&mut pool, 1, &[16_384, 8192]);
        let mut v = Voices::default();
        v.apply(Command::Play(PlayCommand {
            start: 0,
            end: u64::MAX,
            pad: 1,
            sound: s.clone(),
            gain: 1.0,
            looped: true,
            stream: None,
        }));
        let out = cycle(&mut v, 6);
        assert_eq!(
            out.iter().step_by(2).copied().collect::<Vec<_>>(),
            vec![0.5, 0.25, 0.5, 0.25, 0.5, 0.25]
        );
        for pad in 0..(MAX_VOICES as u32 + 5) {
            v.apply(Command::Play(PlayCommand {
                start: 0,
                end: u64::MAX,
                pad: 100 + pad,
                sound: s.clone(),
                gain: 1.0,
                looped: true,
                stream: None,
            }));
        }
        assert_eq!(
            v.count(),
            MAX_VOICES,
            "never more voices than there is room for"
        );
        assert!(!v.playing().any(|p| p == 1), "the oldest gave way");
    }

    /// A long sound: its first frames in memory, the rest as blocks the reader queues.
    fn streamed(
        pool: &mut BlockPool,
        head: &[i16],
        rest: &[Vec<i16>],
    ) -> (Arc<LoadedSound>, Arc<StreamShared>) {
        let mut sound = Arc::try_unwrap(sound_of(pool, 1, head)).unwrap();
        let total: usize = head.len() + rest.iter().map(|b| b.len()).sum::<usize>();
        sound.frames = total as u64;
        let shared = Arc::new(StreamShared::default());
        {
            let mut st = shared.state.lock().unwrap();
            for (i, chunk) in rest.iter().enumerate() {
                let mut b = pool.take().unwrap();
                let inner = Arc::get_mut(&mut b).unwrap();
                inner.data[..chunk.len()].copy_from_slice(chunk);
                inner.len = chunk.len();
                inner.last = i + 1 == rest.len();
                st.queue.push_back(b);
            }
            st.eof = true;
        }
        (Arc::new(sound), shared)
    }

    #[test]
    fn a_long_sound_continues_from_its_first_segment_into_the_blocks_the_reader_sends() {
        let mut pool = BlockPool::new(1 << 20);
        let (sound, shared) = streamed(&mut pool, &[1000; 10], &[vec![2000; 5], vec![3000; 4]]);
        let mut v = Voices::default();
        v.apply(Command::Play(PlayCommand {
            start: 0,
            end: u64::MAX,
            pad: 1,
            sound,
            gain: 1.0,
            looped: false,
            stream: Some(shared.clone()),
        }));
        let out = cycle(&mut v, 30);
        let heard: Vec<i32> = out
            .iter()
            .step_by(2)
            .take(19)
            .map(|s| (s * 32768.0).round() as i32)
            .collect();
        let mut expected = vec![1000; 10];
        expected.extend([2000; 5]);
        expected.extend([3000; 4]);
        assert_eq!(heard, expected);
        assert_eq!(v.count(), 0, "it ended with its last block");
        // the blocks went back for the reader to take
        assert_eq!(shared.state.lock().unwrap().done.len(), 2);
    }

    #[test]
    fn a_disk_that_is_late_is_silence_and_counted_not_a_wait_or_a_crash() {
        let mut pool = BlockPool::new(1 << 20);
        let (sound, shared) = streamed(&mut pool, &[1000; 4], &[]);
        {
            // the reader has not sent anything yet and is not at the end
            let mut st = shared.state.lock().unwrap();
            st.eof = false;
        }
        let mut sound = Arc::try_unwrap(sound).unwrap();
        sound.frames = 100; // a sound of 100 frames of which 4 are in memory
        let mut v = Voices::default();
        v.apply(Command::Play(PlayCommand {
            start: 0,
            end: u64::MAX,
            pad: 1,
            sound: Arc::new(sound),
            gain: 1.0,
            looped: false,
            stream: Some(shared.clone()),
        }));
        let late = AtomicU64::new(0);
        let mut out = vec![0.0; 20];
        v.mix(&mut out, &late);
        assert!(out[..8].iter().any(|s| *s != 0.0), "the head played");
        assert!(
            out[8..].iter().all(|s| *s == 0.0),
            "then silence, not a hang"
        );
        assert_eq!(late.load(Ordering::Relaxed), 6, "6 frames late");
        assert_eq!(v.count(), 1, "the voice waits for its blocks");
        // the reader catches up
        {
            let mut b = pool.take().unwrap();
            let inner = Arc::get_mut(&mut b).unwrap();
            inner.data[..3].copy_from_slice(&[5000, 5000, 5000]);
            inner.len = 3;
            inner.last = true;
            shared.state.lock().unwrap().queue.push_back(b);
        }
        let mut out = vec![0.0; 20];
        v.mix(&mut out, &late);
        assert!(out[0] != 0.0);
        assert_eq!(v.count(), 0);
    }

    #[test]
    fn the_runtime_runs_commands_at_the_start_of_the_cycle_and_never_waits() {
        let mut pool = BlockPool::new(1 << 20);
        let sound = sound_of(&mut pool, 1, &[16_384; 1000]);
        let rt = BoardRuntime::new("b");
        let mut out = vec![1.0; 20];
        rt.mix(&mut out);
        assert!(out.iter().all(|s| *s == 0.0), "nothing playing is silence");
        rt.send(play(&sound, 7));
        rt.mix(&mut out);
        assert!((out[0] - 0.5).abs() < 1e-6);
        assert_eq!(rt.playing_pads(), vec![7]);
        // the control side holds the voices (the panel asks what plays): the graph thread does not wait
        let held = rt.voices.lock().unwrap();
        let t = std::time::Instant::now();
        rt.mix(&mut out);
        assert!(t.elapsed() < std::time::Duration::from_millis(50));
        assert!(out.iter().all(|s| *s == 0.0));
        drop(held);
    }

    #[test]
    fn only_the_chosen_part_of_a_sound_plays_and_a_loop_repeats_that_part() {
        let mut pool = BlockPool::new(1 << 20);
        let samples: Vec<i16> = (0..1000).map(|i| (i + 1) as i16).collect();
        let sound = sound_of(&mut pool, 1, &samples);
        let part = |looped: bool| {
            Command::Play(PlayCommand {
                pad: 1,
                sound: sound.clone(),
                gain: 1.0,
                looped,
                start: 100,
                end: 300,
                stream: None,
            })
        };
        let mut v = Voices::default();
        v.apply(part(false));
        let out = cycle(&mut v, 400);
        for f in 0..200 {
            assert!(
                (out[2 * f] * 32768.0 - (101 + f) as f32).abs() < 0.01,
                "frame {f}"
            );
        }
        assert!(
            out[2 * 200..].iter().all(|x| *x == 0.0),
            "silence after the part"
        );
        assert_eq!(v.count(), 0);

        v.apply(part(true));
        let out = cycle(&mut v, 500);
        // the 201st frame is the first of the part again
        assert!((out[2 * 200] * 32768.0 - 101.0).abs() < 0.01);
        assert!((out[2 * 450] * 32768.0 - (101 + 50) as f32).abs() < 0.01);
        assert_eq!(v.count(), 1, "still looping");

        // a start past the end, or an end before the start, means the whole sound
        let mut v = Voices::default();
        v.apply(Command::Play(PlayCommand {
            pad: 1,
            sound: sound.clone(),
            gain: 1.0,
            looped: false,
            start: 5000,
            end: u64::MAX,
            stream: None,
        }));
        let out = cycle(&mut v, 10);
        assert!((out[0] * 32768.0 - 1.0).abs() < 0.01, "from the beginning");
    }
}
