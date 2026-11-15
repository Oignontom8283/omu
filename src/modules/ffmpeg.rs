//! The FFmpeg module: finding FFmpeg, downloading it into the modules folder, and using it to read sound files the
//! program cannot read itself (Opus in a WebM video, and many formats that are rare).
//!
//! FFmpeg is only ever run as a separate program with fixed arguments, never linked in.

use crate::soundboard::wav::WavWriter;
use crate::types::SoundInfo;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The first major version that is used (the options the program passes have not changed since).
pub const MIN_MAJOR: u32 = 4;

/// Where a usable FFmpeg was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub version: String,
    /// "local" (the modules folder) or "system".
    pub source: &'static str,
}

/// Where the module's own copy lives inside the modules folder.
pub fn local_exe(modules_dir: &Path) -> PathBuf {
    modules_dir.join("ffmpeg").join(if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    })
}

/// A command for a program, without a console window flashing up on Windows.
fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// The version FFmpeg at `path` says it is (`9.0.2`, `7.1-essentials_build`, `N-12345-g...`), or `None` when it does
/// not run or is not FFmpeg.
pub fn version_of(path: &Path) -> Option<String> {
    let out = command(path)
        .arg("-version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

/// The version in the first line of `ffmpeg -version`: `ffmpeg version 9.0.2-essentials_build-www.gyan.dev ...`.
pub fn parse_version(output: &str) -> Option<String> {
    let line = output.lines().next()?;
    let mut words = line.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("ffmpeg") || words.next()? != "version" {
        return None;
    }
    words.next().map(str::to_owned)
}

/// Whether a version can be used: a release from 4 on, or a build made from the development branch (`N-123-g...`,
/// `git-2025-...`), which is recent by nature.
pub fn is_compatible(version: &str) -> bool {
    let v = version.trim_start_matches(['n', 'N']);
    if version.starts_with("N-") || version.starts_with("n-") || version.starts_with("git-") {
        return true;
    }
    v.split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|major| major.parse::<u32>().ok())
        .is_some_and(|major| major >= MIN_MAJOR)
}

/// FFmpeg in the folders of the `PATH`, searched by hand (starting the bare name would also look in the current folder
/// on Windows, which is not a place to run programs from).
fn in_path() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// The FFmpeg to use: the copy in the modules folder first, else one the system already has.
pub fn locate(modules_dir: &Path, use_system: bool) -> Option<Found> {
    let local = local_exe(modules_dir);
    if local.is_file() {
        if let Some(v) = version_of(&local).filter(|v| is_compatible(v)) {
            return Some(Found {
                path: local,
                version: v,
                source: "local",
            });
        }
    }
    if !use_system {
        return None;
    }
    let system = in_path()?;
    let version = version_of(&system).filter(|v| is_compatible(v))?;
    Some(Found {
        path: system,
        version,
        source: "system",
    })
}

/// Reads the sound of any file FFmpeg can read and stores it like any other imported sound (48 kHz stereo, 16 bits).
/// The decoded sound goes straight from FFmpeg's output into the stored file: nothing long is held in memory.
pub fn import(
    ffmpeg: &Path,
    source: &Path,
    destination: &Path,
    id: &str,
    name: &str,
) -> Result<SoundInfo, String> {
    if let Some(dir) = destination.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let result = run_import(ffmpeg, source, destination);
    match result {
        Ok(frames) => Ok(SoundInfo {
            id: id.to_owned(),
            name: name.to_owned(),
            channels: 2,
            frames,
            bytes: std::fs::metadata(destination).map_or(0, |m| m.len()),
            compressed: false,
        }),
        Err(e) => {
            let _ = std::fs::remove_file(destination);
            Err(e)
        }
    }
}

fn run_import(ffmpeg: &Path, source: &Path, destination: &Path) -> Result<u64, String> {
    let mut child = command(ffmpeg)
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(source)
        // the first sound track only: no picture, no subtitles, no data
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn"])
        .args(["-ac", "2", "-ar", "48000", "-f", "s16le", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run FFmpeg: {e}"))?;
    // what FFmpeg complains about, read on the side so that a full pipe never stops it
    let mut stderr = child.stderr.take().expect("piped");
    let complaints = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = (&mut stderr).take(8192).read_to_string(&mut text);
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
        text
    });
    let mut stdout = child.stdout.take().expect("piped");
    let mut writer = WavWriter::create(destination, 2).map_err(|e| e.to_string())?;
    let mut bytes = vec![0u8; 1 << 16];
    let mut kept = 0usize; // bytes at the start of `bytes` that do not make a whole frame yet
    let mut samples: Vec<i16> = Vec::with_capacity(bytes.len() / 2);
    let copied: Result<(), String> = loop {
        let n = match stdout.read(&mut bytes[kept..]) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) => break Err(e.to_string()),
        };
        let have = kept + n;
        let whole = have - have % 4;
        samples.clear();
        samples.extend(
            bytes[..whole]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b)),
        );
        if let Err(e) = writer.write(&samples) {
            break Err(e.to_string());
        }
        bytes.copy_within(whole..have, 0);
        kept = have - whole;
    };
    if copied.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let complaint = complaints.join().unwrap_or_default();
    copied?;
    let frames = writer.frames();
    if !status.success() || frames == 0 {
        return Err(describe_failure(&complaint));
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(frames)
}

