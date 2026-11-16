//! Virtual speakers made of emulated USB audio devices.
//!
//! The mirror image of a virtual microphone: a USB Audio Class 2 device with a stereo *playback* stream (48 kHz,
//! 24 bits). Windows shows it as an output device, "Speakers (<name>)", that any program can be pointed at; what the
//! programs play into it reaches us through [`Uac2Streams::playback`] and becomes the sound of the graph's
//! "virtual speaker" block. Nothing is audible: the device has no physical output.
//!
//! It covers what capturing one application cannot: programs that are sent to a chosen output device, several programs
//! gathered on one device, or a program that cannot be captured by itself.

use super::mic::{attach, name_hash, Attach, Shared, FULL_SCALE_24};
use super::tool::UsbipTool;
use super::usb::uac2::{Uac2Config, Uac2Device, Uac2Streams};
use omu_hal::{CaptureCallback, CaptureStream, HalError, Running, StreamHealth};
use omu_types::AudioFormat;
use std::sync::{Arc, Mutex};

/// Where the audio of a speaker goes: the engine's source callback, given what the USB host played.
struct SpeakerFeed {
    callback: Mutex<CaptureCallback>,
    scratch: Mutex<Vec<f32>>,
}

impl Uac2Streams for SpeakerFeed {
    fn playback(&self, _frame: u64, samples: &[i32]) {
        let mut scratch = super::mic::lock(&self.scratch);
        scratch.clear();
        // 24-bit samples, left-justified in 32-bit slots
        scratch.extend(samples.iter().map(|&s| (s >> 8) as f32 / FULL_SCALE_24));
        (super::mic::lock(&self.callback))(&scratch);
    }

    // This device has no capture stream.
    fn capture(&self, _frame: u64, samples: &mut [i32]) {
        samples.fill(0);
    }
}

/// The name Windows shows is derived from the device's product string; the serial number must differ from a microphone
/// of the same name, or Windows would treat the two as one device.
pub(super) fn serial_of(name: &str) -> String {
    format!("{:016x}", name_hash(&format!("speaker:{}", name.trim())))
}

pub(super) struct SpeakerStream {
    pub name: String,
    pub busid: String,
    pub config: Uac2Config,
    pub shared: Arc<Shared>,
    pub tool: Arc<dyn UsbipTool>,
    pub health: Arc<StreamHealth>,
}

impl CaptureStream for SpeakerStream {
    fn format(&self) -> AudioFormat {
        AudioFormat::ENGINE
    }

    fn health(&self) -> Arc<StreamHealth> {
        self.health.clone()
    }

    fn start(self: Box<Self>, callback: CaptureCallback) -> Result<Box<dyn Running>, HalError> {
        let feed = Arc::new(SpeakerFeed {
            callback: Mutex::new(callback),
            scratch: Mutex::new(Vec::with_capacity(64)),
        });
        let device = Uac2Device::new(self.config.clone(), feed).map_err(HalError::Backend)?;
        attach(
            &Attach {
                what: "virtual speaker",
                name: &self.name,
                busid: &self.busid,
                shared: &self.shared,
                tool: &self.tool,
                health: &self.health,
            },
            Arc::new(device),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_windows_plays_reaches_the_engine_as_floats() {
        let got = Arc::new(Mutex::new(vec![]));
        let g = got.clone();
        let feed = SpeakerFeed {
            callback: Mutex::new(Box::new(move |b: &[f32]| {
                g.lock().unwrap().extend_from_slice(b)
            })),
            scratch: Mutex::new(vec![]),
        };
        // full scale, half scale negative, silence, a tiny value: 24 bits left-justified in 32
        feed.playback(0, &[8_388_607 << 8, -4_194_304 << 8, 0, 1 << 8]);
        let got = got.lock().unwrap();
        assert!((got[0] - 1.0).abs() < 1e-6);
        assert!((got[1] + 0.5).abs() < 1e-6);
        assert_eq!(got[2], 0.0);
        assert!(got[3] > 0.0 && got[3] < 1e-6);
    }

    #[test]
    fn a_speaker_and_a_microphone_of_the_same_name_are_different_devices() {
        assert_ne!(serial_of("Omu"), format!("{:016x}", name_hash("Omu")));
        assert_eq!(
            serial_of(" Omu "),
            serial_of("omu"),
            "case and spaces do not matter, like for microphones"
        );
    }
}
