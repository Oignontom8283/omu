//! Optional modules: components that are not part of the program and that people switch on when they want them.
//! FFmpeg (see [`ffmpeg`]) reads the files the program cannot; DeepFilterNet (see [`deepfilter`]) is a larger noise
//! reduction model.
//!
//! Nothing here reaches the Internet by itself: a download starts only when a person asks for the module, and nothing
//! but the request for the files is sent.

pub mod deepfilter;
pub mod download;
pub mod ffmpeg;

use download::Counters;
use ffmpeg::{Found, Urls};
use omu_types::{ModuleInfo, Progress};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The id of the FFmpeg module.
pub const FFMPEG: &str = "ffmpeg";
/// The id of the DeepFilterNet module.
pub const DEEPFILTER: &str = "deepfilter";

/// What is kept between runs (`modules.json` in the data folder).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    ffmpeg_enabled: bool,
    #[serde(default)]
    deepfilter_enabled: bool,
}

impl Saved {
    fn enabled_mut(&mut self, id: &str) -> &mut bool {
        if id == DEEPFILTER {
            &mut self.deepfilter_enabled
        } else {
            &mut self.ffmpeg_enabled
        }
    }
}

#[derive(Default)]
struct State {
    saved: Saved,
    /// The module being downloaded (one at a time: they share the progress counters).
    installing: Option<&'static str>,
    ffmpeg_error: Option<String>,
    deepfilter_error: Option<String>,
    /// What was found last, and when (looking runs FFmpeg, so it is not done on every question).
    located: Option<(Instant, Option<Found>)>,
}

impl State {
    fn error_mut(&mut self, id: &str) -> &mut Option<String> {
        if id == DEEPFILTER {
            &mut self.deepfilter_error
        } else {
            &mut self.ffmpeg_error
        }
    }
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
    /// Where the DeepFilterNet model is downloaded from, and the SHA-256 it must have.
    model_source: Mutex<(String, String)>,
    /// Whether an FFmpeg the system already has may be used (tests turn it off).
    use_system: bool,
    /// Whose certificates a download believes.
    trust: Mutex<omu_tls::Trust>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The id as the `'static` name the state keeps.
fn known(id: &str) -> Result<&'static str, String> {
    match id {
        FFMPEG => Ok(FFMPEG),
        DEEPFILTER => Ok(DEEPFILTER),
        _ => Err(format!("there is no module {id:?}")),
    }
}

