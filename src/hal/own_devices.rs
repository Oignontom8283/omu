//! The system's default device, when it is one of this program's own virtual devices.
//!
//! The system may make a virtual microphone (or speaker) the default one as soon as it appears. A block that records "the
//! default microphone" would then record what this program plays into it: a loop, and not what the person meant. The default of
//! a block is the default of the system *except* when that is one of our own devices: it is then the first other device.

use crate::{
    AppInfo, Backend, Capabilities, CaptureStream, DeviceInfo, Direction, HalError, HotkeyCallback,
    RenderStream, Running, VirtualDeviceProvider,
};
use omu_types::{AppTarget, DeviceRef, KeyCombo};
use std::sync::Arc;

/// Whether the device called `device` is one of `own` (the names given to the virtual devices): Windows names an emulated device
/// "Microphone (<name>)" or "Speakers (<name>)", so the name is matched whole or inside those parentheses (a short name such as
/// "Mic" must not match every device with "mic" in its name). Case does not matter.
pub fn is_own_device(device: &str, own: &[String]) -> bool {
    let device = device.trim().to_lowercase();
    own.iter().any(|name| {
        let name = name.trim().to_lowercase();
        !name.is_empty() && (device == name || device.ends_with(&format!("({name})")))
    })
}

/// A backend that never gives, as the default device, one of the virtual devices of `provider`: see the module's documentation.
pub struct OwnDevicesAside {
    inner: Arc<dyn Backend>,
    provider: Arc<dyn VirtualDeviceProvider>,
}

impl OwnDevicesAside {
    pub fn new(
        inner: Arc<dyn Backend>,
        provider: Arc<dyn VirtualDeviceProvider>,
    ) -> OwnDevicesAside {
        OwnDevicesAside { inner, provider }
    }

    fn own(&self, direction: Direction) -> Vec<String> {
        match direction {
            Direction::Capture => self.provider.microphone_names(),
            Direction::Render => self.provider.speaker_names(),
        }
    }

    /// The device to open in place of the system's default of `direction`, when that one is ours (none: the system's default is fine).
    fn instead_of_default(&self, direction: Direction) -> Option<DeviceInfo> {
        let own = self.own(direction);
        if own.is_empty() {
            return None;
        }
        let devices = self.inner.list_devices().ok()?;
        let of_direction = || devices.iter().filter(|d| d.direction == direction);
        let default = of_direction().find(|d| d.is_default)?;
        if !is_own_device(&default.name, &own) {
            return None;
        }
        of_direction()
            .find(|d| !is_own_device(&d.name, &own))
            .cloned()
    }
}

impl Backend for OwnDevicesAside {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn list_devices(&self) -> Result<Vec<DeviceInfo>, HalError> {
        let mut devices = self.inner.list_devices()?;
        for direction in [Direction::Capture, Direction::Render] {
            if let Some(other) = self.instead_of_default(direction) {
                for d in devices.iter_mut().filter(|d| d.direction == direction) {
                    d.is_default = d.id == other.id;
                }
            }
        }
        Ok(devices)
    }

    fn open_capture(&self, device: &DeviceRef) -> Result<Box<dyn CaptureStream>, HalError> {
        match (device, self.instead_of_default(Direction::Capture)) {
            (DeviceRef::Default, Some(other)) => self.inner.open_capture(&DeviceRef::Id(other.id)),
            _ => self.inner.open_capture(device),
        }
    }

    fn open_render(&self, device: &DeviceRef) -> Result<Box<dyn RenderStream>, HalError> {
        match (device, self.instead_of_default(Direction::Render)) {
            (DeviceRef::Default, Some(other)) => self.inner.open_render(&DeviceRef::Id(other.id)),
            _ => self.inner.open_render(device),
        }
    }

    fn list_apps(&self) -> Result<Vec<AppInfo>, HalError> {
        self.inner.list_apps()
    }

    fn open_hotkey(
        &self,
        combo: &KeyCombo,
        on_press: HotkeyCallback,
    ) -> Result<Box<dyn Running>, HalError> {
        self.inner.open_hotkey(combo, on_press)
    }

    fn open_hotkey_hold(
        &self,
        combo: &KeyCombo,
        on_press: HotkeyCallback,
        on_release: HotkeyCallback,
    ) -> Result<Box<dyn Running>, HalError> {
        self.inner.open_hotkey_hold(combo, on_press, on_release)
    }

    fn pause_hotkeys(&self, paused: bool) {
        self.inner.pause_hotkeys(paused);
    }

