//! The tool that attaches a USB/IP device to Windows: the `usbip-win2` client (`usbip.exe`).
//!
//! It is not part of this product: it is a signed driver package made by someone else, installed by the user (this
//! product never installs a driver by itself). [`UsbipTool`] hides it behind a trait so that everything above can be
//! tested without a driver, and so that another way of attaching (or another platform's) can be plugged in.
//!
//! Applications sharing the driver must only detach the ports they attached: never `detach --all`.

use std::path::PathBuf;

/// Attaches and detaches devices served by our own USB/IP server.
pub trait UsbipTool: Send + Sync {
    /// Whether the tool is installed and usable; the error says what is missing and how to get it.
    fn available(&self) -> Result<(), String>;
    /// Attaches `busid` of the server listening on `127.0.0.1:server_port`; returns the tool's port number.
    fn attach(&self, server_port: u16, busid: &str) -> Result<u32, String>;
    /// Detaches a port this program attached.
    fn detach(&self, port: u32) -> Result<(), String>;
    /// The devices currently attached through the tool (by any program).
    fn imported(&self) -> Result<Vec<Imported>, String>;
    /// Makes exactly the devices on `ports` permanent: the driver attaches them again by itself every time it is
    /// loaded (at every boot) and whenever their connection drops, as long as the server answers. The tool keeps one
    /// list for everybody: this REPLACES it, so a port that is left out stops being permanent.
    fn stash(&self, ports: &[u32]) -> Result<(), String>;
    /// The permanent devices now, one line each (as the tool prints them).
    fn stashed(&self) -> Result<Vec<String>, String>;
}

/// A device attached through the USB/IP client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub port: u32,
    pub vendor_id: u16,
    pub product_id: u16,
    /// The TCP port of the server it comes from, when it is on this machine.
    pub server_port: Option<u16>,
    /// Its bus id on that server.
    pub busid: Option<String>,
}

/// Reads the output of `usbip port`, for example:
///
/// ```text
/// Imported USB devices
/// ====================
/// Port 01: device in use at High Speed(480Mbps)
///          Generic : pid.codes Test PID (1209:0001)
///            -> usbip://127.0.0.1:43108/1-1
///            -> remote bus/dev 001/001
/// ```
pub fn parse_imported(text: &str) -> Vec<Imported> {
    let mut out: Vec<Imported> = vec![];
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Port ") {
            if let Some(port) = rest.split(':').next().and_then(|p| p.trim().parse().ok()) {
                out.push(Imported {
                    port,
                    vendor_id: 0,
                    product_id: 0,
                    server_port: None,
                    busid: None,
                });
            }
        } else if let Some(current) = out.last_mut() {
            if let Some(ids) = line.rsplit_once('(').and_then(|(_, r)| r.strip_suffix(')')) {
                if let Some((v, p)) = ids.split_once(':') {
                    if let (Ok(v), Ok(p)) = (u16::from_str_radix(v, 16), u16::from_str_radix(p, 16))
                    {
                        current.vendor_id = v;
                        current.product_id = p;
                    }
                }
            }
            if let Some(url) = line.strip_prefix("-> usbip://127.0.0.1:") {
                let (port, busid) = url.split_once('/').unwrap_or((url, ""));
                current.server_port = port.parse().ok();
                current.busid = (!busid.is_empty()).then(|| busid.to_owned());
            }
        }
    }
    out
}

/// The Windows client, `usbip.exe` of usbip-win2.
#[derive(Debug, Default, Clone)]
pub struct UsbipExe;

impl UsbipExe {
    /// Where `usbip.exe` is: `USBIP_EXE` if set, else the installer's default folders.
    pub fn locate() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("USBIP_EXE") {
            return Some(path.into());
        }
        ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|base| PathBuf::from(base).join("USBip").join("usbip.exe"))
            .find(|path| path.is_file())
    }

    #[cfg(windows)]
    fn command(path: &std::path::Path) -> std::process::Command {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = std::process::Command::new(path);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }

    #[cfg(not(windows))]
    fn command(path: &std::path::Path) -> std::process::Command {
        std::process::Command::new(path)
    }
}

