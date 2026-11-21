//! Opus, the codec of the sound that goes through WebRTC: 48 kHz, two channels, 20 ms at a time.
//!
//! The codec is `libopus` translated to Rust (the `unsafe-libopus` library: no C compiler, no cmake). Its functions are
//! raw pointers; these two small types are the only place they are used, and they hand out nothing but slices.

// the library's name says it is unsafe (it is the C code translated as it was): the short name says what it is for here
#[allow(clippy::unsafe_removed_from_name)]
use unsafe_libopus as ffi;

/// Samples per second.
pub const RATE: u32 = 48_000;
/// Interleaved channels.
pub const CHANNELS: usize = 2;
/// Frames in the 20 ms the encoder is given at a time.
pub const FRAME: usize = 960;
/// The longest packet Opus writes for a frame (1275 bytes is the largest a frame can be).
const MAX_PACKET: usize = 1500;
/// The most frames a packet can hold: 120 ms.
const MAX_DECODED: usize = 5760;
/// What the sound is sent at, in bits per second: music-grade stereo.
const BITRATE: i32 = 96_000;

#[derive(Debug, thiserror::Error)]
#[error("opus: error {0}")]
pub struct Error(pub i32);

/// Turns packets into interleaved stereo samples.
pub struct Decoder {
    raw: *mut ffi::OpusDecoder,
    pcm: Vec<f32>,
}

// The decoder is only ever used by one thread at a time (it is moved into the thread of its session).
unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new() -> Result<Decoder, Error> {
        let mut error = 0;
        // SAFETY: plain call with a valid pointer for the error code; a null result is checked below
        let raw = unsafe { ffi::opus_decoder_create(RATE as i32, CHANNELS as i32, &mut error) };
        if raw.is_null() || error != 0 {
            return Err(Error(error));
        }
        Ok(Decoder {
            raw,
            pcm: vec![0.0; MAX_DECODED * CHANNELS],
        })
    }

    /// The samples (interleaved, stereo, 48 kHz) of one packet.
    pub fn decode(&mut self, packet: &[u8]) -> Result<&[f32], Error> {
        // SAFETY: `raw` is a live decoder; the packet and the output buffer are valid for the lengths given
        let frames = unsafe {
            ffi::opus_decode_float(
                self.raw,
                packet.as_ptr(),
                packet.len() as i32,
                self.pcm.as_mut_ptr(),
                MAX_DECODED as i32,
                0,
            )
        };
        if frames < 0 {
            return Err(Error(frames));
        }
        Ok(&self.pcm[..frames as usize * CHANNELS])
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: made by `opus_decoder_create`, destroyed once
        unsafe { ffi::opus_decoder_destroy(self.raw) }
    }
}

/// Turns 20 ms of interleaved stereo samples into a packet.
pub struct Encoder {
    raw: *mut ffi::OpusEncoder,
    packet: Vec<u8>,
}

// As for the decoder: one thread at a time.
unsafe impl Send for Encoder {}

impl Encoder {
    pub fn new() -> Result<Encoder, Error> {
        let mut error = 0;
        // SAFETY: plain call with a valid pointer for the error code; a null result is checked below
        let raw = unsafe {
            ffi::opus_encoder_create(
                RATE as i32,
                CHANNELS as i32,
                ffi::OPUS_APPLICATION_AUDIO,
                &mut error,
            )
        };
        if raw.is_null() || error != 0 {
            return Err(Error(error));
        }
        // SAFETY: `raw` is a live encoder and the request takes one integer
        let set = unsafe { ffi::opus_encoder_ctl!(raw, ffi::OPUS_SET_BITRATE_REQUEST, BITRATE) };
        if set != 0 {
            // SAFETY: made just above, not used again
            unsafe { ffi::opus_encoder_destroy(raw) };
            return Err(Error(set));
        }
        Ok(Encoder {
            raw,
            packet: vec![0; MAX_PACKET],
        })
    }

    /// The packet for exactly [`FRAME`] frames of interleaved stereo samples.
    pub fn encode(&mut self, samples: &[f32]) -> Result<&[u8], Error> {
        assert_eq!(
            samples.len(),
            FRAME * CHANNELS,
            "one frame of 20 ms at a time"
        );
        // SAFETY: `raw` is a live encoder; the samples and the packet buffer are valid for the lengths given
        let bytes = unsafe {
            ffi::opus_encode_float(
                self.raw,
                samples.as_ptr(),
                FRAME as i32,
                self.packet.as_mut_ptr(),
                MAX_PACKET as i32,
            )
        };
        if bytes < 0 {
            return Err(Error(bytes));
        }
        Ok(&self.packet[..bytes as usize])
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: made by `opus_encoder_create`, destroyed once
        unsafe { ffi::opus_encoder_destroy(self.raw) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 20 ms frames of a 440 Hz tone, stereo.
    fn tone(frame: usize) -> Vec<f32> {
        (0..FRAME)
            .flat_map(|i| {
                let t = (frame * FRAME + i) as f32 / RATE as f32;
                let s = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
                [s, s]
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn a_tone_survives_the_codec_with_its_level_and_a_packet_is_small() {
        let mut encoder = Encoder::new().unwrap();
        let mut decoder = Decoder::new().unwrap();
        let mut out = vec![];
        let mut sizes = vec![];
        for frame in 0..25 {
            let packet = encoder.encode(&tone(frame)).unwrap().to_vec();
            sizes.push(packet.len());
            out.extend_from_slice(decoder.decode(&packet).unwrap());
        }
        assert_eq!(out.len(), 25 * FRAME * CHANNELS, "every frame comes back");
        // after the first frames (the codec's start), the level of the tone is the one that was sent (0.5 / sqrt 2)
        let level = rms(&out[10 * FRAME * CHANNELS..]);
        assert!((level - 0.3535).abs() < 0.03, "rms {level}");
        assert!(sizes.iter().all(|&n| n > 0 && n < 600), "{sizes:?}");
    }
}
