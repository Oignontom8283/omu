//! Global keyboard shortcuts on Windows (`RegisterHotKey`).
//!
//! Each shortcut gets a small thread with its own message queue: it registers the shortcut, waits for `WM_HOTKEY`
//! messages and calls the callback; dropping the guard posts `WM_QUIT`, the thread unregisters and ends.
//! Registration fails when another program already owns the shortcut, which is reported as an error of the block.
//!
//! Limits worth knowing: a program running as administrator does not forward its shortcuts to a normal one, and a
//! game with anti-cheat may block the registration; both appear as a failed block, never as a crash.

use crate::hal::{HalError, HotkeyCallback, Running};
use crate::types::{Key, KeyCombo};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_HOTKEY, WM_QUIT,
};

/// The virtual-key code of a key.
fn virtual_key(key: Key) -> u32 {
    match key {
        Key::Letter(c) => c as u32,
        Key::Digit(d) => 0x30 + d as u32,
        Key::Function(n) => 0x70 + n as u32 - 1,
        Key::Numpad(n) => 0x60 + n as u32,
        Key::Space => 0x20,
        Key::Enter => 0x0D,
        Key::Tab => 0x09,
        Key::Escape => 0x1B,
        Key::Backspace => 0x08,
        Key::Insert => 0x2D,
        Key::Delete => 0x2E,
        Key::Home => 0x24,
        Key::End => 0x23,
        Key::PageUp => 0x21,
        Key::PageDown => 0x22,
        Key::Left => 0x25,
        Key::Up => 0x26,
        Key::Right => 0x27,
        Key::Down => 0x28,
    }
}

fn modifiers(combo: &KeyCombo) -> HOT_KEY_MODIFIERS {
    let m = combo.modifiers;
    let mut out = MOD_NOREPEAT; // holding the key does not fire again and again
    if m.ctrl {
        out |= MOD_CONTROL;
    }
    if m.alt {
        out |= MOD_ALT;
    }
    if m.shift {
        out |= MOD_SHIFT;
    }
    if m.meta {
        out |= MOD_WIN;
    }
    out
}

struct Guard {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Running for Guard {}

impl Drop for Guard {
    fn drop(&mut self) {
        // WM_QUIT to the shortcut's thread ends its message loop
        let _ = unsafe {
            PostThreadMessageW(
                self.thread_id,
                WM_QUIT,
                Default::default(),
                Default::default(),
            )
        };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Registers `combo` and calls `on_press` each time it is pressed. Returns once the registration is known.
pub fn register(combo: &KeyCombo, on_press: HotkeyCallback) -> Result<Box<dyn Running>, HalError> {
    let (combo, label) = (*combo, combo.to_string());
    let (tx, rx) = mpsc::channel::<Result<u32, String>>();
    let thread = std::thread::Builder::new()
        .name(format!("hotkey {label}"))
        .spawn(move || {
            // make sure this thread has a message queue before anyone posts to it
            let mut msg = MSG::default();
            let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
            let registered =
                unsafe { RegisterHotKey(None, 1, modifiers(&combo), virtual_key(combo.key)) };
            match registered {
                Ok(()) => {
                    let _ = tx.send(Ok(unsafe { GetCurrentThreadId() }));
                }
                Err(e) => {
                    let _ = tx.send(Err(e.message().trim().to_string()));
                    return;
                }
            }
            // GetMessageW returns 0 on WM_QUIT (and -1 on error)
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
                if msg.message == WM_HOTKEY {
                    on_press();
                }
            }
            let _ = unsafe { UnregisterHotKey(None, 1) };
        })
        .map_err(|e| HalError::Backend(format!("cannot start the shortcut thread: {e}")))?;
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(thread_id)) => Ok(Box::new(Guard {
            thread_id,
            thread: Some(thread),
        })),
        Ok(Err(reason)) => {
            let _ = thread.join();
            Err(HalError::Busy(format!(
                "the shortcut {label} is already used by another program or not allowed ({reason})"
            )))
        }
        Err(_) => Err(HalError::Backend(format!(
            "registering the shortcut {label} did not answer"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_keys_match_windows_codes() {
        assert_eq!(virtual_key(Key::Letter('A')), 0x41);
        assert_eq!(virtual_key(Key::Digit(0)), 0x30);
        assert_eq!(virtual_key(Key::Function(1)), 0x70);
        assert_eq!(virtual_key(Key::Function(24)), 0x87);
        assert_eq!(virtual_key(Key::Numpad(5)), 0x65);
        assert_eq!(virtual_key(Key::Escape), 0x1B);
        assert_eq!(virtual_key(Key::Down), 0x28);
    }

    #[test]
    fn a_free_shortcut_registers_a_taken_one_is_refused_and_dropping_releases_it() {
        // Ctrl+Alt+Shift+F24: nothing uses it
        let combo = KeyCombo::parse("Ctrl+Alt+Shift+F24").unwrap();
        let first = register(&combo, Box::new(|| {})).expect("a free shortcut registers");
        let second = register(&combo, Box::new(|| {}));
        assert!(
            matches!(&second, Err(HalError::Busy(m)) if m.contains("already used")),
            "the same shortcut twice: {:?}",
            second.err()
        );
        drop(first);
        let again = register(&combo, Box::new(|| {})).expect("released when dropped");
        drop(again);
    }

    /// Sends the keys of Ctrl+Alt+Shift+F24 (a combination no program uses) like a user would, and checks that the
    /// registered shortcut reports it. It injects keystrokes into the desktop for a moment, hence `ignore`.
    #[test]
    #[ignore = "injects keyboard input for a moment"]
    fn pressing_the_keys_fires_the_callback() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
            KEYEVENTF_KEYUP, VIRTUAL_KEY,
        };
        let hits = Arc::new(AtomicU32::new(0));
        let h2 = hits.clone();
        let combo = KeyCombo::parse("Ctrl+Alt+Shift+F24").unwrap();
        let guard = register(
            &combo,
            Box::new(move || {
                h2.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        let key = |vk: u16, up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        // Ctrl, Alt, Shift down; F24; then everything up
        let keys = [
            (0x11, false),
            (0x12, false),
            (0x10, false),
            (0x87, false),
            (0x87, true),
            (0x10, true),
            (0x12, true),
            (0x11, true),
        ];
        let inputs: Vec<INPUT> = keys.iter().map(|&(vk, up)| key(vk, up)).collect();
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        assert_eq!(sent as usize, inputs.len());
        for _ in 0..50 {
            if hits.load(Ordering::SeqCst) > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1, "the shortcut fired once");
        drop(guard);
    }
}
