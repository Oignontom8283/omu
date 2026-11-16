//! The soundboard engine: the memory, what is loaded in it, and the thread that reads long sounds ahead of their voices.
//!
//! * Nothing is reserved until a soundboard is in the running graph ([`SoundEngine::sync`]); the memory is a pool of
//!   same-size blocks that grows when it is asked to, up to a limit, and is given back whole when the last soundboard
//!   leaves the graph.
//! * A sound is loaded the first time it is played (kept whole up to 20 MB, otherwise its first 5 seconds) and stays in
//!   memory. When the pool is full, the least recently played sounds that nobody is playing are unloaded to make room.
//! * A long sound starts from its first segment at once while a thread reads the rest from the disk ahead of it.
//! * What the graph thread runs is in `voices`: it never reads the disk, allocates, waits or frees a sound's memory.

use super::library::{new_id, Library};
use super::pool::{BlockPool, BlockRef};
use super::voices::{BoardRuntime, Command, LoadedSound, PlayCommand, StreamShared, FADE_FRAMES};
use super::{
    AHEAD_SEGMENTS, BLOCK_SAMPLES, DEFAULT_CACHE_MB, ENGINE_RATE, MAX_STREAMS, SEGMENT_SECONDS,
    WHOLE_SOUND_MAX_BYTES,
};
use omu_audiofile::stored::Stored;
use omu_dsp::db_to_linear;
use omu_types::{Board, Pad, Settings, SoundInfo, SoundStatus};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SoundError {
    #[error("no soundboard is in the running graph, so there is no memory for sounds")]
    NotActive,
    #[error("there is no soundboard {0:?}")]
    UnknownBoard(String),
    #[error("there is no pad {0:?} on this soundboard")]
    UnknownPad(String),
    #[error("this pad has no sound")]
    NoSound,
    #[error("there is no sound {0:?}")]
    UnknownSound(String),
    #[error("the soundboard {0:?} is not in the running graph: add its block and apply the graph")]
    NotInGraph(String),
    #[error("the sounds that are playing use all the memory soundboards may use ({0} MB): raise the limit in the settings")]
    PoolFull(u64),
    #[error("too many long sounds are playing at once")]
    TooManyStreams,
    /// The file can only be read with an optional module that is not switched on: what is wrong, and what to do.
    #[error("{0}")]
    ModuleNeeded(String),
    #[error("{0}")]
    Io(String),
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Frames a pad fades out over when stopped: its `fade_ms`, never shorter than the fade that keeps a stop from clicking.
fn fade_frames(fade_ms: u32) -> u32 {
    (fade_ms.min(super::library::MAX_FADE_MS) * ENGINE_RATE / 1000).max(FADE_FRAMES)
}

/// A key for a pad that the graph thread can compare without text.
pub fn pad_key(pad: &str) -> u32 {
    pad.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ b as u32).wrapping_mul(0x0100_0193)
    })
}

struct CacheEntry {
    sound: Arc<LoadedSound>,
    used: u64,
}

struct Inner {
    /// `None` while no soundboard is in the running graph: nothing is reserved.
    pool: Option<BlockPool>,
    cache: HashMap<String, CacheEntry>,
    tick: u64,
}

/// A long sound being read ahead of its voice.
struct StreamJob {
    shared: Arc<StreamShared>,
    stored: Stored,
    next_frame: u64,
    /// Blocks to keep queued ahead of the voice.
    ahead: usize,
}

struct Shared {
    library: Library,
    /// The optional modules (FFmpeg reads the files the program cannot).
    modules: Arc<omu_modules::Modules>,
    /// A fixed limit in bytes (tests, embedding); otherwise the environment variable, then the setting.
    limit_override: Option<u64>,
    inner: Mutex<Inner>,
    /// One sound is loaded at a time.
    load_gate: Mutex<()>,
    runtimes: Mutex<HashMap<String, Arc<BoardRuntime>>>,
    new_jobs: Mutex<Vec<StreamJob>>,
    streams: AtomicUsize,
}

struct Worker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

pub struct SoundEngine {
    shared: Arc<Shared>,
    worker: Mutex<Option<Worker>>,
    /// A second handle on the same state, used by the reading thread to play the pads the graph asks for. Dropping it
    /// leaves everything running.
    view: bool,
}

impl std::fmt::Debug for SoundEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SoundEngine")
    }
}

/// Blocks needed for `samples` samples.
fn blocks_for(samples: u64) -> usize {
    samples.div_ceil(BLOCK_SAMPLES as u64) as usize
}

/// Takes unused sounds out of memory, least recently played first, until `blocks` blocks can be taken. `false` when
/// what is left is all being played.
fn make_room(inner: &mut Inner, blocks: usize) -> bool {
    let Inner { pool, cache, .. } = inner;
    let Some(pool) = pool.as_mut() else {
        return false;
    };
    while pool.available() < blocks {
        let victim = cache
            .iter()
            .filter(|(_, e)| Arc::strong_count(&e.sound) == 1)
            .min_by_key(|(_, e)| e.used)
            .map(|(k, _)| k.clone());
        let Some(key) = victim else {
            return false;
        };
        if let Some(entry) = cache.remove(&key) {
            if let Ok(sound) = Arc::try_unwrap(entry.sound) {
                for b in sound.blocks {
                    pool.give(b);
                }
            }
        }
    }
    true
}

fn give_all(shared: &Shared, blocks: impl IntoIterator<Item = BlockRef>) {
    let mut inner = lock(&shared.inner);
    if let Some(pool) = inner.pool.as_mut() {
        for b in blocks {
            pool.give(b);
        }
    }
}

