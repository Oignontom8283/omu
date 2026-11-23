//! The `PATH` of the computer: a program's folder put in it (or taken out), so that its commands can be typed anywhere.
//!
//! This is the list of the whole machine, in the registry, so it needs administrator rights (what an installer has). On Windows the
//! value keeps its kind (it holds `%SystemRoot%`-like references), whatever its length, and the programs already running are told
//! that it changed. Other systems have no such list here.

/// `path` (a `PATH` value: folders separated by `;`) with `entry` in it, or without it. The folders that were not touched keep their
/// order and their text; comparing ignores the case and a closing backslash, as Windows does.
pub fn with_entry(path: &str, entry: &str, present: bool) -> String {
    let key = |p: &str| p.trim().trim_end_matches(['\\', '/']).to_ascii_lowercase();
    let wanted = key(entry);
    let mut parts: Vec<&str> = path.split(';').filter(|p| !p.trim().is_empty()).collect();
    if present {
        if !parts.iter().any(|p| key(p) == wanted) {
            parts.push(entry.trim());
        }
    } else {
        parts.retain(|p| key(p) != wanted);
    }
    parts.join(";")
}

/// Puts `folder` in the `PATH` of the computer, or takes it out.
pub fn set(folder: &std::path::Path, present: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        win::set(&folder.display().to_string(), present)
    }
    #[cfg(not(windows))]
    {
        let _ = (folder, present);
        Err("the PATH of the computer is only handled on Windows".into())
    }
}

#[cfg(windows)]
mod win {
    use super::with_entry;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM, WPARAM};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE,
        KEY_SET_VALUE, REG_VALUE_TYPE, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    const KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    const NAME: &str = "Path";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The value as it is written (references not expanded), whatever its length, and its kind (plain or with references).
    fn read() -> Result<(String, REG_VALUE_TYPE), String> {
        let key = wide(KEY);
        let name = wide(NAME);
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
        let mut bytes = 0u32;
        let mut kind = REG_VALUE_TYPE::default();
        unsafe {
            let asked = RegGetValueW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                flags,
                Some(&mut kind),
                None,
                Some(&mut bytes),
            );
            if asked != ERROR_SUCCESS {
                return Err(format!("cannot read the PATH ({})", asked.0));
            }
            let mut buffer = vec![0u16; bytes as usize / 2 + 2];
            bytes = (buffer.len() * 2) as u32;
            let read = RegGetValueW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                flags,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            );
            if read != ERROR_SUCCESS {
                return Err(format!("cannot read the PATH ({})", read.0));
            }
            let chars = (bytes as usize / 2).saturating_sub(1);
            Ok((String::from_utf16_lossy(&buffer[..chars]), kind))
        }
    }

    pub fn set(folder: &str, present: bool) -> Result<(), String> {
        let (before, kind) = read()?;
        let after = with_entry(&before, folder, present);
        if after == before {
            return Ok(());
        }
        let key_name = wide(KEY);
        let name = wide(NAME);
        let data = wide(&after);
        unsafe {
            let mut key = HKEY::default();
            let opened = RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key_name.as_ptr()),
                None,
                KEY_SET_VALUE,
                &mut key,
            );
            if opened != ERROR_SUCCESS {
                return Err(format!(
                    "cannot open the PATH of the computer ({}): administrator rights are needed",
                    opened.0
                ));
            }
            let bytes = std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2);
            let written = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, kind, Some(bytes));
            let _ = RegCloseKey(key);
            if written != ERROR_SUCCESS {
                return Err(format!("cannot write the PATH ({})", written.0));
            }
            // the programs that are open learn about it (a terminal opened afterwards has it anyway)
            let environment = wide("Environment");
            let _ = SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(environment.as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                5000,
                None,
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::with_entry;

    #[test]
    fn a_folder_is_added_once_at_the_end() {
        let path = r"C:\Windows;C:\Tools\";
        let with = with_entry(path, r"C:\Program Files\Omu", true);
        assert_eq!(with, r"C:\Windows;C:\Tools\;C:\Program Files\Omu");
        assert_eq!(
            with_entry(&with, r"c:\program files\omu\", true),
            with,
            "already there, whatever the case"
        );
    }

    #[test]
    fn a_folder_is_taken_out_and_the_others_stay_as_they_are() {
        let path = r"%SystemRoot%\system32;C:\Program Files\Omu;D:\x";
        assert_eq!(
            with_entry(path, r"C:\Program Files\Omu\", false),
            r"%SystemRoot%\system32;D:\x"
        );
        assert_eq!(
            with_entry(path, r"E:\none", false),
            path,
            "nothing to take out"
        );
    }

    #[test]
    fn an_empty_path_is_fine() {
        assert_eq!(with_entry("", r"C:\a", true), r"C:\a");
        assert_eq!(with_entry(r"C:\a", r"C:\a", false), "");
    }
}
