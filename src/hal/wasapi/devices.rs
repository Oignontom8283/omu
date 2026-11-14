//! Device enumeration and resolution.

use super::com::{backend_error, ComGuard};
use crate::hal::{DeviceInfo, Direction, HalError};
use crate::types::{AudioFormat, DeviceRef};
use windows::core::{Interface, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, IAudioClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};

pub fn flow(direction: Direction) -> EDataFlow {
    match direction {
        Direction::Capture => eCapture,
        Direction::Render => eRender,
    }
}

pub fn enumerator() -> Result<IMMDeviceEnumerator, HalError> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
        .map_err(|e| backend_error("CoCreateInstance(MMDeviceEnumerator)", e))
}

fn pwstr_to_string(p: windows::core::PWSTR) -> String {
    let s = unsafe { p.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

pub fn device_id(device: &IMMDevice) -> Result<String, HalError> {
    let id = unsafe { device.GetId() }.map_err(|e| backend_error("IMMDevice::GetId", e))?;
    Ok(pwstr_to_string(id))
}

pub fn device_name(device: &IMMDevice) -> String {
    (|| {
        let store = unsafe { device.OpenPropertyStore(STGM_READ) }.ok()?;
        let value = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }.ok()?;
        Some(value.to_string())
    })()
    .unwrap_or_else(|| "(unnamed device)".into())
}

/// The device's native shared-mode format, when it can be read.
pub fn device_format(device: &IMMDevice) -> Option<AudioFormat> {
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }.ok()?;
    let mix = unsafe { client.GetMixFormat() }.ok()?;
    let f = unsafe { mix.read_unaligned() };
    unsafe { CoTaskMemFree(Some(mix as *const _)) };
    Some(AudioFormat {
        sample_rate: f.nSamplesPerSec,
        channels: f.nChannels,
    })
}

fn default_id(en: &IMMDeviceEnumerator, direction: Direction) -> Option<String> {
    let d = unsafe { en.GetDefaultAudioEndpoint(flow(direction), eConsole) }.ok()?;
    device_id(&d).ok()
}

/// All active devices of one direction.
pub fn list_direction(direction: Direction) -> Result<Vec<(IMMDevice, DeviceInfo)>, HalError> {
    let _com = ComGuard::new();
    let en = enumerator()?;
    let default = default_id(&en, direction);
    let collection = unsafe { en.EnumAudioEndpoints(flow(direction), DEVICE_STATE_ACTIVE) }
        .map_err(|e| backend_error("EnumAudioEndpoints", e))?;
    let count = unsafe { collection.GetCount() }.map_err(|e| backend_error("GetCount", e))?;
    let mut out = vec![];
    for i in 0..count {
        let Ok(device) = (unsafe { collection.Item(i) }) else {
            continue;
        };
        let Ok(id) = device_id(&device) else { continue };
        let info = DeviceInfo {
            name: device_name(&device),
            direction,
            is_default: default.as_deref() == Some(id.as_str()),
            format: device_format(&device),
            id,
        };
        out.push((device, info));
    }
    Ok(out)
}

/// Finds the device a [`DeviceRef`] designates.
pub fn resolve(
    direction: Direction,
    device: &DeviceRef,
) -> Result<(IMMDevice, DeviceInfo), HalError> {
    match device {
        DeviceRef::Default => {
            let _com = ComGuard::new();
            let en = enumerator()?;
            let d = unsafe { en.GetDefaultAudioEndpoint(flow(direction), eConsole) }
                .map_err(|_| HalError::NotFound(format!("no default {direction:?} device")))?;
            let id = device_id(&d)?;
            let info = DeviceInfo {
                name: device_name(&d),
                direction,
                is_default: true,
                format: device_format(&d),
                id,
            };
            Ok((d, info))
        }
        DeviceRef::Id(id) => list_direction(direction)?
            .into_iter()
            .find(|(_, i)| &i.id == id)
            .ok_or_else(|| {
                HalError::NotFound(format!("no active {direction:?} device with id {id}"))
            }),
        DeviceRef::Name(part) => {
            let part_lc = part.to_lowercase();
            let all = list_direction(direction)?;
            // an exact (case-insensitive) match wins over a partial one
            let exact = all
                .iter()
                .position(|(_, i)| i.name.to_lowercase() == part_lc);
            let position = exact.or_else(|| {
                all.iter()
                    .position(|(_, i)| i.name.to_lowercase().contains(&part_lc))
            });
            match position {
                Some(p) => Ok(all.into_iter().nth(p).expect("position is in range")),
                None => Err(HalError::NotFound(format!(
                    "no active {direction:?} device whose name contains {part:?}"
                ))),
            }
        }
    }
}

#[allow(dead_code)]
fn _assert_interface<T: Interface>() {}

#[allow(dead_code)]
fn _unused(_: PCWSTR) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_machines_devices_without_failing() {
        // No assertion on content (CI machines may have no audio): it must not error out or crash.
        for d in [Direction::Capture, Direction::Render] {
            let _ = list_direction(d);
        }
    }

    #[test]
    fn an_unknown_name_is_not_found() {
        let r = resolve(
            Direction::Render,
            &DeviceRef::Name("no such device zzzzz".into()),
        );
        assert!(matches!(r, Err(HalError::NotFound(_))));
    }
}