/// How long a `usbip.exe` command may take. The driver can be busy or being reloaded, and the command then never returns; the engine asks
/// for these commands while it holds what everything else waits for, so a command that hangs must not be waited for.
const WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// Runs `command` and collects what it says, but gives up on a program that is not finished after `limit`: it is killed.
fn run(
    mut command: std::process::Command,
    limit: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    use std::process::Stdio;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    // read as it writes, so that a program that says a lot is never stuck on a full pipe
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut text);
            }
            text
        })
    };
    let out = reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let end = std::time::Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if std::time::Instant::now() >= end {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("it did not finish in {} seconds and was stopped", limit.as_secs()),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    Ok(std::process::Output {
        status,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

const MISSING: &str =
    "the USB/IP client (usbip-win2) is not installed: virtual microphones need it \
(https://github.com/vadimgrn/usbip-win2/releases)";

impl UsbipTool for UsbipExe {
    fn available(&self) -> Result<(), String> {
        if cfg!(windows) {
            Self::locate()
                .map(|_| ())
                .ok_or_else(|| MISSING.to_string())
        } else {
            Err("virtual microphones through USB/IP are only available on Windows for now".into())
        }
    }

    fn attach(&self, server_port: u16, busid: &str) -> Result<u32, String> {
        let path = Self::locate().ok_or_else(|| MISSING.to_string())?;
        let mut command = Self::command(&path);
        command
            .args([
                "--tcp-port",
                &server_port.to_string(),
                "attach",
                "--remote",
                "127.0.0.1",
                "--bus-id",
                busid,
            ])
            .args(["--once", "--terse"]);
        let output = run(command, WAIT).map_err(|e| format!("could not run {}: {e}", path.display()))?;
        if !output.status.success() {
            return Err(format!(
                "usbip attach failed: {}{}",
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        text.trim()
            .parse()
            .map_err(|_| format!("usbip attach returned no port number: {text:?}"))
    }

    fn detach(&self, port: u32) -> Result<(), String> {
        let path = Self::locate().ok_or_else(|| MISSING.to_string())?;
        let mut command = Self::command(&path);
        command.args(["detach", "--port", &port.to_string()]);
        let status = run(command, WAIT)
            .map_err(|e| format!("could not run {}: {e}", path.display()))?
            .status;
        status
            .success()
            .then_some(())
            .ok_or_else(|| format!("usbip detach of port {port} failed: {status}"))
    }

    fn imported(&self) -> Result<Vec<Imported>, String> {
        let path = Self::locate().ok_or_else(|| MISSING.to_string())?;
        let mut command = Self::command(&path);
        command.arg("port");
        let output = run(command, WAIT).map_err(|e| format!("could not run {}: {e}", path.display()))?;
        Ok(parse_imported(&String::from_utf8_lossy(&output.stdout)))
    }

    fn stash(&self, ports: &[u32]) -> Result<(), String> {
        // `port --stash` with no port number makes EVERY attached device permanent. With nothing attached at all that
        // is exactly what empties the list; with other devices attached it must never be called that way.
        if ports.is_empty() && !self.imported()?.is_empty() {
            return Err(
                "other USB/IP devices are attached: the permanent list is left as it is".into(),
            );
        }
        let path = Self::locate().ok_or_else(|| MISSING.to_string())?;
        let mut command = Self::command(&path);
        command.args(["port", "--stash"]).args(ports.iter().map(|p| p.to_string()));
        let output = run(command, WAIT).map_err(|e| format!("could not run {}: {e}", path.display()))?;
        output.status.success().then_some(()).ok_or_else(|| {
            format!(
                "usbip port --stash failed: {}{}",
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            )
        })
    }

    fn stashed(&self) -> Result<Vec<String>, String> {
        let path = Self::locate().ok_or_else(|| MISSING.to_string())?;
        let mut command = Self::command(&path);
        command.args(["list", "--stashed"]);
        let output = run(command, WAIT).map_err(|e| format!("could not run {}: {e}", path.display()))?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect())
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::{Imported, UsbipTool};
    use std::sync::Mutex;

    /// A tool that records what it is asked and can be told to fail.
    #[derive(Default)]
    pub struct FakeTool {
        pub unavailable: Mutex<Option<String>>,
        pub attach_error: Mutex<Option<String>>,
        pub attached: Mutex<Vec<(u16, String)>>,
        pub detached: Mutex<Vec<u32>>,
        pub imported: Mutex<Vec<Imported>>,
        /// Every list of ports made permanent, in the order they were asked.
        pub stashes: Mutex<Vec<Vec<u32>>>,
        /// What `list --stashed` prints.
        pub stashed_lines: Mutex<Vec<String>>,
    }

    impl UsbipTool for FakeTool {
        fn available(&self) -> Result<(), String> {
            self.unavailable.lock().unwrap().clone().map_or(Ok(()), Err)
        }
        fn attach(&self, server_port: u16, busid: &str) -> Result<u32, String> {
            if let Some(e) = self.attach_error.lock().unwrap().clone() {
                return Err(e);
            }
            let mut attached = self.attached.lock().unwrap();
            attached.push((server_port, busid.to_owned()));
            Ok(attached.len() as u32)
        }
        fn detach(&self, port: u32) -> Result<(), String> {
            self.detached.lock().unwrap().push(port);
            Ok(())
        }
        fn imported(&self) -> Result<Vec<Imported>, String> {
            Ok(self.imported.lock().unwrap().clone())
        }
        fn stash(&self, ports: &[u32]) -> Result<(), String> {
            self.stashes.lock().unwrap().push(ports.to_vec());
            Ok(())
        }
        fn stashed(&self) -> Result<Vec<String>, String> {
            Ok(self.stashed_lines.lock().unwrap().clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_port_listing() {
        let text = "Imported USB devices
====================
Port 01: device in use at High Speed(480Mbps)
         Generic : pid.codes Test PID (1209:0001)
           -> usbip://127.0.0.1:43108/1-1
           -> remote bus/dev 001/001
Port 02: device in use at Full Speed(12Mbps)
         Acme : Gadget (046d:c52b)
           -> usbip://192.168.1.5:3240/3-2
";
        let got = parse_imported(text);
        assert_eq!(
            got,
            vec![
                Imported {
                    port: 1,
                    vendor_id: 0x1209,
                    product_id: 0x0001,
                    server_port: Some(43108),
                    busid: Some("1-1".into()),
                },
                Imported {
                    port: 2,
                    vendor_id: 0x046d,
                    product_id: 0xc52b,
                    server_port: None,
                    busid: None,
                },
            ]
        );
    }

    #[test]
    fn an_empty_listing_is_no_devices() {
        assert!(parse_imported("").is_empty());
        assert!(parse_imported(
            "Imported USB devices
====================
"
        )
        .is_empty());
    }
}

#[cfg(all(test, windows))]
mod run_tests {
    use super::run;
    use std::time::{Duration, Instant};

    fn cmd(script: &str) -> std::process::Command {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", script]);
        c
    }

    #[test]
    fn a_program_is_read_to_its_end_and_one_that_hangs_is_stopped_instead_of_waited_for() {
        let out = run(cmd("echo hello"), Duration::from_secs(10)).unwrap();
        assert!(out.status.success() && String::from_utf8_lossy(&out.stdout).contains("hello"));
        let started = Instant::now();
        let e = run(cmd("ping -n 30 127.0.0.1 >nul"), Duration::from_millis(400)).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut, "{e}");
        assert!(started.elapsed() < Duration::from_secs(5), "not waited for the whole of it");
    }
}
