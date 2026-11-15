//! Optional modules: components that are not part of the program and that people switch on when they want them.
//! The first one is FFmpeg (see [`ffmpeg`]); large models are meant to follow.
//!
//! Nothing here reaches the Internet by itself: a download starts only when a person asks for the module, and nothing
//! but the request for the files is sent.

pub mod ffmpeg;

use crate::types::{ModuleInfo, Progress};
use ffmpeg::{Counters, Found, Urls};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The id of the FFmpeg module.
pub const FFMPEG: &str = "ffmpeg";

/// What is kept between runs (`modules.json` in the data folder).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    ffmpeg_enabled: bool,
}

#[derive(Default)]
struct State {
    saved: Saved,
    installing: bool,
    error: Option<String>,
    /// What was found last, and when (looking runs FFmpeg, so it is not done on every question).
    located: Option<(Instant, Option<Found>)>,
}

/// How long what was found is believed.
const LOCATE_TTL: Duration = Duration::from_secs(10);

pub struct Modules {
    data_dir: PathBuf,
    dir: PathBuf,
    state: Mutex<State>,
    counters: Counters,
    /// Where FFmpeg is downloaded from (the official source; tests point it at a local server).
    urls: Mutex<Option<Urls>>,
    /// Whether an FFmpeg the system already has may be used (tests turn it off).
    use_system: bool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Modules {
    pub fn new(data_dir: &std::path::Path) -> Modules {
        let saved = std::fs::read_to_string(data_dir.join("modules.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Modules {
            data_dir: data_dir.to_owned(),
            dir: data_dir.join("modules"),
            state: Mutex::new(State {
                saved,
                ..State::default()
            }),
            counters: Counters::default(),
            urls: Mutex::new(ffmpeg::default_urls()),
            use_system: true,
        }
    }

    /// Where the modules are kept.
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Never uses an FFmpeg the system has (tests).
    pub fn without_system(mut self) -> Modules {
        self.use_system = false;
        self
    }

    /// Downloads from other addresses (tests).
    pub fn with_urls(self, urls: Option<Urls>) -> Modules {
        *lock(&self.urls) = urls;
        self
    }

    fn save(&self, saved: &Saved) {
        let text = serde_json::to_string_pretty(saved).unwrap_or_default();
        let tmp = self.data_dir.join("modules.json.part");
        if std::fs::create_dir_all(&self.data_dir)
            .and_then(|_| std::fs::write(&tmp, text))
            .and_then(|_| std::fs::rename(&tmp, self.data_dir.join("modules.json")))
            .is_err()
        {
            tracing::warn!("cannot save which modules are switched on");
        }
    }

    /// The FFmpeg that is usable now (found, compatible), whether or not the module is switched on.
    fn found(&self) -> Option<Found> {
        {
            let st = lock(&self.state);
            if let Some((at, found)) = &st.located {
                if at.elapsed() < LOCATE_TTL {
                    return found.clone();
                }
            }
        }
        // running FFmpeg to ask its version takes a moment: not under the lock
        let found = ffmpeg::locate(&self.dir, self.use_system);
        lock(&self.state).located = Some((Instant::now(), found.clone()));
        found
    }

    fn forget_location(&self) {
        lock(&self.state).located = None;
    }

    /// The FFmpeg to use for reading a file: only when the module is switched on and something usable is there.
    pub fn ffmpeg(&self) -> Option<PathBuf> {
        if !lock(&self.state).saved.ffmpeg_enabled {
            return None;
        }
        self.found().map(|f| f.path)
    }

    /// The modules and how each stands.
    pub fn list(&self) -> Vec<ModuleInfo> {
        let found = self.found();
        let st = lock(&self.state);
        vec![ModuleInfo {
            id: FFMPEG.into(),
            name: "FFmpeg".into(),
            description: "Reads the sound of many more files and videos (for example Opus sound in WebM videos). \
                          Used only when a file cannot be read without it."
                .into(),
            enabled: st.saved.ffmpeg_enabled,
            available: found.is_some(),
            source: found.as_ref().map(|f| f.source.to_owned()),
            version: found.as_ref().map(|f| f.version.clone()),
            installing: st.installing.then(|| Progress {
                done: self.counters.done.load(Ordering::Relaxed),
                total: self.counters.total.load(Ordering::Relaxed),
            }),
            error: st.error.clone(),
            downloadable: lock(&self.urls).is_some(),
            download_mb: ffmpeg::DOWNLOAD_MB,
        }]
    }

    fn check_id(id: &str) -> Result<(), String> {
        if id == FFMPEG {
            Ok(())
        } else {
            Err(format!("there is no module {id:?}"))
        }
    }

    /// Switches a module on or off. It cannot be switched on while nothing usable is there (install it instead).
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        Self::check_id(id)?;
        if enabled && self.found().is_none() {
            return Err("FFmpeg is not installed: install the module first".into());
        }
        let saved = {
            let mut st = lock(&self.state);
            st.saved.ffmpeg_enabled = enabled;
            st.saved.clone()
        };
        self.save(&saved);
        Ok(())
    }

    /// Switches the module on. When the system already has a compatible FFmpeg that is all; otherwise FFmpeg is
    /// downloaded into the modules folder in the background (follow it with [`Modules::list`]).
    pub fn install(self: &Arc<Self>, id: &str) -> Result<(), String> {
        Self::check_id(id)?;
        self.forget_location();
        if self.found().is_some() {
            return self.set_enabled(id, true);
        }
        let Some(urls) = lock(&self.urls).clone() else {
            return Err(
                "FFmpeg is not installed, and it cannot be downloaded on this system: install it with the system's \
                 package manager (for example `apt install ffmpeg` or `brew install ffmpeg`)"
                    .into(),
            );
        };
        {
            let mut st = lock(&self.state);
            if st.installing {
                return Ok(());
            }
            st.installing = true;
            st.error = None;
        }
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("module-download".into())
            .spawn(move || {
                tracing::info!("downloading the FFmpeg module");
                // a failure of the download must never take the daemon down with it
                let result = crate::diagnostics::crash::contained(|| {
                    ffmpeg::download(&me.dir, &urls, &me.counters)
                })
                .unwrap_or_else(|_| Err("the download failed unexpectedly (see the logs)".into()));
                me.forget_location();
                let saved = {
                    let mut st = lock(&me.state);
                    st.installing = false;
                    match &result {
                        Ok(()) => {
                            st.error = None;
                            st.saved.ffmpeg_enabled = true;
                        }
                        Err(e) => {
                            tracing::warn!("the FFmpeg module could not be installed: {e}");
                            st.error = Some(e.clone());
                        }
                    }
                    st.saved.clone()
                };
                me.save(&saved);
            });
        if let Err(e) = spawned {
            let mut st = lock(&self.state);
            st.installing = false;
            return Err(format!("cannot start the download: {e}"));
        }
        Ok(())
    }

    /// Switches the module off and deletes its copy in the modules folder (a copy the system has is not touched).
    pub fn remove(&self, id: &str) -> Result<(), String> {
        Self::check_id(id)?;
        if lock(&self.state).installing {
            return Err("FFmpeg is being downloaded: wait for it to finish".into());
        }
        let saved = {
            let mut st = lock(&self.state);
            st.saved.ffmpeg_enabled = false;
            st.error = None;
            st.saved.clone()
        };
        self.save(&saved);
        let folder = self.dir.join("ffmpeg");
        if folder.exists() {
            std::fs::remove_dir_all(&folder)
                .map_err(|e| format!("cannot delete {}: {e}", folder.display()))?;
        }
        self.forget_location();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("omu-modules-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A server that answers each of `files` (path -> bytes) once per request, on a free port; returns its address.
    fn serve(files: Vec<(&'static str, Vec<u8>)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut request = [0u8; 2048];
                let n = stream.read(&mut request).unwrap_or(0);
                let first = String::from_utf8_lossy(&request[..n]);
                let path = first.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (status, body) = match files.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => ("200 OK", body.clone()),
                    None => ("404 Not Found", Vec::new()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        address
    }

    fn wait_installed(m: &Arc<Modules>) -> ModuleInfo {
        for _ in 0..600 {
            let info = m.list().remove(0);
            if info.installing.is_none() {
                return info;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the download never ended");
    }

    #[test]
    fn a_module_that_is_not_there_cannot_be_switched_on_and_nothing_is_used_until_it_is() {
        let dir = temp("off");
        let m = Modules::new(&dir).without_system();
        assert_eq!(m.ffmpeg(), None, "off by default");
        assert!(m.set_enabled("nope", true).is_err());
        let info = m.list().remove(0);
        assert!(!info.enabled && info.installing.is_none());
        // switching on needs something usable: with none (no local copy, none on the system) it is refused
        assert!(!info.available);
        assert!(m.set_enabled(FFMPEG, true).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_with_a_wrong_checksum_is_refused_and_leaves_nothing() {
        let dir = temp("badsum");
        let address = serve(vec![
            ("/a.zip", b"not really a zip".to_vec()),
            (
                "/a.zip.sha256",
                format!("{}  a.zip", "0".repeat(64)).into_bytes(),
            ),
        ]);
        let m = Arc::new(Modules::new(&dir).without_system().with_urls(Some(Urls {
            archive: format!("http://{address}/a.zip"),
            sha256: format!("http://{address}/a.zip.sha256"),
        })));
        m.install(FFMPEG).unwrap();
        let info = wait_installed(&m);
        let error = info.error.expect("the failure is reported");
        assert!(error.contains("checksum"), "{error}");
        assert!(!info.enabled && !info.available);
        assert!(!ffmpeg::local_exe(m.dir()).exists());
        let leftovers: Vec<_> = std::fs::read_dir(m.dir().join("ffmpeg"))
            .map(|d| d.map(|e| e.unwrap().file_name()).collect())
            .unwrap_or_default();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_that_is_right_but_holds_no_ffmpeg_says_so() {
        use sha2::{Digest, Sha256};
        let dir = temp("nobinary");
        // a valid archive with something else in it
        let mut zip_bytes = Vec::new();
        {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_bytes));
            z.start_file("pkg/readme.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"hello").unwrap();
            z.finish().unwrap();
        }
        let sum = format!("{:x}", Sha256::digest(&zip_bytes));
        let address = serve(vec![
            ("/b.zip", zip_bytes),
            ("/b.zip.sha256", sum.into_bytes()),
        ]);
        let m = Arc::new(Modules::new(&dir).without_system().with_urls(Some(Urls {
            archive: format!("http://{address}/b.zip"),
            sha256: format!("http://{address}/b.zip.sha256"),
        })));
        m.install(FFMPEG).unwrap();
        let info = wait_installed(&m);
        assert!(info
            .error
            .unwrap()
            .contains("not in the downloaded archive"));
        assert!(!info.enabled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_system_without_a_download_source_says_how_to_get_ffmpeg() {
        let dir = temp("nosource");
        let m = Arc::new(Modules::new(&dir).without_system().with_urls(None));
        let e = m.install(FFMPEG).unwrap_err();
        assert!(e.contains("package manager"), "{e}");
        assert!(!m.list()[0].downloadable);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_is_switched_on_is_there_after_a_restart_and_removing_switches_it_off() {
        let dir = temp("persist");
        {
            let m = Modules::new(&dir);
            // pretend a copy exists: a program that answers `-version` like FFmpeg is not needed to test the saving
            m.save(&Saved {
                ffmpeg_enabled: true,
            });
        }
        let m = Modules::new(&dir);
        assert!(m.list()[0].enabled, "remembered");
        m.remove(FFMPEG).unwrap();
        assert!(!m.list()[0].enabled);
        assert!(
            !Modules::new(&dir).list()[0].enabled,
            "and that is remembered too"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