    fn open_app_capture(&self, target: &AppTarget) -> Result<Box<dyn CaptureStream>, HalError> {
        self.inner.open_app_capture(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A system with these devices (name, direction, is the default), that remembers what it was asked to open.
    struct System {
        devices: Vec<DeviceInfo>,
        opened: Mutex<Vec<DeviceRef>>,
    }

    impl Backend for System {
        fn name(&self) -> &str {
            "test"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        fn list_devices(&self) -> Result<Vec<DeviceInfo>, HalError> {
            Ok(self.devices.clone())
        }
        fn open_capture(&self, device: &DeviceRef) -> Result<Box<dyn CaptureStream>, HalError> {
            self.opened.lock().unwrap().push(device.clone());
            Err(HalError::Busy("test".into()))
        }
        fn open_render(&self, device: &DeviceRef) -> Result<Box<dyn RenderStream>, HalError> {
            self.opened.lock().unwrap().push(device.clone());
            Err(HalError::Busy("test".into()))
        }
    }

    struct Provider;

    impl VirtualDeviceProvider for Provider {
        fn name(&self) -> &str {
            "test"
        }
        fn available(&self) -> Result<(), HalError> {
            Ok(())
        }
        fn create_microphone(&self, _: &str) -> Result<Box<dyn RenderStream>, HalError> {
            Err(HalError::Busy("test".into()))
        }
        fn microphone_names(&self) -> Vec<String> {
            vec!["Sortie".into()]
        }
        fn speaker_names(&self) -> Vec<String> {
            vec!["Wow".into()]
        }
    }

    fn device(name: &str, direction: Direction, default: bool) -> DeviceInfo {
        DeviceInfo {
            id: format!("id-{name}"),
            name: name.into(),
            direction,
            is_default: default,
            format: None,
        }
    }

    fn aside(devices: Vec<DeviceInfo>) -> (Arc<System>, OwnDevicesAside) {
        let system = Arc::new(System {
            devices,
            opened: Mutex::new(vec![]),
        });
        (
            system.clone(),
            OwnDevicesAside::new(system, Arc::new(Provider)),
        )
    }

    fn defaults(backend: &OwnDevicesAside) -> Vec<String> {
        backend
            .list_devices()
            .unwrap()
            .into_iter()
            .filter(|d| d.is_default)
            .map(|d| d.name)
            .collect()
    }

    #[test]
    fn a_device_is_ours_by_its_whole_name_or_inside_the_parentheses_of_the_one_windows_gives_it() {
        let own = vec!["Sortie".to_string(), "".to_string()];
        assert!(is_own_device("Microphone (SORTIE)", &own));
        assert!(is_own_device("sortie", &own));
        assert!(!is_own_device("Microphone (Realtek)", &own));
        assert!(!is_own_device("Sortie de la carte son", &own));
        assert!(!is_own_device("Microphone ()", &own));
    }

    #[test]
    fn the_default_microphone_is_the_first_other_one_when_the_system_made_ours_the_default() {
        let (system, backend) = aside(vec![
            device("Microphone (Sortie)", Direction::Capture, true),
            device("Microphone (Realtek)", Direction::Capture, false),
            device("Speakers (Realtek)", Direction::Render, true),
        ]);
        assert_eq!(
            defaults(&backend),
            vec!["Microphone (Realtek)", "Speakers (Realtek)"]
        );
        let _ = backend.open_capture(&DeviceRef::Default);
        assert_eq!(
            *system.opened.lock().unwrap(),
            vec![DeviceRef::Id("id-Microphone (Realtek)".into())]
        );
    }

    #[test]
    fn the_default_speakers_are_the_first_other_ones_when_the_system_made_our_speaker_the_default()
    {
        let (system, backend) = aside(vec![
            device("Speakers (Wow)", Direction::Render, true),
            device("Speakers (Realtek)", Direction::Render, false),
        ]);
        assert_eq!(defaults(&backend), vec!["Speakers (Realtek)"]);
        let _ = backend.open_render(&DeviceRef::Default);
        assert_eq!(
            *system.opened.lock().unwrap(),
            vec![DeviceRef::Id("id-Speakers (Realtek)".into())]
        );
    }

    #[test]
    fn the_default_of_the_system_is_left_alone_when_it_is_not_ours() {
        let (system, backend) = aside(vec![
            device("Microphone (Realtek)", Direction::Capture, true),
            device("Microphone (Sortie)", Direction::Capture, false),
        ]);
        assert_eq!(defaults(&backend), vec!["Microphone (Realtek)"]);
        let _ = backend.open_capture(&DeviceRef::Default);
        assert_eq!(*system.opened.lock().unwrap(), vec![DeviceRef::Default]);
    }

    #[test]
    fn when_every_device_is_ours_the_default_stays_the_systems() {
        let (system, backend) = aside(vec![device(
            "Microphone (Sortie)",
            Direction::Capture,
            true,
        )]);
        assert_eq!(defaults(&backend), vec!["Microphone (Sortie)"]);
        let _ = backend.open_capture(&DeviceRef::Default);
        assert_eq!(*system.opened.lock().unwrap(), vec![DeviceRef::Default]);
    }

    #[test]
    fn a_device_chosen_by_hand_is_opened_as_it_is() {
        let (system, backend) = aside(vec![
            device("Microphone (Sortie)", Direction::Capture, true),
            device("Microphone (Realtek)", Direction::Capture, false),
        ]);
        let chosen = DeviceRef::Id("id-Microphone (Sortie)".into());
        let _ = backend.open_capture(&chosen);
        assert_eq!(*system.opened.lock().unwrap(), vec![chosen]);
    }
}
