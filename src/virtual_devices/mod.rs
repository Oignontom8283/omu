//! Virtual audio devices: microphones that other programs can pick, fed by the audio of a graph.
//!
//! * [`usb`]: a USB/IP server and a USB Audio Class 2 device (vendored, MIT);
//! * [`tool`]: the client that attaches such a device to Windows (`usbip-win2`, installed by the user);
//! * [`mic`]: the [`UsbMicProvider`], the [`VirtualDeviceProvider`](crate::hal::VirtualDeviceProvider) that ties them.
//!
//! No driver of ours is involved: Windows binds its in-box USB audio driver to the emulated device.

pub mod mic;
pub mod tool;
pub mod usb;

pub use mic::UsbMicProvider;
pub use tool::{UsbipExe, UsbipTool};

use crate::hal::VirtualDeviceProvider;
use std::sync::Arc;

/// The virtual-device provider of this platform.
pub fn platform_provider() -> Arc<dyn VirtualDeviceProvider> {
    Arc::new(UsbMicProvider::new(Arc::new(UsbipExe)))
}
