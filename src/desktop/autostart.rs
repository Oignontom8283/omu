//! Starting the daemon when the user logs in.
//!
//! The daemon has to run in the user's own session (keyboard shortcuts, the sound of applications and the icon all live
//! there), so "start with the computer" means "start at logon": an entry in the user's own startup list, which needs no
//! administrator rights and is removed the same way. On Windows that list is the `Run` key of the current user; other
//! systems have no way here yet.

/// The name of the entry in the startup list.
pub const ENTRY: &str = omu_branding::DISPLAY_NAME;

/// What the startup list runs: the daemon, without a console window.
pub fn command_line(exe: &std::path::Path) -> String {
    format!("\"{}\" --background", exe.display())
}

#[cfg(windows)]
mod win {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Writes (or replaces) the entry `name` of the startup list.
    pub fn set(name: &str, command: &str) -> Result<(), String> {
        let key_name = wide(RUN_KEY);
        let value_name = wide(name);
        let data = wide(command);
        unsafe {
            let mut key = HKEY::default();
            let opened = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_name.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut key,
                None,
            );
            if opened != ERROR_SUCCESS {
                return Err(format!("cannot open the startup list ({})", opened.0));
            }
            let bytes = std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2);
            let written =
                RegSetValueExW(key, PCWSTR(value_name.as_ptr()), None, REG_SZ, Some(bytes));
            let _ = RegCloseKey(key);
            if written != ERROR_SUCCESS {
                return Err(format!("cannot write the startup entry ({})", written.0));
            }
        }
        Ok(())
    }

    /// Removes the entry; fine when it is not there.
    pub fn remove(name: &str) -> Result<(), String> {
        let key_name = wide(RUN_KEY);
        let value_name = wide(name);
        unsafe {
            let mut key = HKEY::default();
            let opened = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_name.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut key,
                None,
            );
            if opened != ERROR_SUCCESS {
                return Err(format!("cannot open the startup list ({})", opened.0));
            }
            // 2 = the value was not there: nothing to remove
            let removed = RegDeleteValueW(key, PCWSTR(value_name.as_ptr()));
            let _ = RegCloseKey(key);
            if removed != ERROR_SUCCESS && removed.0 != 2 {
                return Err(format!("cannot remove the startup entry ({})", removed.0));
            }
        }
        Ok(())
    }

    /// What the entry runs now, when there is one.
    pub fn get(name: &str) -> Option<String> {
        let key_name = wide(RUN_KEY);
        let value_name = wide(name);
        let mut buffer = vec![0u16; 2048];
        let mut bytes = (buffer.len() * 2) as u32;
        let read = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key_name.as_ptr()),
                PCWSTR(value_name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        if read != ERROR_SUCCESS {
            return None;
        }
        let chars = (bytes as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buffer[..chars]))
    }
}

/// Makes the startup list hold exactly what `enabled` says: the daemon at `exe` is started at logon, or nothing is.
pub fn apply(entry: &str, enabled: bool, exe: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        if enabled {
            // writing again each time keeps the entry right when the program was moved or updated
            win::set(entry, &command_line(exe))
        } else {
            win::remove(entry)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (entry, enabled, exe);
        Err("starting at logon is not available on this system yet".into())
    }
}

/// What the startup list runs for `entry`, when it has one.
pub fn current(entry: &str) -> Option<String> {
    #[cfg(windows)]
    {
        win::get(entry)
    }
    #[cfg(not(windows))]
    {
        let _ = entry;
        None
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn the_entry_is_written_read_back_replaced_and_removed() {
        // a name of its own: the real entry of the program is never touched by a test
        let name = format!("OmuTest{}", std::process::id());
        let exe = std::path::Path::new(r"C:\Program Files\Omu\omud.exe");
        assert_eq!(current(&name), None);
        apply(&name, true, exe).unwrap();
        assert_eq!(
            current(&name).as_deref(),
            Some(r#""C:\Program Files\Omu\omud.exe" --background"#)
        );
        // again with another place: replaced, not added
        apply(&name, true, std::path::Path::new(r"D:\omu\omud.exe")).unwrap();
        assert_eq!(
            current(&name).as_deref(),
            Some(r#""D:\omu\omud.exe" --background"#)
        );
        apply(&name, false, exe).unwrap();
        assert_eq!(current(&name), None);
        // removing what is not there is fine
        apply(&name, false, exe).unwrap();
    }
}

#[cfg(test)]
mod portable_tests {
    use super::*;

    #[test]
    fn the_command_quotes_the_path_and_runs_without_a_console() {
        let c = command_line(std::path::Path::new("/opt/my omu/omud"));
        assert!(c.starts_with('"') && c.ends_with("\" --background"), "{c}");
        assert!(c.contains("my omu"));
    }
}
