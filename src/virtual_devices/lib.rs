//! Virtual audio devices: microphones that other programs can pick, fed by the audio of a graph, and speakers that
//! other programs can play into, whose sound the graph receives.
//!
//! * [`usb`]: a USB/IP server and a USB Audio Class 2 device (vendored, MIT);
//! * [`tool`]: the client that attaches such a device to Windows (`usbip-win2`, installed by the user);
//! * [`mic`]: the [`UsbMicProvider`], the [`VirtualDeviceProvider`](omu_hal::VirtualDeviceProvider) that ties them;
//! * [`speaker`]: the speaker side of that provider.
//!
//! No driver of ours is involved: Windows binds its in-box USB audio driver to the emulated device.

pub mod clean;
pub mod mic;
pub mod speaker;
pub mod stale;
pub mod tool;
pub mod usb;

pub use mic::UsbMicProvider;
pub use tool::{UsbipExe, UsbipTool};

use omu_hal::VirtualDeviceProvider;
use std::sync::Arc;

/// The virtual-device provider of this platform.
pub fn platform_provider() -> Arc<dyn VirtualDeviceProvider> {
    Arc::new(UsbMicProvider::new(Arc::new(UsbipExe)))
}
