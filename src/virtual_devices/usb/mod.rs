//! A USB/IP server and a USB Audio Class 2 device emulation.
//!
//! Windows can attach a USB device that lives on a TCP server (the signed `usbip-win2` client driver does it) and
//! then binds its own in-box USB audio driver to it: the device shows up as a normal microphone or speaker, and no
//! audio driver of ours is involved. This module is the server side and the device: the audio flows through
//! [`uac2::Uac2Streams`].
//!
//! Vendored from the MIT-licensed `usbip-audio` project (commit 58032ab, see `usbip-audio.MIT.txt` and `README.md` in this folder),
//! with these changes: the Windows timer resolution calls use the `windows` crate; the server reports whether a
//! device is currently attached ([`Server::is_attached`]); the UAC1 speaker and the `usbip.exe` helper were left out
//! (the attach tool lives in [`super::tool`]); the integration tests became [`protocol_tests`].
//! The code keeps its original style, hence the lint allowances below.
#![allow(clippy::all)]
#![allow(dead_code)]

mod clock;
mod device;
mod server;
pub mod uac2;

#[cfg(test)]
mod protocol_tests;

pub use clock::{Clock, MonotonicClock};
pub use device::{IsoEndpoint, IsoTiming, Notifier, Setup, Speed, UsbDevice, EPIPE};
pub use server::Server;
