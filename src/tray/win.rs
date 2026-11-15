//! The Windows icon: `Shell_NotifyIconW` and a window that is never shown, on a thread of its own.

use super::{Actions, Texts};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
    SetForegroundWindow, TrackPopupMenu, TranslateMessage, HICON, IDI_APPLICATION, LR_DEFAULTCOLOR,
    MF_SEPARATOR, MF_STRING, MSG, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_COMMAND,
    WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};

/// The message the system sends the window about the icon (legacy form: `lParam` is the mouse message).
pub(crate) const WM_TRAY: u32 = WM_APP + 1;
const ID_OPEN: usize = 1;
const ID_STOP_ENGINE: usize = 2;
const ID_QUIT: usize = 3;
/// The one icon this program has in the notification area.
const ICON_ID: u32 = 1;

/// What the window procedure calls (it cannot carry state of its own).
struct Shared {
    actions: Actions,
    texts: Texts,
}

fn shared() -> &'static Mutex<Option<Arc<Shared>>> {
    static SHARED: OnceLock<Mutex<Option<Arc<Shared>>>> = OnceLock::new();
    SHARED.get_or_init(|| Mutex::new(None))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The language of the computer, as a locale name (`fr-FR`).
fn locale() -> String {
    let mut buffer = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    if n > 1 {
        String::from_utf16_lossy(&buffer[..n as usize - 1])
    } else {
        String::new()
    }
}

/// The product's icon, made from the picture inside `icon.ico` (an ICO file is a directory of pictures; the largest is
/// taken and the system scales it); the system's plain application icon when that fails.
fn make_icon() -> (HICON, bool) {
    let ico: &[u8] = include_bytes!("../../icons/icon.ico");
    let largest = (|| {
        let count = u16::from_le_bytes([*ico.get(4)?, *ico.get(5)?]) as usize;
        (0..count)
            .filter_map(|i| {
                let e = ico.get(6 + 16 * i..22 + 16 * i)?;
                let width = if e[0] == 0 { 256 } else { e[0] as u32 };
                let size = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
                let offset = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
                Some((width, ico.get(offset..offset.checked_add(size)?)?))
            })
            .max_by_key(|(w, _)| *w)
            .map(|(_, data)| data)
    })();
    if let Some(data) = largest {
        // 0x00030000: the version the system asks for; a PNG picture is accepted as it is
        if let Ok(icon) =
            unsafe { CreateIconFromResourceEx(data, true, 0x0003_0000, 0, 0, LR_DEFAULTCOLOR) }
        {
            return (icon, true);
        }
    }
    let plain = unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default();
    (plain, false)
}

fn icon_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    // the tooltip: the product's name
    let tip = wide(crate::branding::DISPLAY_NAME);
    let n = tip.len().min(data.szTip.len());
    data.szTip[..n].copy_from_slice(&tip[..n]);
    if n == data.szTip.len() {
        data.szTip[n - 1] = 0;
    }
    data
}

/// The menu at the cursor; returns what was chosen.
fn show_menu(hwnd: HWND, texts: &Texts) -> usize {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return 0;
        };
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, PCWSTR(wide(texts.open).as_ptr()));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            ID_STOP_ENGINE,
            PCWSTR(wide(texts.stop_engine).as_ptr()),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, PCWSTR(wide(texts.quit).as_ptr()));
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // the menu closes by itself when it loses the focus only if its window had it first
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD,
            at.x,
            at.y,
            Some(0),
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        chosen.0 as usize
    }
}