impl SoundEngine {
    /// The engine for the sounds saved in `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> SoundEngine {
        SoundEngine::build(dir.into(), None)
    }

    /// The same with a fixed memory limit in bytes, whatever the settings say (tests, embedding).
    pub fn with_cache_bytes(dir: impl Into<PathBuf>, bytes: u64) -> SoundEngine {
        SoundEngine::build(dir.into(), Some(bytes))
    }

    fn build(dir: PathBuf, limit_override: Option<u64>) -> SoundEngine {
        SoundEngine {
            shared: Arc::new(Shared {
                modules: Arc::new(omu_modules::Modules::new(&dir)),
                library: Library::load(dir),
                limit_override,
                inner: Mutex::new(Inner {
                    pool: None,
                    cache: HashMap::new(),
                    tick: 0,
                }),
                load_gate: Mutex::new(()),
                runtimes: Mutex::new(HashMap::new()),
                new_jobs: Mutex::new(vec![]),
                streams: AtomicUsize::new(0),
            }),
            worker: Mutex::new(None),
            view: false,
        }
    }

    /// A handle that shares this engine's state without owning it.
    fn view(shared: Arc<Shared>) -> SoundEngine {
        SoundEngine {
            shared,
            worker: Mutex::new(None),
            view: true,
        }
    }

    pub fn library(&self) -> &Library {
        &self.shared.library
    }

    /// The optional modules.
    pub fn modules(&self) -> &Arc<omu_modules::Modules> {
        &self.shared.modules
    }

    /// The memory the pool may grow to, in bytes: the environment variable `<PREFIX>_SOUND_CACHE_MB` if it is set to a
    /// number, else the setting; never below 16 MB.
    pub fn cache_limit_bytes(&self) -> u64 {
        if let Some(b) = self.shared.limit_override {
            return b;
        }
        let mb = std::env::var(omu_branding::env_var("SOUND_CACHE_MB"))
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(self.shared.library.settings().sound_cache_mb as u64);
        mb.max(16) * 1024 * 1024
    }

    pub fn settings(&self) -> Settings {
        self.shared.library.settings()
    }

    pub fn set_settings(&self, settings: Settings) -> Result<(), String> {
        self.shared
            .library
            .set_settings(settings)
            .map_err(|e| format!("cannot save the settings: {e}"))
    }

    /// Tells the engine which soundboards the running graph has. The first one makes the memory (and the reading
    /// thread); when the last one is gone the memory goes back to the system whole.
    pub fn sync(&self, boards_in_graph: &[String]) {
        let known: Vec<String> = boards_in_graph
            .iter()
            .filter(|id| self.shared.library.board(id).is_some())
            .cloned()
            .collect();
        {
            let mut runtimes = lock(&self.shared.runtimes);
            runtimes.retain(|id, _| known.contains(id));
            for id in &known {
                runtimes
                    .entry(id.clone())
                    .or_insert_with(|| Arc::new(BoardRuntime::new(id)));
            }
        }
        if known.is_empty() {
            self.deactivate();
        } else {
            self.activate();
        }
    }

    fn activate(&self) {
        let mut worker = lock(&self.worker);
        if worker.is_some() {
            return;
        }
        lock(&self.shared.inner).pool = Some(BlockPool::new(self.cache_limit_bytes()));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, flag) = (self.shared.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("soundboard-reader".into())
            .spawn(move || worker_main(shared, flag))
            .ok();
        *worker = Some(Worker { stop, thread });
        tracing::info!(
            limit_mb = self.cache_limit_bytes() / (1024 * 1024),
            "the soundboard memory is ready"
        );
    }

    fn deactivate(&self) {
        let Some(mut w) = lock(&self.worker).take() else {
            return;
        };
        w.stop.store(true, Ordering::Relaxed);
        if let Some(t) = w.thread.take() {
            let _ = t.join();
        }
        let mut inner = lock(&self.shared.inner);
        inner.cache.clear();
        inner.pool = None;
        lock(&self.shared.new_jobs).clear();
        self.shared.streams.store(0, Ordering::Relaxed);
        tracing::info!(
            "no soundboard is in the graph any more: the memory of sounds was given back"
        );
    }

    /// What the graph thread runs for a soundboard block, if the soundboard is known and in the graph.
    pub fn runtime(&self, board: &str) -> Option<Arc<BoardRuntime>> {
        lock(&self.shared.runtimes).get(board).cloned()
    }

    /// Imports a file as a sound (converted once to the stored format); long files take a while.
    pub fn import(
        &self,
        source: &std::path::Path,
        name: Option<&str>,
    ) -> Result<SoundInfo, SoundError> {
        let id = new_id();
        let name = name
            .map(str::to_owned)
            .or_else(|| source.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "sound".into());
        let destination = self.shared.library.path_of(&id);
        let info = match omu_audiofile::import::import_file(source, &destination, &id, &name) {
            Ok(info) => info,
            // this program cannot read it: FFmpeg, if the module is on, may
            Err(omu_audiofile::import::ImportError::Unsupported(why)) => {
                match self.shared.modules.ffmpeg() {
                    Some(ffmpeg) => omu_modules::ffmpeg::import(&ffmpeg, source, &destination, &id, &name)
                        .map_err(SoundError::Io)?,
                    None => {
                        return Err(SoundError::ModuleNeeded(format!(
                            "{why}. The FFmpeg module reads many more kinds of files: install it in the Modules panel, then add the file again."
                        )))
                    }
                }
            }
            Err(omu_audiofile::import::ImportError::Failed(e)) => return Err(SoundError::Io(e)),
        };
        self.shared
            .library
            .add_sound(info.clone())
            .map_err(|e| SoundError::Io(e.to_string()))?;
        // a long sound is stored compressed when the settings say so; if that fails the sound stays as it is
        let above = self.shared.library.settings().compress_above_mb as u64 * 1024 * 1024;
        let raw = info.frames * info.channels as u64 * 2;
        if above > 0 && raw > above {
            match self.compress(&id) {
                Ok(packed) => return Ok(packed),
                Err(e) => tracing::warn!("cannot compress the sound {id}: {e}"),
            }
        }
        Ok(info)
    }

    /// Stores a sound compressed, without loss: it takes less room on the disk and plays exactly the same. A long sound
    /// takes a while.
    pub fn compress(&self, id: &str) -> Result<SoundInfo, SoundError> {
        let lib = &self.shared.library;
        let info = lib
            .sound(id)
            .ok_or_else(|| SoundError::UnknownSound(id.to_owned()))?;
        if info.compressed {
            return Ok(info);
        }
        let (plain, packed) = (lib.path_of(id), lib.packed_path_of(id));
        let partial = packed.with_extension("omuf.part");
        omu_audiofile::stored::compress(&plain, &partial).map_err(SoundError::Io)?;
        std::fs::rename(&partial, &packed).map_err(|e| SoundError::Io(e.to_string()))?;
        let bytes = std::fs::metadata(&packed).map_or(info.bytes, |m| m.len());
        let packed_info = SoundInfo {
            bytes,
            compressed: true,
            ..info
        };
        lib.update_sound(packed_info.clone())
            .map_err(|e| SoundError::Io(e.to_string()))?;
        // a sound that is playing may still hold the plain file open (Windows then refuses to delete it): the
        // compressed one is what is read from now on, and the plain one goes when the sound is removed
        if let Err(e) = std::fs::remove_file(&plain) {
            tracing::debug!("the plain file of {id} stays for now: {e}");
        }
        Ok(packed_info)
    }

    /// Where an uploaded file is to be written (a browser sends the bytes of a file, not its path): next to the sounds,
    /// with the extension of the name it came with (it helps to recognise the format). Finish with
    /// [`SoundEngine::finish_upload`].
    pub fn upload_slot(&self, name: &str) -> Result<PathBuf, SoundError> {
        let dir = self.shared.library.sounds_dir();
        std::fs::create_dir_all(&dir).map_err(|e| SoundError::Io(e.to_string()))?;
        let ext = std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("bin");
        Ok(dir.join(format!("{}.upload.{ext}", new_id())))
    }

    /// Imports the file written at an upload slot, and deletes it.
    pub fn finish_upload(
        &self,
        path: &std::path::Path,
        name: &str,
    ) -> Result<SoundInfo, SoundError> {
        let result = self.import(path, Some(name));
        let _ = std::fs::remove_file(path);
        result
    }

    /// Imports sound already read from a file, from its bytes.
    pub fn import_bytes(&self, bytes: &[u8], name: &str) -> Result<SoundInfo, SoundError> {
        let slot = self.upload_slot(name)?;
        std::fs::write(&slot, bytes).map_err(|e| SoundError::Io(e.to_string()))?;
        self.finish_upload(&slot, name)
    }

    /// Removes a sound for good (its stored file too), and from memory if nobody is playing it.
    pub fn remove_sound(&self, id: &str) -> Result<bool, SoundError> {
        let removed = self
            .shared
            .library
            .remove_sound(id)
            .map_err(|e| SoundError::Io(e.to_string()))?;
        let mut inner = lock(&self.shared.inner);
        let unused = inner
            .cache
            .get(id)
            .is_some_and(|e| Arc::strong_count(&e.sound) == 1);
        if unused {
            if let Some(entry) = inner.cache.remove(id) {
                if let (Ok(sound), Some(pool)) = (Arc::try_unwrap(entry.sound), inner.pool.as_mut())
                {
                    for b in sound.blocks {
                        pool.give(b);
                    }
                }
            }
        }
        Ok(removed)
    }

    /// The sound in memory (loading it if it is not): all of it when it is short enough, else its first segment.
    pub fn load(&self, id: &str) -> Result<Arc<LoadedSound>, SoundError> {
        {
            let mut inner = lock(&self.shared.inner);
            inner.tick += 1;
            let tick = inner.tick;
            if let Some(e) = inner.cache.get_mut(id) {
                e.used = tick;
                return Ok(e.sound.clone());
            }
            if inner.pool.is_none() {
                return Err(SoundError::NotActive);
            }
        }
        let _one_at_a_time = lock(&self.shared.load_gate);
        // another thread may have loaded it while this one waited
        if let Some(e) = lock(&self.shared.inner).cache.get(id) {
            return Ok(e.sound.clone());
        }
        let path = self.shared.library.stored_path(id);
        let mut stored = Stored::open(&path)
            .map_err(|e| SoundError::Io(format!("cannot open the sound: {e}")))?;
        let (channels, total_frames) = (stored.channels(), stored.frames());
        let limit = self.cache_limit_bytes();
        // the size that matters here is the plain samples, however the sound is stored
        let raw_bytes = total_frames * channels as u64 * 2;
        let whole = raw_bytes <= WHOLE_SOUND_MAX_BYTES.min(limit / 4);
        let frames = if whole {
            total_frames
        } else {
            total_frames.min(SEGMENT_SECONDS * ENGINE_RATE as u64)
        };
        let samples = frames * channels as u64;
        let needed = blocks_for(samples);

        // take the blocks (making room if need be), then fill them without holding the lock
        let mut blocks: Vec<BlockRef> = Vec::with_capacity(needed);
        {
            let mut inner = lock(&self.shared.inner);
            if !make_room(&mut inner, needed) {
                return Err(SoundError::PoolFull(limit / (1024 * 1024)));
            }
            let Some(pool) = inner.pool.as_mut() else {
                return Err(SoundError::NotActive);
            };
            for _ in 0..needed {
                blocks.push(pool.take().expect("room was made"));
            }
        }
        let mut failure = None;
        for (i, block) in blocks.iter_mut().enumerate() {
            let inner_block = Arc::get_mut(block).expect("a block just taken is held by nobody");
            let want = (samples - (i * BLOCK_SAMPLES) as u64).min(BLOCK_SAMPLES as u64) as usize;
            let start_frame = (i * BLOCK_SAMPLES / channels) as u64;
            match stored.read_samples(start_frame, &mut inner_block.data[..want]) {
                Ok(n) if n == want => inner_block.len = want,
                Ok(_) => {
                    failure = Some("the stored sound is shorter than its header says".to_string());
                    break;
                }
                Err(e) => {
                    failure = Some(e.to_string());
                    break;
                }
            }
        }
        if let Some(why) = failure {
            give_all(&self.shared, blocks);
            return Err(SoundError::Io(why));
        }
        let sound = Arc::new(LoadedSound {
            id: id.to_owned(),
            channels,
            frames: total_frames,
            loaded_frames: frames,
            blocks,
            path,
        });
        let mut inner = lock(&self.shared.inner);
        inner.tick += 1;
        let used = inner.tick;
        inner.cache.insert(
            id.to_owned(),
            CacheEntry {
                sound: sound.clone(),
                used,
            },
        );
        Ok(sound)
    }

    /// Starts reading the rest of a long sound; `from` is the first frame the reader must bring (never before the
    /// end of what is in memory).
    fn start_stream(
        &self,
        sound: &LoadedSound,
        from: u64,
    ) -> Result<Arc<StreamShared>, SoundError> {
        {
            let waiting = lock(&self.shared.new_jobs);
            if self.shared.streams.load(Ordering::Relaxed) + waiting.len() >= MAX_STREAMS {
                return Err(SoundError::TooManyStreams);
            }
        }
        // the file may have been compressed since the sound was loaded: ask where it is now
        let stored = Stored::open(&self.shared.library.stored_path(&sound.id))
            .map_err(|e| SoundError::Io(e.to_string()))?;
        let segment_blocks =
            blocks_for(SEGMENT_SECONDS * ENGINE_RATE as u64 * stored.channels() as u64);
        let shared = Arc::new(StreamShared::default());
        lock(&self.shared.new_jobs).push(StreamJob {
            shared: shared.clone(),
            stored,
            next_frame: from.max(sound.loaded_frames),
            ahead: AHEAD_SEGMENTS as usize * segment_blocks,
        });
        Ok(shared)
    }

    fn pad_of(&self, board: &str, pad: &str) -> Result<(Board, Pad), SoundError> {
        let b = self
            .shared
            .library
            .board(board)
            .ok_or_else(|| SoundError::UnknownBoard(board.to_owned()))?;
        let p = b
            .pads
            .iter()
            .find(|p| p.id == pad)
            .cloned()
            .ok_or_else(|| SoundError::UnknownPad(pad.to_owned()))?;
        Ok((b, p))
    }

    /// Plays a pad of a soundboard that is in the running graph.
    pub fn play(&self, board: &str, pad: &str) -> Result<(), SoundError> {
        let (_, p) = self.pad_of(board, pad)?;
        let sound_id = p.sound.clone().ok_or(SoundError::NoSound)?;
        let runtime = self
            .runtime(board)
            .ok_or_else(|| SoundError::NotInGraph(board.to_owned()))?;
        let sound = self.load(&sound_id)?;
        let ms_to_frames = |ms: u32| ms as u64 * ENGINE_RATE as u64 / 1000;
        let start = ms_to_frames(p.start_ms);
        let end = if p.end_ms == 0 {
            u64::MAX
        } else {
            ms_to_frames(p.end_ms)
        };
        let stream = if sound.streamed() {
            Some(self.start_stream(&sound, start)?)
        } else {
            None
        };
        if p.exclusive {
            runtime.send(Command::stop(None));
        }
        runtime.send(Command::Play(PlayCommand {
            pad: pad_key(pad),
            looped: p.looped && stream.is_none(),
            start,
            end,
            gain: db_to_linear(p.gain_db),
            sound,
            stream,
        }));
        Ok(())
    }

    /// Stops one pad of a soundboard, or all of it.
    pub fn stop(&self, board: &str, pad: Option<&str>) -> Result<(), SoundError> {
        let runtime = self
            .runtime(board)
            .ok_or_else(|| SoundError::NotInGraph(board.to_owned()))?;
        let fade = pad
            .and_then(|id| self.pad_of(board, id).ok())
            .map_or(FADE_FRAMES, |(_, p)| fade_frames(p.fade_ms));
        runtime.send(Command::Stop {
            pad: pad.map(pad_key),
            fade,
        });
        Ok(())
    }

    /// The pads of a soundboard that are playing now.
    pub fn playing(&self, board: &str) -> Vec<String> {
        let (Some(runtime), Some(b)) = (self.runtime(board), self.shared.library.board(board))
        else {
            return vec![];
        };
        let keys = runtime.playing_pads();
        b.pads
            .into_iter()
            .filter(|p| keys.contains(&pad_key(&p.id)))
            .map(|p| p.id)
            .collect()
    }

    pub fn status(&self) -> SoundStatus {
        let inner = lock(&self.shared.inner);
        let block_bytes = (BLOCK_SAMPLES * 2) as u64;
        let (allocated, used) = inner
            .pool
            .as_ref()
            .map_or((0, 0), |p| (p.allocated() as u64, p.in_use() as u64));
        let runtimes = lock(&self.shared.runtimes);
        SoundStatus {
            active: inner.pool.is_some(),
            cache_limit_bytes: self.cache_limit_bytes(),
            cache_allocated_bytes: allocated * block_bytes,
            cache_used_bytes: used * block_bytes,
            sounds_loaded: inner.cache.len(),
            active_boards: {
                let mut ids: Vec<String> = runtimes.keys().cloned().collect();
                ids.sort();
                ids
            },
            late_frames: runtimes
                .values()
                .map(|r| r.late_frames.load(Ordering::Relaxed))
                .sum(),
            streams: self.shared.streams.load(Ordering::Relaxed),
            hotkey_problems: vec![],
        }
    }

    /// The default limit, for messages.
    pub fn default_cache_mb() -> u32 {
        DEFAULT_CACHE_MB
    }
}

