//! How a client finds the local daemon: the daemon writes a small file in the user's data directory with the
//! address it listens on and the access token; the command line and the desktop window read it.
//!
//! The file is only readable by the user (it lives in the user's profile), which is what keeps the token private on
//! a shared machine.

pub mod durable;

use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

/// The user's data directory for this product (`%APPDATA%\Omu`, `~/.local/share/Omu`...), by default.
/// The environment variable `OMU_DATA_DIR` (prefix from the branding) overrides it, for tests and portable setups.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(omu_branding::env_var("DATA_DIR")) {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(omu_branding::DATA_DIR_NAME)
}

/// What the daemon publishes about itself while it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovery {
    /// Address the daemon listens on, `host:port`.
    pub address: String,
    /// Secret every request must carry.
    pub token: String,
    pub pid: u32,
}

impl Discovery {
    /// Whether something answers at the recorded address. A file left by a daemon that is gone, whose process number was given to
    /// another program since (after a restart of Windows, for example), points at nobody: it must not be taken for a running daemon.
    pub fn answers(&self) -> bool {
        self.address.parse::<std::net::SocketAddr>().is_ok_and(|a| {
            std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_millis(500)).is_ok()
        })
    }

    pub fn path_in(dir: &std::path::Path) -> PathBuf {
        dir.join("daemon.json")
    }

    pub fn write(&self, dir: &std::path::Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let tmp = dir.join("daemon.json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, Self::path_in(dir))
    }

    pub fn read(dir: &std::path::Path) -> Option<Discovery> {
        serde_json::from_str(&std::fs::read_to_string(Self::path_in(dir)).ok()?).ok()
    }

    /// Removes the file, but only when it still describes this process (a newer daemon may have replaced it).
    pub fn remove_if_mine(dir: &std::path::Path) {
        if Self::read(dir).is_some_and(|d| d.pid == std::process::id()) {
            let _ = std::fs::remove_file(Self::path_in(dir));
        }
    }

    /// The address of the web panel, with the token in the fragment (a fragment is never sent to a server or logged).
    pub fn panel_url(&self) -> String {
        format!("http://{}/#token={}", self.address, self.token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("omu-disc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_record_answers_only_when_something_listens_at_its_address() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let record = |address: String| Discovery {
            address,
            token: "t".into(),
            pid: std::process::id(),
        };
        let live = record(listener.local_addr().unwrap().to_string());
        assert!(live.answers());
        drop(listener);
        assert!(
            !live.answers(),
            "nobody there any more: the record is stale, whatever process has that number now"
        );
        assert!(!record("not an address".into()).answers());
    }

    #[test]
    fn write_read_and_remove() {
        let dir = temp("rw");
        let d = Discovery {
            address: "127.0.0.1:7650".into(),
            token: "t0k3n".into(),
            pid: std::process::id(),
        };
        d.write(&dir).unwrap();
        assert_eq!(Discovery::read(&dir), Some(d.clone()));
        assert!(d.panel_url().starts_with("http://127.0.0.1:7650/#token="));
        Discovery::remove_if_mine(&dir);
        assert_eq!(Discovery::read(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_of_another_process_is_left_alone() {
        let dir = temp("other");
        let d = Discovery {
            address: "a:1".into(),
            token: "x".into(),
            pid: std::process::id().wrapping_add(1),
        };
        d.write(&dir).unwrap();
        Discovery::remove_if_mine(&dir);
        assert!(Discovery::read(&dir).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_file_means_no_daemon() {
        let dir = temp("bad");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(Discovery::path_in(&dir), "nope").unwrap();
        assert_eq!(Discovery::read(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