fn run_command(shared: &Shared, id: usize) {
    // a failure of an action never takes the icon down with it
    let _ = crate::diagnostics::contained(|| match id {
        ID_OPEN => (shared.actions.open)(),
        ID_STOP_ENGINE => (shared.actions.stop_engine)(),
        ID_QUIT => (shared.actions.quit)(),
        _ => {}
    });
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state = shared().lock().ok().and_then(|s| s.clone());
    match message {
        WM_TRAY => {
            if let Some(state) = state {
                match lparam.0 as u32 {
                    // a click opens the interface, a right click the menu
                    WM_LBUTTONUP => run_command(&state, ID_OPEN),
                    WM_RBUTTONUP => {
                        let chosen = show_menu(hwnd, &state.texts);
                        run_command(&state, chosen);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            if let Some(state) = state {
                run_command(&state, wparam.0 & 0xffff);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// The icon in the notification area. Dropping it takes the icon away.
pub struct Tray {
    window: isize,
    thread: Option<JoinHandle<()>>,
}

impl Tray {
    /// Puts the icon there. Returns once it is, or says why it could not be.
    pub fn start(actions: Actions) -> Result<Tray, String> {
        *shared().lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(Shared {
            actions,
            texts: Texts::for_locale(&locale()),
        }));
        let (tx, rx) = mpsc::channel::<Result<isize, String>>();
        let thread = std::thread::Builder::new()
            .name("tray".into())
            .spawn(move || {
                let outcome = crate::diagnostics::contained(|| run_thread(&tx));
                if outcome.is_err() {
                    let _ = tx.send(Err("the icon thread failed".into()));
                }
            })
            .map_err(|e| format!("cannot start the icon thread: {e}"))?;
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(window)) => Ok(Tray {
                window,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err("the notification-area icon did not answer".into()),
        }
    }

    /// The icon's window (for the tests, which send it what the system would).
    #[cfg(test)]
    fn window(&self) -> HWND {
        HWND(self.window as *mut _)
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // closing the window ends the thread's loop, which takes the icon away
        let _ = unsafe {
            PostMessageW(
                Some(HWND(self.window as *mut _)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        *shared().lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// The icon thread: a hidden window, the icon, and the message loop.
fn run_thread(started: &mpsc::Sender<Result<isize, String>>) {
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            let _ = started.send(Err("no module handle".into()));
            return;
        };
        let class = w!("OmuTrayWindow");
        let registered = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        });
        // a second icon in the same run finds the class registered already: that is fine
        let _ = registered;
        let hwnd = match CreateWindowExW(
            Default::default(),
            class,
            w!(""),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                let _ = started.send(Err(format!("cannot make the icon's window: {e}")));
                return;
            }
        };
        let (icon, owned) = make_icon();
        let data = icon_data(hwnd, icon);
        if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
            let _ = started.send(Err("the system refused the notification-area icon".into()));
            let _ = DestroyWindow(hwnd);
            return;
        }
        // when the taskbar is made again (the explorer restarted) the icon goes with it: put it back
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        let _ = started.send(Ok(hwnd.0 as isize));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            if taskbar_created != 0 && msg.message == taskbar_created {
                let _ = Shell_NotifyIconW(NIM_ADD, &data);
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        if owned {
            let _ = DestroyIcon(icon);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use windows::Win32::UI::Shell::{Shell_NotifyIconGetRect, NOTIFYICONIDENTIFIER};
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

    /// Starts an icon whose actions only count. It puts an icon in the notification area for a moment.
    #[test]
    fn the_icon_is_there_a_click_and_the_menu_commands_reach_the_actions_and_it_goes_when_dropped()
    {
        let counts: Arc<[AtomicU32; 3]> = Arc::new(Default::default());
        let count = |k: usize| {
            let c = counts.clone();
            Box::new(move || {
                c[k].fetch_add(1, Ordering::SeqCst);
            }) as Box<dyn Fn() + Send + Sync>
        };
        let tray = Tray::start(Actions {
            open: count(0),
            stop_engine: count(1),
            quit: count(2),
        })
        .expect("the icon starts");

        // the system knows the icon: it has a place on the screen
        let id = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: tray.window(),
            uID: ICON_ID,
            ..Default::default()
        };
        assert!(
            unsafe { Shell_NotifyIconGetRect(&id) }.is_ok(),
            "the icon exists"
        );

        // what the system sends for a click, and what the menu sends for each choice
        unsafe {
            SendMessageW(
                tray.window(),
                WM_TRAY,
                Some(WPARAM(0)),
                Some(LPARAM(WM_LBUTTONUP as isize)),
            );
            SendMessageW(
                tray.window(),
                WM_COMMAND,
                Some(WPARAM(ID_STOP_ENGINE)),
                Some(LPARAM(0)),
            );
            SendMessageW(
                tray.window(),
                WM_COMMAND,
                Some(WPARAM(ID_QUIT)),
                Some(LPARAM(0)),
            );
        }
        let seen: Vec<u32> = counts.iter().map(|c| c.load(Ordering::SeqCst)).collect();
        assert_eq!(seen, vec![1, 1, 1]);

        let window = tray.window();
        drop(tray);
        let id = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: window,
            uID: ICON_ID,
            ..Default::default()
        };
        assert!(
            unsafe { Shell_NotifyIconGetRect(&id) }.is_err(),
            "the icon went with it"
        );
    }

    #[test]
    fn the_product_icon_can_be_made_from_the_ico_file() {
        let (icon, owned) = make_icon();
        assert!(!icon.is_invalid());
        assert!(
            owned,
            "the picture in icon.ico was used, not the plain system icon"
        );
        let _ = unsafe { DestroyIcon(icon) };
    }
}