impl Drop for SoundEngine {
    fn drop(&mut self) {
        if !self.view {
            self.deactivate();
        }
    }
}

/// The reading thread: keeps every long sound's queue full, takes back the blocks voices are done with, and forgets
/// sounds that were stopped or whose voice is gone.
fn worker_main(shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    let mut jobs: Vec<StreamJob> = vec![];
    let me = SoundEngine::view(shared.clone());
    while !stop.load(Ordering::Relaxed) {
        {
            // taking the new jobs and counting them are one step for anyone checking the limit under the same lock
            let mut queue = lock(&shared.new_jobs);
            jobs.append(&mut queue);
            shared.streams.store(jobs.len(), Ordering::Relaxed);
        }
        let mut worked = false;
        play_what_the_graph_asked(&shared, &me, &mut worked);
        jobs.retain_mut(|job| service(&shared, job, &mut worked));
        shared.streams.store(jobs.len(), Ordering::Relaxed);
        if !worked {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Plays (and stops) the pads the graph's actions asked for since the last pass: the graph thread only counts them,
/// because loading a sound is not something it may do.
fn play_what_the_graph_asked(shared: &Shared, me: &SoundEngine, worked: &mut bool) {
    let runtimes: Vec<Arc<BoardRuntime>> = lock(&shared.runtimes).values().cloned().collect();
    for runtime in runtimes {
        let (pads, stop) = runtime.take_requests();
        if stop {
            let _ = me.stop(&runtime.id, None);
            *worked = true;
        }
        for pad in pads {
            if let Err(e) = me.play(&runtime.id, &pad) {
                tracing::debug!("a pad the graph asked for: {e}");
            }
            *worked = true;
        }
    }
}

/// One pass over one long sound. Returns whether to keep it.
fn service(shared: &Shared, job: &mut StreamJob, worked: &mut bool) -> bool {
    let (done, stopped) = {
        let mut st = lock(&job.shared.state);
        (std::mem::take(&mut st.done), st.stop)
    };
    if !done.is_empty() {
        give_all(shared, done);
    }
    // a voice that is gone (it ended, or the graph changed) leaves us the only holder
    if stopped || Arc::strong_count(&job.shared) == 1 {
        let (queue, done) = {
            let mut st = lock(&job.shared.state);
            (std::mem::take(&mut st.queue), std::mem::take(&mut st.done))
        };
        give_all(shared, queue.into_iter().chain(done));
        return false;
    }
    let channels = job.stored.channels();
    for _ in 0..4 {
        let (queued, eof) = {
            let st = lock(&job.shared.state);
            (st.queue.len(), st.eof)
        };
        if eof || queued >= job.ahead {
            break;
        }
        // a block, making room if it takes unloading a sound nobody plays
        let block = {
            let mut inner = lock(&shared.inner);
            if !make_room(&mut inner, 1) {
                None
            } else {
                inner.pool.as_mut().and_then(|p| p.take())
            }
        };
        let Some(mut block) = block else { break };
        let inner_block = Arc::get_mut(&mut block).expect("a block just taken is held by nobody");
        let n = match job
            .stored
            .read_samples(job.next_frame, &mut inner_block.data[..])
        {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("cannot read a sound from the disk: {e}");
                0
            }
        };
        if n == 0 {
            give_all(shared, [block]);
            lock(&job.shared.state).eof = true;
            break;
        }
        inner_block.len = n;
        job.next_frame += (n / channels) as u64;
        let at_end = job.next_frame >= job.stored.frames();
        inner_block.last = at_end;
        let mut st = lock(&job.shared.state);
        st.queue.push_back(block);
        if at_end {
            st.eof = true;
        }
        *worked = true;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use omu_audiofile::wav::WavWriter;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("omu-sounds-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A stored sound of `frames` mono frames: a ramp that repeats, so that any piece can be checked.
    fn add_sound(engine: &SoundEngine, id: &str, frames: u64) {
        std::fs::create_dir_all(engine.library().sounds_dir()).unwrap();
        let mut w = WavWriter::create(&engine.library().path_of(id), 1).unwrap();
        let mut left = frames;
        let mut at = 0u64;
        while left > 0 {
            let n = left.min(50_000);
            let chunk: Vec<i16> = (at..at + n).map(ramp).collect();
            w.write(&chunk).unwrap();
            at += n;
            left -= n;
        }
        w.finish().unwrap();
        engine
            .library()
            .add_sound(SoundInfo {
                id: id.into(),
                name: id.into(),
                channels: 1,
                frames,
                bytes: 44 + frames * 2,
                compressed: false,
            })
            .unwrap();
    }

    /// Waits (at most 5 s) for something the reading thread does on its own time.
    fn wait_until(mut done: impl FnMut() -> bool) {
        for _ in 0..500 {
            if done() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn ramp(frame: u64) -> i16 {
        (frame % 20_000) as i16 + 1
    }

    fn board(engine: &SoundEngine, id: &str, pads: &[(&str, &str)]) {
        let mut boards = engine.library().get().boards;
        boards.push(Board {
            id: id.into(),
            name: id.into(),
            pads: pads
                .iter()
                .map(|(p, s)| Pad {
                    id: (*p).into(),
                    name: (*p).into(),
                    sound: Some((*s).into()),
                    gain_db: 0.0,
                    looped: false,
                    exclusive: false,
                    fade_ms: 0,
                    start_ms: 0,
                    end_ms: 0,
                    color: None,
                    hold: false,
                    key: None,
                })
                .collect(),
        });
        engine.library().set_boards(boards).unwrap();
    }

    fn blocks_of(frames: u64) -> u64 {
        blocks_for(frames) as u64
    }

    #[test]
    fn nothing_is_reserved_until_a_soundboard_is_in_the_graph_and_all_is_given_back_when_the_last_leaves(
    ) {
        let dir = temp("lazy");
        let e = SoundEngine::with_cache_bytes(&dir, 64 * 32 * 1024);
        add_sound(&e, "a", 20_000);
        board(&e, "b1", &[("p", "a")]);
        let s = e.status();
        assert!(!s.active && s.cache_allocated_bytes == 0, "{s:?}");
        assert_eq!(e.load("a").unwrap_err(), SoundError::NotActive);
        // a soundboard the graph does not have does not count either
        e.sync(&["unknown".into()]);
        assert!(!e.status().active);
        e.sync(&["b1".into()]);
        let s = e.status();
        assert!(s.active && s.active_boards == vec!["b1".to_string()]);
        assert_eq!(
            s.cache_allocated_bytes, 0,
            "the pool is ready but has made no block yet"
        );
        let sound = e.load("a").unwrap();
        assert_eq!(
            e.status().cache_allocated_bytes,
            blocks_of(20_000) * 32 * 1024
        );
        assert_eq!(sound.frames, 20_000);
        drop(sound);
        e.sync(&[]);
        let s = e.status();
        assert!(
            !s.active && s.cache_allocated_bytes == 0 && s.sounds_loaded == 0,
            "{s:?}"
        );
        assert!(e.runtime("b1").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_loaded_sound_stays_and_when_the_pool_is_full_the_least_recently_played_goes_first() {
        let dir = temp("lru");
        // room for 25 blocks; each sound takes 10 (163840 mono frames = 10 blocks of 16384)
        let e = SoundEngine::with_cache_bytes(&dir, 25 * 32 * 1024);
        for id in ["a", "b", "c"] {
            add_sound(&e, id, 163_840);
        }
        board(&e, "x", &[]);
        e.sync(&["x".into()]);
        let a = e.load("a").unwrap();
        let b = e.load("b").unwrap();
        assert!(
            Arc::ptr_eq(&a, &e.load("a").unwrap()),
            "the second time it is the one in memory"
        );
        drop((a, b));
        // a was played last: loading c takes b out
        e.load("c").unwrap();
        let loaded: Vec<String> = {
            let inner = lock(&e.shared.inner);
            let mut k: Vec<String> = inner.cache.keys().cloned().collect();
            k.sort();
            k
        };
        assert_eq!(loaded, vec!["a", "c"], "b was the least recently played");
        assert!(e.status().cache_used_bytes <= 25 * 32 * 1024);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sound_that_is_playing_is_never_unloaded_to_make_room() {
        let dir = temp("pinned");
        let e = SoundEngine::with_cache_bytes(&dir, 25 * 32 * 1024);
        for id in ["a", "b", "c"] {
            add_sound(&e, id, 163_840);
        }
        board(&e, "x", &[]);
        e.sync(&["x".into()]);
        let playing_a = e.load("a").unwrap(); // held, as a voice holds it
        let playing_b = e.load("b").unwrap();
        // nothing can be unloaded: refused, with the memory untouched
        assert!(matches!(e.load("c").unwrap_err(), SoundError::PoolFull(_)));
        assert_eq!(playing_a.blocks.len(), 10);
        drop(playing_b);
        // b is free to go now
        assert!(e.load("c").is_ok());
        drop(playing_a);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_short_sound_plays_through_the_runtime_and_a_pad_without_a_graph_is_refused_with_a_reason()
    {
        let dir = temp("play");
        let e = SoundEngine::with_cache_bytes(&dir, 64 * 32 * 1024);
        add_sound(&e, "a", 1000);
        board(&e, "b", &[("p", "a")]);
        assert!(matches!(
            e.play("b", "p").unwrap_err(),
            SoundError::NotInGraph(_)
        ));
        assert!(matches!(
            e.play("nope", "p").unwrap_err(),
            SoundError::UnknownBoard(_)
        ));
        e.sync(&["b".into()]);
        assert!(matches!(
            e.play("b", "nope").unwrap_err(),
            SoundError::UnknownPad(_)
        ));
        e.play("b", "p").unwrap();
        let rt = e.runtime("b").unwrap();
        // one cycle longer than the sound: the voice sees its end and leaves
        let mut out = vec![0.0; 4000];
        rt.mix(&mut out);
        for f in 0..1000 {
            let want = ramp(f) as f32 / 32768.0;
            assert!((out[2 * f as usize] - want).abs() < 1e-6, "frame {f}");
        }
        assert!(
            out[2 * 1000..].iter().all(|s| *s == 0.0),
            "silence after the sound"
        );
        assert_eq!(e.playing("b"), Vec::<String>::new(), "it has ended");
        e.play("b", "p").unwrap();
        rt.mix(&mut vec![0.0; 100]);
        assert_eq!(e.playing("b"), vec!["p".to_string()]);
        e.stop("b", None).unwrap();
        for _ in 0..3 {
            rt.mix(&mut vec![0.0; 960]);
        }
        assert!(e.playing("b").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pad_that_stops_the_others_does_so_and_a_pad_fades_out_over_the_time_it_asks_for() {
        let dir = temp("options");
        let e = SoundEngine::with_cache_bytes(&dir, 64 * 32 * 1024);
        add_sound(&e, "a", 200_000);
        board(&e, "b", &[("p1", "a"), ("p2", "a"), ("slow", "a")]);
        let mut boards = e.library().get().boards;
        boards[0].pads[1].exclusive = true;
        boards[0].pads[2].fade_ms = 500;
        e.library().set_boards(boards).unwrap();
        e.sync(&["b".into()]);
        let rt = e.runtime("b").unwrap();
        let cycle = |frames: usize| {
            let mut out = vec![0.0; frames * 2];
            rt.mix(&mut out);
            out
        };

        e.play("b", "p1").unwrap();
        cycle(480);
        assert_eq!(e.playing("b"), vec!["p1".to_string()]);
        // p2 stops the others: p1 fades out within 10 ms and only p2 is left
        e.play("b", "p2").unwrap();
        cycle(480);
        cycle(480);
        assert_eq!(e.playing("b"), vec!["p2".to_string()]);

        // a pad that is not exclusive leaves the others alone
        e.play("b", "p1").unwrap();
        cycle(480);
        assert_eq!(e.playing("b").len(), 2);
        e.stop("b", None).unwrap();
        cycle(960);
        assert!(e.playing("b").is_empty());

        // a fade of 500 ms: still there after 400 ms, gone after 600
        e.play("b", "slow").unwrap();
        cycle(480);
        e.stop("b", Some("slow")).unwrap();
        // a pad on its way out is no longer "playing", but it is still heard until its fade ends
        cycle(48_000 * 4 / 10);
        let still = cycle(480);
        assert!(still.iter().any(|x| *x != 0.0), "still audible at 400 ms");
        cycle(48_000 * 2 / 10);
        let gone = cycle(480);
        assert!(gone.iter().all(|x| *x == 0.0), "silent after 600 ms");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_sound_starts_from_memory_and_continues_from_the_disk_without_a_gap_and_gives_its_blocks_back(
    ) {
        let dir = temp("long");
        // 4 MB of memory: whole sounds are kept up to 1 MB, this one is 30 s of mono = 2.9 MB
        let e = SoundEngine::with_cache_bytes(&dir, 4 * 1024 * 1024);
        let frames = 30 * 48_000u64;
        add_sound(&e, "long", frames);
        board(&e, "b", &[("p", "long")]);
        e.sync(&["b".into()]);
        let head = e.load("long").unwrap();
        assert!(head.streamed());
        assert_eq!(
            head.loaded_frames,
            5 * 48_000,
            "the first segment, 5 seconds"
        );
        drop(head);
        e.play("b", "p").unwrap();
        let rt = e.runtime("b").unwrap();
        // the graph thread: 10 ms cycles, paced so that the reading thread has time to work (not real time)
        let mut heard = Vec::new();
        for cycle in 0..20_000 {
            let mut out = vec![0.0f32; 960];
            rt.mix(&mut out);
            heard.extend(out.iter().step_by(2).copied());
            if cycle % 100 == 99 {
                std::thread::sleep(Duration::from_millis(8));
            }
            // until the voice has ended (a busy machine makes the disk late: that is silence, and the sound waits)
            if cycle > 10 && e.playing("b").is_empty() {
                break;
            }
        }
        // the ramp never holds a zero, so zeros are the silence of a late disk: take them out, what is left is the sound
        heard.retain(|s| *s != 0.0);
        let sound_frames = heard.len();
        assert!(
            (sound_frames as i64 - frames as i64).abs() <= 2,
            "{sound_frames} of {frames} frames"
        );
        // every frame is the ramp: no gap, no repeat, no skipped block, at the seams of head, segments and blocks
        let wrong = (0..sound_frames)
            .filter(|&f| (heard[f] - ramp(f as u64) as f32 / 32768.0).abs() > 1e-6)
            .count();
        assert_eq!(wrong, 0, "frames that are not what the file holds");
        // the reader notices the voice is gone and takes everything back: only the head stays in memory
        wait_until(|| e.status().streams == 0);
        assert_eq!(e.status().streams, 0);
        let used = e.status().cache_used_bytes;
        assert_eq!(
            used,
            blocks_of(5 * 48_000) * 32 * 1024,
            "only the first segment is kept"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_sound_can_start_beyond_its_first_segment_and_stop_early() {
        let dir = temp("trim");
        let e = SoundEngine::with_cache_bytes(&dir, 4 * 1024 * 1024);
        add_sound(&e, "long", 30 * 48_000);
        board(&e, "b", &[("p", "long")]);
        let mut boards = e.library().get().boards;
        boards[0].pads[0].start_ms = 12_000;
        boards[0].pads[0].end_ms = 14_000;
        e.library().set_boards(boards).unwrap();
        e.sync(&["b".into()]);
        e.play("b", "p").unwrap();
        let rt = e.runtime("b").unwrap();
        let mut heard = Vec::new();
        for cycle in 0..20_000 {
            let mut out = vec![0.0f32; 960];
            rt.mix(&mut out);
            heard.extend(out.iter().step_by(2).copied());
            if cycle % 50 == 49 {
                std::thread::sleep(Duration::from_millis(8));
            }
            if cycle > 10 && e.playing("b").is_empty() {
                break;
            }
        }
        heard.retain(|s| *s != 0.0);
        let first = 0;
        let last = heard.len() - 1;
        let begin = 12_000 * 48;
        // every audible frame is the ramp from the chosen start, in order
        let n = last + 1 - first;
        assert!(n >= 90_000, "most of the two seconds was heard: {n} frames");
        let wrong = (first..=last)
            .filter(|&f| {
                (heard[f] - ramp((begin + (f - first)) as u64) as f32 / 32768.0).abs() > 1e-6
            })
            .count();
        assert_eq!(wrong, 0);
        assert!(n <= 96_001, "and it stopped at 14 s: {n} frames");
        // stopping early gives every block back to the pool
        wait_until(|| e.status().streams == 0);
        assert_eq!(e.status().streams, 0);
        assert_eq!(
            e.status().cache_used_bytes,
            blocks_of(5 * 48_000) * 32 * 1024,
            "only the first segment is kept"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_compressed_long_sound_plays_exactly_like_a_plain_one_and_the_plain_file_goes() {
        let dir = temp("packed-play");
        let e = SoundEngine::with_cache_bytes(&dir, 4 * 1024 * 1024);
        let frames = 30 * 48_000u64;
        add_sound(&e, "long", frames);
        board(&e, "b", &[("p", "long")]);
        let plain_bytes = std::fs::metadata(e.library().path_of("long"))
            .unwrap()
            .len();
        let info = e.compress("long").unwrap();
        assert!(info.compressed && info.bytes < plain_bytes, "{info:?}");
        assert_eq!(e.library().sound("long").unwrap(), info);
        assert!(e.library().packed_path_of("long").exists());
        assert!(
            !e.library().path_of("long").exists(),
            "the plain file is gone"
        );
        assert!(e.compress("long").unwrap().compressed, "again is harmless");
        assert!(matches!(
            e.compress("nope").unwrap_err(),
            SoundError::UnknownSound(_)
        ));

        e.sync(&["b".into()]);
        e.play("b", "p").unwrap();
        let rt = e.runtime("b").unwrap();
        let mut heard = Vec::new();
        for cycle in 0..20_000 {
            let mut out = vec![0.0f32; 960];
            rt.mix(&mut out);
            heard.extend(out.iter().step_by(2).copied());
            if cycle % 100 == 99 {
                std::thread::sleep(Duration::from_millis(8));
            }
            // until the voice has ended (a busy machine makes the disk late: that is silence, and the sound waits)
            if cycle > 10 && e.playing("b").is_empty() {
                break;
            }
        }
        // the ramp never holds a zero, so zeros are the silence of a late disk: take them out, what is left is the sound
        heard.retain(|s| *s != 0.0);
        let sound_frames = heard.len();
        assert!(
            (sound_frames as i64 - frames as i64).abs() <= 2,
            "{sound_frames}"
        );
        let wrong = (0..sound_frames)
            .filter(|&f| (heard[f] - ramp(f as u64) as f32 / 32768.0).abs() > 1e-6)
            .count();
        assert_eq!(wrong, 0, "frames that are not what was stored");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_import_is_compressed_when_the_settings_say_so_and_a_short_one_is_left_alone() {
        let dir = temp("auto-pack");
        let e = SoundEngine::with_cache_bytes(&dir, 64 * 1024 * 1024);
        let make = |name: &str, frames: u64| {
            let path = dir.join(name);
            let mut w = WavWriter::create(&path, 1).unwrap();
            let samples: Vec<i16> = (0..frames).map(ramp).collect();
            w.write(&samples).unwrap();
            w.finish().unwrap();
            path
        };
        e.set_settings(Settings {
            compress_above_mb: 1,
            ..Default::default()
        })
        .unwrap();
        let long = e.import(&make("long.wav", 700_000), None).unwrap();
        assert!(long.compressed, "1.4 MB of samples is above 1 MB");
        assert!(e.library().packed_path_of(&long.id).exists());
        let short = e.import(&make("short.wav", 100_000), None).unwrap();
        assert!(!short.compressed && e.library().path_of(&short.id).exists());
        // 0 never compresses
        e.set_settings(Settings {
            compress_above_mb: 0,
            ..Default::default()
        })
        .unwrap();
        let again = e.import(&make("again.wav", 700_000), None).unwrap();
        assert!(!again.compressed);
        // removing a sound removes whichever file it has
        assert!(e.remove_sound(&long.id).unwrap());
        assert!(!e.library().packed_path_of(&long.id).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stopped_long_sound_gives_its_blocks_back_at_once_and_too_many_long_sounds_are_refused() {
        let dir = temp("stop");
        let e = SoundEngine::with_cache_bytes(&dir, 4 * 1024 * 1024);
        add_sound(&e, "long", 30 * 48_000);
        let pads: Vec<(String, &str)> = (0..=MAX_STREAMS)
            .map(|i| (format!("p{i}"), "long"))
            .collect();
        let refs: Vec<(&str, &str)> = pads.iter().map(|(a, b)| (a.as_str(), *b)).collect();
        board(&e, "b", &refs);
        e.sync(&["b".into()]);
        for i in 0..MAX_STREAMS {
            e.play("b", &format!("p{i}")).unwrap();
        }
        assert_eq!(
            e.play("b", &format!("p{MAX_STREAMS}")).unwrap_err(),
            SoundError::TooManyStreams
        );
        let rt = e.runtime("b").unwrap();
        let mut out = vec![0.0; 960];
        rt.mix(&mut out);
        e.stop("b", None).unwrap();
        for _ in 0..3 {
            rt.mix(&mut out);
        }
        wait_until(|| e.status().streams == 0);
        let s = e.status();
        assert_eq!(s.streams, 0, "{s:?}");
        assert_eq!(
            s.cache_used_bytes,
            blocks_of(5 * 48_000) * 32 * 1024,
            "only the head is left in memory"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_a_sound_takes_it_out_of_memory_and_off_the_disk_and_a_status_tells_the_truth() {
        let dir = temp("remove");
        let e = SoundEngine::with_cache_bytes(&dir, 64 * 32 * 1024);
        add_sound(&e, "a", 20_000);
        board(&e, "b", &[("p", "a")]);
        e.sync(&["b".into()]);
        e.load("a").unwrap();
        assert!(e.status().sounds_loaded == 1 && e.status().cache_used_bytes > 0);
        assert!(e.remove_sound("a").unwrap());
        let s = e.status();
        assert_eq!((s.sounds_loaded, s.cache_used_bytes), (0, 0));
        assert!(!e.library().path_of("a").exists());
        assert_eq!(e.library().get().boards[0].pads[0].sound, None);
        assert!(matches!(e.play("b", "p").unwrap_err(), SoundError::NoSound));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_limit_comes_from_the_setting_the_environment_wins_and_there_is_a_floor() {
        let dir = temp("limit");
        let e = SoundEngine::new(&dir);
        assert_eq!(e.cache_limit_bytes(), DEFAULT_CACHE_MB as u64 * 1024 * 1024);
        e.set_settings(Settings {
            sound_cache_mb: 64,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(e.cache_limit_bytes(), 64 * 1024 * 1024);
        e.set_settings(Settings {
            sound_cache_mb: 1,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(e.cache_limit_bytes(), 16 * 1024 * 1024, "never below 16 MB");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
