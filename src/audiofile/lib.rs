//! Files that hold sound: reading and writing them, and turning whatever people bring (MP3, FLAC, OGG, AAC, the sound of a
//! video) into the format sounds are stored in.
//!
//! * [`wav`]: the plain format sounds are stored in (WAV, 16 bits, 48 kHz);
//! * [`stored`]: a stored sound in either shape it can have, the plain WAV or the same samples compressed without loss;
//! * [`import`]: turns an MP3, FLAC, OGG, AAC or video file into a stored sound.

pub mod import;
pub mod stored;
pub mod wav;

/// The engine's rate, and the one stored sounds have.
pub const ENGINE_RATE: u32 = 48_000;
/// Mono or stereo.
pub const MAX_CHANNELS: usize = 2;
/// How long one segment of a compressed or long sound is, in seconds (the segment the readers work in).
pub const SEGMENT_SECONDS: u64 = 5;
