//! How a client finds the local daemon: the daemon writes a small file in the user's data directory with the
//! address it listens on and the access token; the command line and the desktop window read it.
//!
//! The file is only readable by the user (it lives in the user's profile), which is what keeps the token private on
//! a shared machine.

pub mod durable;
pub mod layout;

pub use layout::DataLayout;

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

    /// Where the file is (in the `run` folder of the data folder `dir`).
    pub fn path_in(dir: &std::path::Path) -> PathBuf {
        DataLayout::new(dir).daemon_file()
    }

    /// Where an earlier version of the daemon (still running after an update) wrote it: the root of the data folder.
    fn old_path_in(dir: &std::path::Path) -> PathBuf {
        dir.join("daemon.json")
    }

    pub fn write(&self, dir: &std::path::Path) -> io::Result<()> {
        let path = Self::path_in(dir);
        if let Some(run) = path.parent() {
            std::fs::create_dir_all(run)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let tmp = path.with_file_name("daemon.json.tmp");
        {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            // the token is a secret: on a machine that several people use, only the owner may read it
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options.open(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, path)
    }

    pub fn read(dir: &std::path::Path) -> Option<Discovery> {
        [Self::path_in(dir), Self::old_path_in(dir)]
            .iter()
            .find_map(|path| serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok())
    }

    /// Removes the file, but only when it still describes this process (a newer daemon may have replaced it).
    pub fn remove_if_mine(dir: &std::path::Path) {
        if Self::read(dir).is_some_and(|d| d.pid == std::process::id()) {
            let _ = std::fs::remove_file(Self::path_in(dir));
            let _ = std::fs::remove_file(Self::old_path_in(dir));
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

    #[cfg(unix)]
    #[test]
    fn the_file_with_the_token_is_for_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("mode");
        let d = Discovery {
            address: "127.0.0.1:7650".into(),
            token: "t0k3n".into(),
            pid: std::process::id(),
        };
        d.write(&dir).unwrap();
        let mode = std::fs::metadata(Discovery::path_in(&dir))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "nobody else may read the token");
        let _ = std::fs::remove_dir_all(&dir);
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
    fn the_file_is_in_the_run_folder_and_one_left_by_an_earlier_version_is_still_found() {
        let dir = temp("old");
        let d = Discovery {
            address: "127.0.0.1:7650".into(),
            token: "t".into(),
            pid: std::process::id(),
        };
        d.write(&dir).unwrap();
        assert!(dir.join("run").join("daemon.json").is_file());
        assert!(!dir.join("daemon.json").exists());
        Discovery::remove_if_mine(&dir);
        // what an earlier version wrote at the root
        std::fs::write(dir.join("daemon.json"), serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(Discovery::read(&dir), Some(d));
        Discovery::remove_if_mine(&dir);
        assert!(!dir.join("daemon.json").exists());
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
        std::fs::create_dir_all(Discovery::path_in(&dir).parent().unwrap()).unwrap();
        std::fs::write(Discovery::path_in(&dir), "nope").unwrap();
        assert_eq!(Discovery::read(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
