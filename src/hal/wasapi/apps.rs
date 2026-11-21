//! Applications known to the audio system, and resolving an [`AppTarget`] to a process id.

use super::com::{backend_error, ComGuard};
use super::devices::list_direction;
use crate::{AppInfo, Direction, HalError};
use omu_types::AppTarget;
use std::collections::{HashMap, HashSet};
use windows::core::Interface;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2,
};
use windows::Win32::System::Com::{CoTaskMemFree, CLSCTX_ALL};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

/// What `IAudioSessionControl2::IsSystemSoundsSession` answers: S_OK means "this is the system sounds session" and S_FALSE (1)
/// means "it is not". Both are successes, so `HRESULT::is_ok()` (any value from 0 up) cannot tell them apart.
fn is_system_sounds(answer: windows::core::HRESULT) -> bool {
    answer == windows::Win32::Foundation::S_OK
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent: u32,
    pub exe: String,
}

/// All running processes.
pub fn processes() -> Vec<ProcessEntry> {
    let mut out = vec![];
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return out;
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while ok {
        let len = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        out.push(ProcessEntry {
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            exe: String::from_utf16_lossy(&entry.szExeFile[..len]),
        });
        ok = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    let _ = unsafe { CloseHandle(snapshot) };
    out
}

/// One audio session of an application on a playback device.
#[derive(Debug, Clone)]
struct Session {
    pid: u32,
    title: Option<String>,
    active: bool,
}

fn sessions() -> Result<Vec<Session>, HalError> {
    let _com = ComGuard::new();
    let mut out = vec![];
    for (device, _) in list_direction(Direction::Render)? {
        let manager: IAudioSessionManager2 = match unsafe { device.Activate(CLSCTX_ALL, None) } {
            Ok(m) => m,
            Err(_) => continue,
        };
        let Ok(list) = (unsafe { manager.GetSessionEnumerator() }) else {
            continue;
        };
        let count =
            unsafe { list.GetCount() }.map_err(|e| backend_error("GetCount(sessions)", e))?;
        for i in 0..count {
            let Ok(control) = (unsafe { list.GetSession(i) }) else {
                continue;
            };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                continue;
            };
            if is_system_sounds(unsafe { control2.IsSystemSoundsSession() }) {
                continue;
            }
            let Ok(pid) = (unsafe { control2.GetProcessId() }) else {
                continue;
            };
            if pid == 0 {
                continue;
            }
            let active = unsafe { control2.GetState() }
                .map(|s| s == AudioSessionStateActive)
                .unwrap_or(false);
            let title = unsafe { control2.GetDisplayName() }.ok().and_then(|p| {
                let s = unsafe { p.to_string() }.ok();
                unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
                s.filter(|s| !s.trim().is_empty())
            });
            out.push(Session { pid, title, active });
        }
    }
    Ok(out)
}

/// Applications that have an audio session, once each (a session active on any device counts as active).
pub fn list_apps() -> Result<Vec<AppInfo>, HalError> {
    let procs: HashMap<u32, String> = processes().into_iter().map(|p| (p.pid, p.exe)).collect();
    let mut apps: HashMap<u32, AppInfo> = HashMap::new();
    for s in sessions()? {
        let name = procs
            .get(&s.pid)
            .cloned()
            .unwrap_or_else(|| format!("pid {}", s.pid));
        let entry = apps.entry(s.pid).or_insert(AppInfo {
            pid: s.pid,
            name,
            title: None,
            active: false,
        });
        entry.active |= s.active;
        if entry.title.is_none() {
            entry.title = s.title;
        }
    }
    let mut out: Vec<AppInfo> = apps.into_values().collect();
    out.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.pid.cmp(&b.pid))
    });
    Ok(out)
}

/// True if `ancestor` is `pid` itself or one of its parents.
fn is_in_tree(procs: &HashMap<u32, u32>, ancestor: u32, pid: u32) -> bool {
    let mut cur = pid;
    for _ in 0..64 {
        if cur == ancestor {
            return true;
        }
        match procs.get(&cur) {
            Some(&parent) if parent != cur && parent != 0 => cur = parent,
            _ => return false,
        }
    }
    false
}

