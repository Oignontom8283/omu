//! Downloading a file for a module: pieces that resume (the HTTPS client comes from `omu_tls`, with the certificates the user trusts),
//! progress that can be shown, a way to stop, and the SHA-256 of what was written.

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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

/// How much is asked for at a time (a stalled connection then costs one piece, which is asked for again).
const PIECE: u64 = 8 * 1024 * 1024;
/// How long one piece may take, and how many times in a row a piece may fail before the download is given up.
const PIECE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const FAILURES_ALLOWED: u32 = 6;

/// Downloads `url` into `file`, piece by piece when the server allows asking for a range (so that a stalled or broken
/// connection resumes where it stopped instead of ending the download), and returns the SHA-256 of what was written.
pub(crate) fn fetch(
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

pub(crate) fn host_of(url: &str) -> &str {
    url.split("//")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
}
