//! The soundboard: sounds in memory and on disk, and the blocks of the graph that play them.
//!
//! * [`wav`]: the format sounds are stored in (a plain PCM WAV, the simplest thing to read back);
//! * [`pool`]: memory for sounds, as blocks of one size so that nothing ever fragments;
//! * [`voices`]: the real-time side, what plays and the mixer the graph thread runs;
//! * [`library`]: the soundboards people made and the sounds in them, saved in the data folder;
//! * [`hotkeys`]: the keyboard shortcuts that play pads;
//! * [`import`]: turns an MP3, FLAC, OGG or WAV file into a stored sound;
//! * [`engine`]: ties it together: the pool, what is loaded, the thread that reads long sounds ahead of their voices.

pub mod engine;
pub mod hotkeys;
pub mod import;
pub mod library;
pub mod pool;
pub mod voices;
pub mod wav;

pub use engine::{SoundEngine, SoundError};

/// Samples per block: 16384 16-bit samples = 32 KB (8192 stereo frames, about 0.17 s). Small enough that a short sound
/// wastes little, and a whole number of frames for mono and stereo.
pub const BLOCK_SAMPLES: usize = 16_384;
/// The engine's rate, and the one stored sounds have.
pub const ENGINE_RATE: u32 = 48_000;
/// Mono or stereo.
pub const MAX_CHANNELS: usize = 2;
/// A sound whose stored size is at most this is kept whole in memory; a longer one is read from the disk by segments.
pub const WHOLE_SOUND_MAX_BYTES: u64 = 20 * 1024 * 1024;
/// How long one segment of a long sound is, in seconds. The first one stays in memory so that the sound starts at once.
pub const SEGMENT_SECONDS: u64 = 5;
/// How many segments are read ahead of a voice that plays a long sound.
pub const AHEAD_SEGMENTS: u64 = 2;
/// Sounds that can play at the same time on one soundboard.
pub const MAX_VOICES: usize = 32;
/// Long sounds that can play at the same time on the whole engine (each one reads the disk ahead).
pub const MAX_STREAMS: usize = 8;
/// The default memory the pool may use, in megabytes (from the branding; a setting and an environment variable change it).
pub const DEFAULT_CACHE_MB: u32 = crate::branding::SOUND_CACHE_MB as u32;