/// The process whose tree is captured for `target`.
///
/// * `Pid(p)`: `p`.
/// * `ProcessName(n)`: among the processes called `n`, the roots of their groups (those whose parent is not itself
///   one of them); the root whose tree has an active audio session wins, else the first one.
///
/// Refused: this process, or any ancestor of it (it would capture this product's own sound).
pub fn resolve_target(target: &AppTarget) -> Result<u32, HalError> {
    let all = processes();
    let parents: HashMap<u32, u32> = all.iter().map(|p| (p.pid, p.parent)).collect();
    let me = std::process::id();
    let pid = match target {
        AppTarget::Pid(p) => {
            if !parents.contains_key(p) {
                return Err(HalError::NotFound(format!("no process with id {p}")));
            }
            *p
        }
        AppTarget::ProcessName(name) => {
            let named: Vec<&ProcessEntry> = all
                .iter()
                .filter(|p| p.exe.eq_ignore_ascii_case(name))
                .collect();
            if named.is_empty() {
                return Err(HalError::NotFound(format!(
                    "no running process named {name:?}"
                )));
            }
            let pids: HashSet<u32> = named.iter().map(|p| p.pid).collect();
            let roots: Vec<u32> = named
                .iter()
                .filter(|p| !pids.contains(&p.parent))
                .map(|p| p.pid)
                .collect();
            let roots = if roots.is_empty() {
                named.iter().map(|p| p.pid).collect()
            } else {
                roots
            };
            let active: Vec<u32> = sessions()
                .unwrap_or_default()
                .into_iter()
                .filter(|s| s.active)
                .map(|s| s.pid)
                .collect();
            roots
                .iter()
                .copied()
                .find(|&r| active.iter().any(|&a| is_in_tree(&parents, r, a)))
                .unwrap_or(roots[0])
        }
    };
    if is_in_tree(&parents, pid, me) {
        return Err(HalError::Unsupported(
            "refusing to capture the sound of this program or of the program that started it (it would hear itself)".into(),
        ));
    }
    Ok(pid)
}

#[allow(dead_code)]
fn _unused<T: Interface>() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_process_list_contains_this_process() {
        let me = std::process::id();
        assert!(processes().iter().any(|p| p.pid == me));
    }

    #[test]
    fn tree_membership_follows_parents() {
        let parents: HashMap<u32, u32> =
            [(10, 1), (11, 10), (12, 11), (20, 1)].into_iter().collect();
        assert!(is_in_tree(&parents, 10, 12));
        assert!(is_in_tree(&parents, 12, 12));
        assert!(!is_in_tree(&parents, 20, 12));
        assert!(!is_in_tree(&parents, 12, 10));
    }

    #[test]
    fn capturing_ourselves_is_refused() {
        let r = resolve_target(&AppTarget::Pid(std::process::id()));
        assert!(matches!(r, Err(HalError::Unsupported(_))), "{r:?}");
    }

    #[test]
    fn unknown_targets_are_not_found() {
        assert!(matches!(
            resolve_target(&AppTarget::Pid(4_000_000_000)),
            Err(HalError::NotFound(_))
        ));
        assert!(matches!(
            resolve_target(&AppTarget::ProcessName("no-such-program-zzz.exe".into())),
            Err(HalError::NotFound(_))
        ));
    }

    #[test]
    fn listing_apps_does_not_fail() {
        assert!(list_apps().is_ok());
    }
}

#[cfg(test)]
mod system_sounds_tests {
    use super::is_system_sounds;
    use windows::core::HRESULT;

    #[test]
    fn only_s_ok_is_the_system_sounds_session() {
        assert!(is_system_sounds(HRESULT(0)), "S_OK: it is");
        assert!(!is_system_sounds(HRESULT(1)), "S_FALSE: it is not, though it is a success");
        assert!(!is_system_sounds(HRESULT(-2147467259)), "an error is not an answer");
    }
}
