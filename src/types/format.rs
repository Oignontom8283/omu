use serde::{Deserialize, Serialize};

/// Sample rate of the audio engine's graph. Devices running at another rate are bridged by a resampler.
pub const ENGINE_SAMPLE_RATE: u32 = 48_000;
/// Channel count of the audio engine's graph (interleaved `f32`). Devices with another channel count are
/// down/up-mixed at the boundary.
pub const ENGINE_CHANNELS: u16 = 2;

/// A PCM format: always 32-bit float samples, interleaved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioFormat {
    /// The format every graph edge carries.
    pub const ENGINE: AudioFormat = AudioFormat {
        sample_rate: ENGINE_SAMPLE_RATE,
        channels: ENGINE_CHANNELS,
    };

    /// Number of samples (all channels) in `frames` frames.
    pub fn samples(&self, frames: usize) -> usize {
        frames * self.channels as usize
    }
}