/// What FFmpeg said, in a form people can read.
fn describe_failure(stderr: &str) -> String {
    if stderr.contains("matches no streams") {
        return "there is no sound in this file".into();
    }
    let line = stderr
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("it could not be read");
    format!("FFmpeg could not read this file: {line}")
}

/// Where a download comes from.
#[derive(Debug, Clone)]
pub struct Urls {
    pub archive: String,
    /// A text file holding the archive's SHA-256, as hexadecimal.
    pub sha256: String,
}

/// The official source of FFmpeg for this system, when there is one to download from: for Windows, the builds that
/// ffmpeg.org's own download page points to (gyan.dev). Elsewhere FFmpeg comes from the system's package manager.
pub fn default_urls() -> Option<Urls> {
    cfg!(windows).then(|| Urls {
        archive: "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip".into(),
        sha256: "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256".into(),
    })
}

/// About how big the download is, in megabytes.
pub const DOWNLOAD_MB: u32 = 115;

/// How far a download is (read by whoever shows it).
#[derive(Debug, Default)]
pub struct Counters {
    pub done: AtomicU64,
    pub total: AtomicU64,
    /// Set to stop the download (it is checked between reads).
    pub cancel: AtomicBool,
}

/// What a download that was stopped by the user returns as its error (it is not a failure to report).
pub const CANCELLED: &str = "cancelled";

fn agent() -> ureq::Agent {
    // rustls: the same pure-Rust TLS on every system (nothing to install on Linux), trusting the certificates the
    // operating system trusts
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::Rustls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .timeout_global(None)
        .timeout_connect(Some(std::time::Duration::from_secs(30)))
        // the answer must start within a minute; the body then takes as long as the connection needs
        .timeout_recv_response(Some(std::time::Duration::from_secs(60)))
        .build()
        .into()
}

/// Downloads FFmpeg into `<modules_dir>/ffmpeg/`: the archive is checked against its published SHA-256 before anything
/// is taken out of it, and only the program and its licence are kept.
pub fn download(modules_dir: &Path, urls: &Urls, counters: &Counters) -> Result<(), String> {
    let dir = modules_dir.join("ffmpeg");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let agent = agent();
    counters.done.store(0, Ordering::Relaxed);
    counters.total.store(0, Ordering::Relaxed);
    counters.cancel.store(false, Ordering::Relaxed);

    let wanted = agent
        .get(&urls.sha256)
        .call()
        .map_err(|e| format!("cannot reach {}: {e}", host_of(&urls.sha256)))?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let wanted = wanted
        .split_whitespace()
        .next()
        .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or("the published checksum is not a SHA-256")?
        .to_ascii_lowercase();

    let archive = dir.join("download.zip.part");
    let result = (|| -> Result<(), String> {
        let hex = fetch(&agent, &urls.archive, &archive, counters)?;
        if hex != wanted {
            return Err(
                "the downloaded file does not match its published checksum: it was not installed"
                    .into(),
            );
        }
        extract(&archive, &dir)
    })();
    let _ = std::fs::remove_file(&archive);
    result?;
    let exe = local_exe(modules_dir);
    match version_of(&exe) {
        Some(v) if is_compatible(&v) => Ok(()),
        _ => {
            let _ = std::fs::remove_file(&exe);
            Err("the downloaded FFmpeg does not run on this computer".into())
        }
    }
}

/// How much is asked for at a time (a stalled connection then costs one piece, which is asked for again).
const PIECE: u64 = 8 * 1024 * 1024;
/// How long one piece may take, and how many times in a row a piece may fail before the download is given up.
const PIECE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const FAILURES_ALLOWED: u32 = 6;

