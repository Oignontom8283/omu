//! Opening things the way the system does for the user: an address in the browser, a folder in the file manager.

/// Opens an address, a file or a folder the way the system does for the user (the browser, the file manager...).
pub fn open_target(target: &str) -> Result<(), String> {
    #[cfg(windows)]
    let command = {
        // not `cmd /C start`: it would read an `&` in the address as another command
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", target]);
        c
    };
    #[cfg(target_os = "macos")]
    let command = {
        let mut c = std::process::Command::new("open");
        c.arg(target);
        c
    };
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(target);
        c
    };
    spawn_detached(command)
}

/// Starts a program without waiting for it and without a console window.
pub fn spawn_detached(mut command: std::process::Command) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // no console window
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot start {:?}: {e}", command.get_program()))
}
