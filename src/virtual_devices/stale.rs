//! The devices the system remembers although they are gone.
//!
//! Windows keeps a record of every device that was ever plugged in, and our virtual devices are plugged in under a
//! serial number made from their name: every name a microphone or a speaker has had leaves a record behind (the USB
//! device, its audio function and the audio endpoints of the sound settings), shown as "not connected" for ever. They
//! hurt nothing, but they pile up. This finds the ones that belong to a name the graph no longer uses (read only, no
//! rights needed) and forgets them (needs administrator rights).
//!
//! The decision of what is stale is plain logic over what the system reports ([`classify`]) and is tested without a
//! system; only reading and removing touch it, and only on Windows. A device that is plugged in, or whose name is in
//! the graph, is never touched.

use super::mic::name_hash;
use super::speaker;
use omu_types::{KeptDevices, StaleDevice};
use std::collections::BTreeMap;

/// One device as the system remembers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remembered {
    pub instance_id: String,
    /// What the system calls it (the audio function of ours is called by its product name, the sound settings' endpoint
    /// by "Microphone (name)").
    pub name: String,
    /// Plugged in now.
    pub present: bool,
    /// The instance id of the device it hangs under.
    pub parent: Option<String>,
}

/// Why forgetting devices failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The system refused: administrator rights are needed.
    NeedsAdministrator,
    Other(String),
}

fn mic_serial(name: &str) -> String {
    format!("{:016x}", name_hash(name))
}

/// What is stale among `all`: the records of ours that are not plugged in and whose name is not in `kept`, grouped by
/// the name the user gave the device. `vendor` and `product` are the USB ids our virtual devices carry.
pub fn classify(
    vendor: u16,
    product: u16,
    all: &[Remembered],
    kept: &KeptDevices,
) -> Vec<(StaleDevice, Vec<String>)> {
    let root_prefix = format!("USB\\VID_{vendor:04X}&PID_{product:04X}\\");
    let function_prefix = format!("USB\\VID_{vendor:04X}&PID_{product:04X}&MI_");
    let endpoint_prefix = "SWD\\MMDEVAPI\\";
    let upper = |s: &str| s.to_uppercase();

    let kept_serials: Vec<String> = kept
        .microphones
        .iter()
        .map(|n| mic_serial(n))
        .chain(kept.speakers.iter().map(|n| speaker::serial_of(n)))
        .map(|s| s.to_lowercase())
        .collect();
    let kept_names: Vec<String> = kept
        .microphones
        .iter()
        .chain(&kept.speakers)
        .map(|n| n.trim().to_lowercase())
        .collect();

    // name (lower case) -> (name as the user wrote it, the records to forget)
    type Groups = BTreeMap<String, (String, Vec<String>)>;
    fn add(groups: &mut Groups, key: &str, name: &str, id: &str) {
        let entry = groups
            .entry(key.to_lowercase())
            .or_insert_with(|| (name.to_owned(), vec![]));
        if !entry.1.iter().any(|x| x.eq_ignore_ascii_case(id)) {
            entry.1.push(id.to_owned());
        }
    }
    let mut groups = Groups::new();

    // a USB device of ours that is gone, with another serial number than any name of the graph
    let mut handled_functions: Vec<&str> = vec![];
    for root in all.iter().filter(|d| {
        !d.present
            && upper(&d.instance_id).starts_with(&root_prefix)
            && !kept_serials.contains(
                &d.instance_id
                    .rsplit('\\')
                    .next()
                    .unwrap_or_default()
                    .to_lowercase(),
            )
    }) {
        let children: Vec<&Remembered> = all
            .iter()
            .filter(|f| {
                upper(&f.instance_id).starts_with(&function_prefix)
                    && f.parent
                        .as_deref()
                        .is_some_and(|p| p.eq_ignore_ascii_case(&root.instance_id))
            })
            .collect();
        let named = children.iter().find(|f| !f.name.trim().is_empty());
        let serial = root.instance_id.rsplit('\\').next().unwrap_or_default();
        let (key, name) = named.map_or((serial, serial), |f| (f.name.trim(), f.name.trim()));
        add(&mut groups, key, name, &root.instance_id);
        for f in children {
            add(&mut groups, key, name, &f.instance_id);
            handled_functions.push(&f.instance_id);
        }
    }

    // an audio function left without its USB device, or whose name is not in the graph
    for f in all.iter().filter(|f| {
        !f.present
            && upper(&f.instance_id).starts_with(&function_prefix)
            && !f.name.trim().is_empty()
            && !handled_functions.contains(&f.instance_id.as_str())
            && !kept_names.contains(&f.name.trim().to_lowercase())
    }) {
        add(&mut groups, f.name.trim(), f.name.trim(), &f.instance_id);
    }

    // The sound settings' entries of a stale device: the ones that hang under one of its records, and (when the system
    // does not say what an entry hangs under) the ones named after it: "Microphone (name)", "Speakers (name)".
    let known: Vec<String> = all.iter().map(|d| d.instance_id.to_uppercase()).collect();
    let stale_keys: Vec<String> = groups.keys().cloned().collect();
    for e in all.iter().filter(|e| {
        !e.present && upper(&e.instance_id).starts_with(endpoint_prefix) && !e.name.is_empty()
    }) {
        let by_parent = e.parent.as_deref().and_then(|p| {
            stale_keys
                .iter()
                .find(|k| groups[*k].1.iter().any(|id| id.eq_ignore_ascii_case(p)))
        });
        let orphan = e
            .parent
            .as_deref()
            .is_none_or(|p| !known.contains(&p.to_uppercase()));
        let label = e.name.trim().to_lowercase();
        let by_name = orphan
            .then(|| {
                stale_keys
                    .iter()
                    .find(|k| label.ends_with(&format!("({k})")))
            })
            .flatten();
        if let Some(key) = by_parent.or(by_name) {
            let name = groups[key].0.clone();
            add(&mut groups, key, &name, &e.instance_id);
        }
    }

    groups
        .into_values()
        .map(|(name, ids)| (StaleDevice { name }, ids))
        .collect()
}