/// Downloads `url` into `file`, piece by piece when the server allows asking for a range (so that a stalled or broken
/// connection resumes where it stopped instead of ending the download), and returns the SHA-256 of what was written.
fn fetch(
    agent: &ureq::Agent,
    url: &str,
    file: &Path,
    counters: &Counters,
) -> Result<String, String> {
    let mut out = std::fs::File::create(file).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    let mut failures = 0u32;
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        if counters.cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let mut response = match agent
            .get(url)
            .header("Range", format!("bytes={done}-{}", done + PIECE - 1))
            .config()
            .timeout_global(Some(PIECE_TIMEOUT))
            .build()
            .call()
        {
            Ok(r) => r,
            Err(e) => {
                failures += 1;
                if failures >= FAILURES_ALLOWED {
                    return Err(format!("cannot download FFmpeg from {}: {e}", host_of(url)));
                }
                std::thread::sleep(std::time::Duration::from_secs(2 * failures as u64));
                continue;
            }
        };
        let status = response.status().as_u16();
        let ranged = status == 206;
        let total: u64;
        if ranged {
            // "bytes 0-8388607/114768076"
            total = response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
        } else {
            // the server sends the whole file whatever is asked: take it as it comes, from the start
            total = response
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if done != 0 {
                use std::io::Seek;
                out.set_len(0).map_err(|e| e.to_string())?;
                out.rewind().map_err(|e| e.to_string())?;
                hasher = Sha256::new();
                done = 0;
                counters.done.store(0, Ordering::Relaxed);
            }
        }
        counters.total.store(total, Ordering::Relaxed);
        let before = done;
        let mut body = response.body_mut().as_reader();
        let broke = loop {
            if counters.cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            match body.read(&mut buffer) {
                Ok(0) => break false,
                Ok(n) => {
                    hasher.update(&buffer[..n]);
                    out.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                    done += n as u64;
                    counters.done.store(done, Ordering::Relaxed);
                }
                Err(_) => break true,
            }
        };
        if done > before {
            failures = 0;
        }
        if broke {
            failures += 1;
            if failures >= FAILURES_ALLOWED {
                return Err(format!(
                    "the download of FFmpeg stopped at {} MB and could not be resumed",
                    done / (1024 * 1024)
                ));
            }
            if !ranged {
                // without ranges there is no resuming: start over
                use std::io::Seek;
                out.set_len(0).map_err(|e| e.to_string())?;
                out.rewind().map_err(|e| e.to_string())?;
                hasher = Sha256::new();
                done = 0;
                counters.done.store(0, Ordering::Relaxed);
            }
            continue;
        }
        if !ranged || (total > 0 && done >= total) {
            break;
        }
        if done == before {
            // a piece that brought nothing and did not fail: the server has nothing more
            break;
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn host_of(url: &str) -> &str {
    url.split("//")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
}

/// Takes `bin/ffmpeg` and the licence out of the archive (written under another name until complete).
fn extract(archive: &Path, dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("the download is not a valid archive: {e}"))?;
    let exe_name = if cfg!(windows) {
        "bin/ffmpeg.exe"
    } else {
        "bin/ffmpeg"
    };
    let mut found_exe = false;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?.replace('\\', "/");
        let target = if name.ends_with(exe_name) && name.matches('/').count() <= 2 {
            found_exe = true;
            dir.join(if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            })
        } else if name.ends_with("/LICENSE") || name == "LICENSE" {
            dir.join("LICENSE.txt")
        } else {
            continue;
        };
        let partial = target.with_extension("part");
        let mut out = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        drop(out);
        std::fs::rename(&partial, &target).map_err(|e| e.to_string())?;
    }
    if found_exe {
        Ok(())
    } else {
        Err("FFmpeg is not in the downloaded archive".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_read_from_the_first_line_and_old_or_unknown_ones_are_refused() {
        let out = "ffmpeg version 9.0.2-essentials_build-www.gyan.dev Copyright (c) 2000-2025 the FFmpeg developers\nbuilt with gcc";
        assert_eq!(
            parse_version(out).as_deref(),
            Some("9.0.2-essentials_build-www.gyan.dev")
        );
        assert_eq!(
            parse_version("ffmpeg version 4.4.2-0ubuntu0 Copyright").as_deref(),
            Some("4.4.2-0ubuntu0")
        );
        assert_eq!(parse_version("something else entirely"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("ffmpeg"), None);

        for good in [
            "9.0.2-essentials_build",
            "4.0",
            "7.1",
            "N-118000-gabcdef",
            "git-2025-01-01-abc",
            "10.2",
        ] {
            assert!(is_compatible(good), "{good}");
        }
        for bad in ["3.4.11", "2.8", "", "abc", "0.9"] {
            assert!(!is_compatible(bad), "{bad}");
        }
    }

    #[test]
    fn a_failure_is_described_in_words() {
        assert_eq!(
            describe_failure("Stream map '0:a:0' matches no streams.\n"),
            "there is no sound in this file"
        );
        assert!(
            describe_failure("x.mp4: Invalid data found when processing input\n")
                .contains("Invalid data found")
        );
        assert!(describe_failure("").contains("could not be read"));
    }

    #[test]
    fn the_host_of_an_address_is_what_a_message_names() {
        assert_eq!(host_of("https://www.gyan.dev/ffmpeg/x.zip"), "www.gyan.dev");
        assert_eq!(host_of("http://127.0.0.1:8080/a"), "127.0.0.1:8080");
    }

    #[test]
    fn nothing_found_in_an_empty_modules_folder_when_the_system_has_none() {
        // the local copy is looked for first, and a folder with nothing in it holds none
        let dir = std::env::temp_dir().join(format!("omu-ffmpeg-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!local_exe(&dir).exists());
        assert_eq!(
            locate(&dir, false),
            None,
            "nothing local, and the system is not asked"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