impl Modules {
    pub fn new(data_dir: &std::path::Path) -> Modules {
        let saved = std::fs::read_to_string(data_dir.join("modules.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Modules {
            data_dir: data_dir.to_owned(),
            trust: Mutex::new(omu_tls::Trust {
                mode: omu_types::TrustMode::System,
                folder: data_dir.join(omu_tls::FOLDER_NAME),
            }),
            dir: data_dir.join("modules"),
            state: Mutex::new(State {
                saved,
                ..State::default()
            }),
            counters: Counters::default(),
            urls: Mutex::new(ffmpeg::default_urls()),
            model_source: Mutex::new((
                deepfilter::MODEL_URL.into(),
                deepfilter::MODEL_SHA256.into(),
            )),
            use_system: true,
        }
    }

    /// Where the modules are kept.
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Whose certificates the next downloads believe.
    pub fn set_trust(&self, trust: omu_tls::Trust) {
        *lock(&self.trust) = trust;
    }

    /// Never uses an FFmpeg the system has (tests).
    pub fn without_system(mut self) -> Modules {
        self.use_system = false;
        self
    }

    /// Downloads FFmpeg from other addresses (tests).
    pub fn with_urls(self, urls: Option<Urls>) -> Modules {
        *lock(&self.urls) = urls;
        self
    }

    /// Downloads the DeepFilterNet model from another address, with the SHA-256 it must have (tests).
    pub fn with_model_source(self, url: &str, sha256: &str) -> Modules {
        *lock(&self.model_source) = (url.to_owned(), sha256.to_owned());
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

    /// The DeepFilterNet model to use: only when the module is switched on and the model was downloaded.
    pub fn deepfilter_model(&self) -> Option<PathBuf> {
        if !lock(&self.state).saved.deepfilter_enabled {
            return None;
        }
        let path = deepfilter::model_path(&self.dir);
        path.is_file().then_some(path)
    }

    /// The modules and how each stands.
    pub fn list(&self) -> Vec<ModuleInfo> {
        let found = self.found();
        let model = deepfilter::model_path(&self.dir).is_file();
        let st = lock(&self.state);
        let progress = |id: &str| {
            (st.installing == Some(id)).then(|| Progress {
                done: self.counters.done.load(Ordering::Relaxed),
                total: self.counters.total.load(Ordering::Relaxed),
            })
        };
        vec![
            ModuleInfo {
                id: FFMPEG.into(),
                name: "FFmpeg".into(),
                description: "Reads the sound of many more files and videos (for example Opus sound in WebM videos). \
                              Used only when a file cannot be read without it."
                    .into(),
                enabled: st.saved.ffmpeg_enabled,
                available: found.is_some(),
                source: found.as_ref().map(|f| f.source.to_owned()),
                version: found.as_ref().map(|f| f.version.clone()),
                installing: progress(FFMPEG),
                error: st.ffmpeg_error.clone(),
                downloadable: lock(&self.urls).is_some(),
                download_mb: ffmpeg::DOWNLOAD_MB,
                supported: true,
            },
            ModuleInfo {
                id: DEEPFILTER.into(),
                name: "DeepFilterNet".into(),
                description: "A larger noise reduction than the built-in one: a neural network that keeps the voice and \
                              takes out most other sounds. Used by the \"Deep noise reduction\" block; it needs a \
                              little processor time."
                    .into(),
                enabled: st.saved.deepfilter_enabled,
                available: model,
                source: model.then(|| "local".to_owned()),
                version: model.then(|| "DeepFilterNet 3".to_owned()),
                installing: progress(DEEPFILTER),
                error: st.deepfilter_error.clone(),
                downloadable: true,
                download_mb: deepfilter::DOWNLOAD_MB,
                supported: cfg!(feature = "deepfilter"),
            },
        ]
    }

    /// Switches a module on or off. It cannot be switched on while nothing usable is there (install it instead).
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let id = known(id)?;
        let available = if id == DEEPFILTER {
            deepfilter::model_path(&self.dir).is_file()
        } else {
            self.found().is_some()
        };
        if enabled && !available {
            return Err(format!("{id} is not installed: install the module first"));
        }
        let saved = {
            let mut st = lock(&self.state);
            *st.saved.enabled_mut(id) = enabled;
            st.saved.clone()
        };
        self.save(&saved);
        Ok(())
    }

    /// Switches the module on. When what it needs is already there (a compatible FFmpeg on the system, the model on
    /// the disk) that is all; otherwise it is downloaded into the modules folder in the background (follow it with
    /// [`Modules::list`]).
    pub fn install(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let id = known(id)?;
        self.forget_location();
        let there = if id == DEEPFILTER {
            deepfilter::model_path(&self.dir).is_file()
        } else {
            self.found().is_some()
        };
        if there {
            return self.set_enabled(id, true);
        }
        if id == FFMPEG && lock(&self.urls).is_none() {
            return Err(
                "FFmpeg is not installed, and it cannot be downloaded on this system: install it with the system's \
                 package manager (for example `apt install ffmpeg` or `brew install ffmpeg`)"
                    .into(),
            );
        }
        {
            let mut st = lock(&self.state);
            match st.installing {
                Some(running) if running == id => return Ok(()),
                Some(other) => {
                    return Err(format!(
                        "{other} is being downloaded: wait for it to finish"
                    ))
                }
                None => {}
            }
            st.installing = Some(id);
            *st.error_mut(id) = None;
            // a stop asked for before the thread has started must not be forgotten: reset it here, not there
            self.counters.cancel.store(false, Ordering::Relaxed);
        }
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("module-download".into())
            .spawn(move || {
                tracing::info!(module = id, "downloading a module");
                // a failure of the download must never take the daemon down with it
                let result =
                    omu_diagnostics::crash::contained(|| me.download(id)).unwrap_or_else(|_| {
                        Err("the download failed unexpectedly (see the logs)".into())
                    });
                me.forget_location();
                let saved = {
                    let mut st = lock(&me.state);
                    st.installing = None;
                    match &result {
                        Ok(()) => {
                            *st.error_mut(id) = None;
                            *st.saved.enabled_mut(id) = true;
                        }
                        Err(e) if e == download::CANCELLED => {
                            tracing::info!(module = id, "the download was cancelled");
                            *st.error_mut(id) = None;
                        }
                        Err(e) => {
                            tracing::warn!(module = id, "the module could not be installed: {e}");
                            *st.error_mut(id) = Some(e.clone());
                        }
                    }
                    st.saved.clone()
                };
                me.save(&saved);
            });
        if let Err(e) = spawned {
            lock(&self.state).installing = None;
            return Err(format!("cannot start the download: {e}"));
        }
        Ok(())
    }

    /// The download of one module (on the download thread).
    fn download(&self, id: &str) -> Result<(), String> {
        let agent = omu_tls::agent(&lock(&self.trust).clone()).map_err(|e| e.to_string())?;
        if id == DEEPFILTER {
            let (url, sha256) = lock(&self.model_source).clone();
            deepfilter::download(&self.dir, &url, &sha256, &self.counters, &agent)
        } else {
            let urls = lock(&self.urls)
                .clone()
                .ok_or("FFmpeg cannot be downloaded on this system")?;
            ffmpeg::download(&self.dir, &urls, &self.counters, &agent)
        }
    }

    /// Stops a download that is running (what was downloaded so far is thrown away).
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        known(id)?;
        self.counters.cancel.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Switches the module off and deletes its copy in the modules folder (a copy the system has is not touched).
    pub fn remove(&self, id: &str) -> Result<(), String> {
        let id = known(id)?;
        if let Some(running) = lock(&self.state).installing {
            return Err(format!(
                "{running} is being downloaded: wait for it to finish"
            ));
        }
        let saved = {
            let mut st = lock(&self.state);
            *st.saved.enabled_mut(id) = false;
            *st.error_mut(id) = None;
            st.saved.clone()
        };
        self.save(&saved);
        let folder = self.dir.join(id);
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
                ..Default::default()
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

    /// A server whose file never finishes: it sends the sum file at once, and the archive a little at a time, forever.
    fn serve_forever() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                std::thread::spawn(move || {
                    let mut request = [0u8; 2048];
                    let n = stream.read(&mut request).unwrap_or(0);
                    let first = String::from_utf8_lossy(&request[..n]).to_string();
                    let path = first.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    if path.ends_with(".sha256") {
                        let body = "0".repeat(64);
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(head.as_bytes());
                        return;
                    }
                    let head = "HTTP/1.1 200 OK\r\nContent-Length: 1000000000\r\nConnection: close\r\n\r\n";
                    if stream.write_all(head.as_bytes()).is_err() {
                        return;
                    }
                    let chunk = [7u8; 4096];
                    while stream.write_all(&chunk).is_ok() {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                });
            }
        });
        address
    }

    #[test]
    fn a_download_can_be_cancelled_and_leaves_nothing_and_no_error() {
        let dir = temp("cancel");
        let address = serve_forever();
        let m = Arc::new(Modules::new(&dir).without_system().with_urls(Some(Urls {
            archive: format!("http://{address}/big.zip"),
            sha256: format!("http://{address}/big.zip.sha256"),
        })));
        m.install(FFMPEG).unwrap();
        // it is running and making progress
        let mut progressing = false;
        for _ in 0..100 {
            if m.list()[0].installing.is_some_and(|p| p.done > 0) {
                progressing = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(progressing, "the download started");
        m.cancel(FFMPEG).unwrap();
        let info = wait_installed(&m);
        assert!(
            info.error.is_none(),
            "a cancelled download is not a failure: {:?}",
            info.error
        );
        assert!(!info.enabled && !info.available);
        assert!(!ffmpeg::local_exe(m.dir()).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn wait_for(m: &Arc<Modules>, id: &str) -> ModuleInfo {
        for _ in 0..600 {
            let info = m.list().into_iter().find(|i| i.id == id).unwrap();
            if info.installing.is_none() {
                return info;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the download never ended");
    }

    #[test]
    fn the_model_is_installed_only_when_its_checksum_is_the_expected_one_and_can_be_removed() {
        use sha2::{Digest, Sha256};
        let dir = temp("model");
        let model: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let sum = format!("{:x}", Sha256::digest(&model));
        let address = serve(vec![("/model.tar.gz", model.clone())]);
        let url = format!("http://{address}/model.tar.gz");

        // a model that is not the one that was checked is refused and leaves nothing
        let wrong = Arc::new(
            Modules::new(&dir)
                .without_system()
                .with_model_source(&url, &"0".repeat(64)),
        );
        wrong.install(DEEPFILTER).unwrap();
        let info = wait_for(&wrong, DEEPFILTER);
        assert!(
            info.error
                .as_deref()
                .is_some_and(|e| e.contains("checksum")),
            "{:?}",
            info.error
        );
        assert!(!info.available && !info.enabled);
        assert_eq!(wrong.deepfilter_model(), None);
        assert!(!deepfilter::model_path(wrong.dir()).exists());

        // the right one is installed, switched on, and found
        let m = Arc::new(
            Modules::new(&dir)
                .without_system()
                .with_model_source(&url, &sum),
        );
        m.install(DEEPFILTER).unwrap();
        let info = wait_for(&m, DEEPFILTER);
        assert!(info.error.is_none(), "{:?}", info.error);
        assert!(info.available && info.enabled && info.source.as_deref() == Some("local"));
        let path = m.deepfilter_model().expect("the model");
        assert_eq!(std::fs::read(&path).unwrap(), model);
        // FFmpeg is a separate module: nothing of it changed
        let ffmpeg = m.list().into_iter().find(|i| i.id == FFMPEG).unwrap();
        assert!(!ffmpeg.enabled);
        // after a restart it is still switched on
        assert!(Modules::new(&dir)
            .without_system()
            .deepfilter_model()
            .is_some());
        // switched off it is not used; removed it is gone
        m.set_enabled(DEEPFILTER, false).unwrap();
        assert_eq!(m.deepfilter_model(), None);
        m.set_enabled(DEEPFILTER, true).unwrap();
        m.remove(DEEPFILTER).unwrap();
        assert_eq!(m.deepfilter_model(), None);
        assert!(!path.exists());
        assert!(
            m.set_enabled(DEEPFILTER, true).is_err(),
            "nothing to switch on"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_one_module_downloads_at_a_time() {
        let dir = temp("one-at-a-time");
        let address = serve_forever();
        let m = Arc::new(
            Modules::new(&dir)
                .without_system()
                .with_urls(Some(Urls {
                    archive: format!("http://{address}/big.zip"),
                    sha256: format!("http://{address}/big.zip.sha256"),
                }))
                .with_model_source(&format!("http://{address}/model"), &"0".repeat(64)),
        );
        m.install(FFMPEG).unwrap();
        let e = m.install(DEEPFILTER).unwrap_err();
        assert!(e.contains("being downloaded"), "{e}");
        assert!(
            m.install(FFMPEG).is_ok(),
            "asking again for the same one is harmless"
        );
        m.cancel(FFMPEG).unwrap();
        wait_installed(&m);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