/// Reads what the system remembers about devices of ours (and the sound settings' endpoints).
#[cfg(windows)]
pub fn remembered(vendor: u16, product: u16) -> Result<Vec<Remembered>, Failure> {
    windows_impl::remembered(vendor, product)
}

/// Forgets the records `ids` (administrator rights needed); returns how many were removed.
#[cfg(windows)]
pub fn forget(ids: &[String]) -> Result<usize, Failure> {
    windows_impl::forget(ids)
}

#[cfg(not(windows))]
pub fn remembered(_vendor: u16, _product: u16) -> Result<Vec<Remembered>, Failure> {
    Err(Failure::Other(
        "this system does not remember devices the way Windows does".into(),
    ))
}

#[cfg(not(windows))]
pub fn forget(_ids: &[String]) -> Result<usize, Failure> {
    Err(Failure::Other(
        "this system does not remember devices the way Windows does".into(),
    ))
}

#[cfg(windows)]
mod windows_impl {
    use super::{Failure, Remembered};
    use std::collections::HashSet;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiCallClassInstaller, SetupDiCreateDeviceInfoList, SetupDiDestroyDeviceInfoList,
        SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW,
        SetupDiGetDevicePropertyW, SetupDiGetDeviceRegistryPropertyW, SetupDiOpenDeviceInfoW,
        DIF_REMOVE, DIGCF_ALLCLASSES, DIGCF_PRESENT, HDEVINFO, SETUP_DI_REGISTRY_PROPERTY,
        SPDRP_DEVICEDESC, SPDRP_FRIENDLYNAME, SP_DEVINFO_DATA,
    };
    use windows::Win32::Devices::Properties::{DEVPKEY_Device_Parent, DEVPROPTYPE};
    use windows::Win32::Foundation::ERROR_ACCESS_DENIED;

    /// An open list of devices, closed when dropped.
    struct DeviceList(HDEVINFO);

    impl Drop for DeviceList {
        fn drop(&mut self) {
            // SAFETY: the handle comes from SetupDiGetClassDevsW / SetupDiCreateDeviceInfoList and is closed once.
            unsafe {
                let _ = SetupDiDestroyDeviceInfoList(self.0);
            }
        }
    }

    /// The 16-bit characters of a buffer of bytes (what the system writes for a text property).
    fn utf16(bytes: &[u8]) -> Vec<u16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect()
    }

    fn wide(buffer: &[u16]) -> String {
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end])
    }

    fn all_devices(present_only: bool) -> Result<DeviceList, Failure> {
        let flags = if present_only {
            DIGCF_ALLCLASSES | DIGCF_PRESENT
        } else {
            DIGCF_ALLCLASSES
        };
        // SAFETY: no class, no enumerator, no window: lists every device the system knows.
        unsafe { SetupDiGetClassDevsW(None, PCWSTR::null(), None, flags) }
            .map(DeviceList)
            .map_err(|e| Failure::Other(format!("cannot list the devices: {e}")))
    }

    /// Calls `each` with every device of `list` and its instance id.
    fn each_device(list: &DeviceList, mut each: impl FnMut(&SP_DEVINFO_DATA, String)) {
        let mut index = 0;
        loop {
            let mut data = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            // SAFETY: `data` is a valid SP_DEVINFO_DATA with its size set.
            if unsafe { SetupDiEnumDeviceInfo(list.0, index, &mut data) }.is_err() {
                break;
            }
            index += 1;
            let mut id = [0u16; 400];
            // SAFETY: `id` is a writable buffer of 400 characters.
            if unsafe { SetupDiGetDeviceInstanceIdW(list.0, &data, Some(&mut id), None) }.is_ok() {
                each(&data, wide(&id));
            }
        }
    }

    fn text_property(
        list: &DeviceList,
        data: &SP_DEVINFO_DATA,
        property: SETUP_DI_REGISTRY_PROPERTY,
    ) -> Option<String> {
        let mut buffer = [0u8; 1024];
        // SAFETY: `buffer` is writable; its size is passed.
        unsafe {
            SetupDiGetDeviceRegistryPropertyW(list.0, data, property, None, Some(&mut buffer), None)
        }
        .ok()?;
        let chars = utf16(&buffer);
        let name = wide(&chars);
        (!name.is_empty()).then_some(name)
    }

    fn parent_of(list: &DeviceList, data: &SP_DEVINFO_DATA) -> Option<String> {
        let mut kind = DEVPROPTYPE::default();
        let mut buffer = [0u8; 800];
        // SAFETY: `buffer` is writable; its size is passed.
        unsafe {
            SetupDiGetDevicePropertyW(
                list.0,
                data,
                &DEVPKEY_Device_Parent,
                &mut kind,
                Some(&mut buffer),
                None,
                0,
            )
        }
        .ok()?;
        let chars = utf16(&buffer);
        let id = wide(&chars);
        (!id.is_empty()).then_some(id)
    }

    pub fn remembered(vendor: u16, product: u16) -> Result<Vec<Remembered>, Failure> {
        let ours = format!("USB\\VID_{vendor:04X}&PID_{product:04X}");
        let wanted = |id: &str| {
            let id = id.to_uppercase();
            id.starts_with(&ours) || id.starts_with("SWD\\MMDEVAPI\\")
        };
        // which of them are plugged in now
        let mut present = HashSet::new();
        each_device(&all_devices(true)?, |_, id| {
            if wanted(&id) {
                present.insert(id.to_uppercase());
            }
        });
        let list = all_devices(false)?;
        let mut out = vec![];
        each_device(&list, |data, id| {
            if !wanted(&id) {
                return;
            }
            let name = text_property(&list, data, SPDRP_FRIENDLYNAME)
                .or_else(|| text_property(&list, data, SPDRP_DEVICEDESC))
                .unwrap_or_default();
            out.push(Remembered {
                present: present.contains(&id.to_uppercase()),
                parent: parent_of(&list, data),
                instance_id: id,
                name,
            });
        });
        Ok(out)
    }

    pub fn forget(ids: &[String]) -> Result<usize, Failure> {
        // SAFETY: an empty list of devices, filled by SetupDiOpenDeviceInfoW below.
        let list = unsafe { SetupDiCreateDeviceInfoList(None, None) }
            .map(DeviceList)
            .map_err(|e| Failure::Other(format!("cannot prepare the removal: {e}")))?;
        let mut removed = 0;
        for id in ids {
            let mut data = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            let instance = HSTRING::from(id.as_str());
            // SAFETY: `instance` lives through the call, `data` is a valid SP_DEVINFO_DATA with its size set.
            let opened = unsafe {
                SetupDiOpenDeviceInfoW(list.0, PCWSTR(instance.as_ptr()), None, 0, Some(&mut data))
            };
            if opened.is_err() {
                continue; // already gone
            }
            // SAFETY: `data` was just filled by SetupDiOpenDeviceInfoW for this list.
            match unsafe { SetupDiCallClassInstaller(DIF_REMOVE, list.0, Some(&data)) } {
                Ok(()) => removed += 1,
                Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => {
                    return Err(Failure::NeedsAdministrator)
                }
                Err(e) => tracing::warn!(device = %id, "cannot forget a device: {e}"),
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: u16 = 0x1209;
    const P: u16 = 0x0001;

    fn rec(id: &str, name: &str, present: bool, parent: Option<&str>) -> Remembered {
        Remembered {
            instance_id: id.into(),
            name: name.into(),
            present,
            parent: parent.map(str::to_owned),
        }
    }

    /// What Windows had on a real machine after a few renames, with the graph using "SORTIE" and speaker "WOW".
    fn machine() -> Vec<Remembered> {
        let sortie = mic_serial("SORTIE").to_uppercase();
        let wow = speaker::serial_of("WOW").to_uppercase();
        let root = |serial: &str| format!("USB\\VID_1209&PID_0001\\{serial}");
        vec![
            // in the graph
            rec(&root(&sortie), "USB composite device", false, None),
            rec(
                "USB\\VID_1209&PID_0001&MI_00\\3&AAA&0&0000",
                "SORTIE",
                false,
                Some(&root(&sortie)),
            ),
            rec(
                "SWD\\MMDEVAPI\\{0.0.1.00000000}.{SORTIE-EP}",
                "Microphone (SORTIE)",
                false,
                None,
            ),
            rec(&root(&wow), "USB composite device", false, None),
            rec(
                "USB\\VID_1209&PID_0001&MI_00\\3&BBB&0&0000",
                "WOW",
                false,
                Some(&root(&wow)),
            ),
            rec(
                "SWD\\MMDEVAPI\\{0.0.0.00000000}.{WOW-EP}",
                "Speakers (WOW)",
                false,
                None,
            ),
            // an old name: root, audio function and one endpoint, all gone
            rec(
                &root("1111111111111111"),
                "USB composite device",
                false,
                None,
            ),
            rec(
                "USB\\VID_1209&PID_0001&MI_00\\3&CCC&0&0000",
                "Old Mic",
                false,
                Some(&root("1111111111111111")),
            ),
            rec(
                "SWD\\MMDEVAPI\\{0.0.1.00000000}.{OLD-EP}",
                "Microphone (Old Mic)",
                false,
                None,
            ),
            // an audio function left alone after its USB device was removed
            rec(
                "USB\\VID_1209&PID_0001&MI_00\\3&DDD&0&0000",
                "Orphan",
                false,
                Some("USB\\VID_1209&PID_0001\\GONE"),
            ),
            // not ours, and one of ours that is plugged in: never touched
            rec("USB\\VID_046D&PID_C52B\\XYZ", "Mouse", false, None),
            rec(
                "SWD\\MMDEVAPI\\{0.0.1.00000000}.{REAL}",
                "Microphone (Real Mic)",
                false,
                None,
            ),
            rec(
                &root("2222222222222222"),
                "USB composite device",
                true,
                None,
            ),
            rec(
                "USB\\VID_1209&PID_0001&MI_00\\3&EEE&0&0000",
                "Plugged",
                true,
                Some(&root("2222222222222222")),
            ),
            rec(
                "SWD\\MMDEVAPI\\{0.0.1.00000000}.{PLUGGED-EP}",
                "Microphone (Plugged)",
                true,
                None,
            ),
        ]
    }

    fn kept() -> KeptDevices {
        KeptDevices {
            microphones: vec!["SORTIE".into()],
            speakers: vec!["WOW".into()],
        }
    }

    #[test]
    fn only_gone_devices_with_a_name_that_the_graph_does_not_use_are_stale() {
        let stale = classify(V, P, &machine(), &kept());
        let names: Vec<&str> = stale.iter().map(|(s, _)| s.name.as_str()).collect();
        assert_eq!(names, ["Old Mic", "Orphan"], "{stale:?}");
        let old = &stale[0].1;
        assert_eq!(
            old.len(),
            3,
            "its USB device, its audio function and its endpoint: {old:?}"
        );
        assert!(old.iter().any(|i| i.contains("1111111111111111")));
        assert!(old.iter().any(|i| i.contains("OLD-EP")));
        assert_eq!(stale[1].1, ["USB\\VID_1209&PID_0001&MI_00\\3&DDD&0&0000"]);
    }

    #[test]
    fn what_is_in_the_graph_is_plugged_in_or_not_ours_is_never_touched() {
        let all: Vec<String> = classify(V, P, &machine(), &kept())
            .into_iter()
            .flat_map(|(_, ids)| ids)
            .collect();
        for safe in [
            "SORTIE", "WOW", "Mouse", "REAL", "PLUGGED", "2222", "EEE", "AAA", "BBB",
        ] {
            assert!(
                !all.iter().any(|i| i.contains(safe)),
                "{safe} would be removed: {all:?}"
            );
        }
    }

    #[test]
    fn the_same_name_in_another_case_or_with_spaces_is_still_the_same_device() {
        let kept = KeptDevices {
            microphones: vec!["  sortie ".into()],
            speakers: vec!["wow".into()],
        };
        let stale = classify(V, P, &machine(), &kept);
        assert!(
            stale
                .iter()
                .all(|(s, _)| s.name != "SORTIE" && s.name != "WOW"),
            "{stale:?}"
        );
    }

    #[test]
    fn an_empty_graph_makes_every_gone_device_of_ours_stale_but_not_the_plugged_ones() {
        let stale = classify(V, P, &machine(), &KeptDevices::default());
        let names: Vec<&str> = stale.iter().map(|(s, _)| s.name.as_str()).collect();
        assert_eq!(names, ["Old Mic", "Orphan", "SORTIE", "WOW"]);
        let ids: Vec<&String> = stale.iter().flat_map(|(_, i)| i).collect();
        assert!(!ids
            .iter()
            .any(|i| i.contains("PLUGGED") || i.contains("2222")));
    }

    #[test]
    fn nothing_remembered_is_nothing_stale() {
        assert!(classify(V, P, &[], &kept()).is_empty());
    }

    /// Reading is harmless and needs no rights: on a real Windows machine it only has to work.
    #[cfg(windows)]
    #[test]
    #[ignore = "reads the real device list of this computer"]
    fn the_real_device_list_can_be_read() {
        let all = remembered(V, P).expect("the device list");
        for d in &all {
            println!(
                "{} | {} | present={} | parent={:?}",
                d.instance_id, d.name, d.present, d.parent
            );
        }
        for (stale, ids) in classify(V, P, &all, &KeptDevices::default()) {
            println!("STALE {} -> {ids:?}", stale.name);
        }
    }
}
