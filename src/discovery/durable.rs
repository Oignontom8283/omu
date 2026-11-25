//! Files the program keeps in its data folder (the graph, the soundboards, the settings), written so that nothing the user made is lost
//! by a crash, a power cut or a file that cannot be read.
//!
//! * **Writing** puts the new version in a temporary file of its own, flushes it to the disk, keeps the version it replaces as a
//!   numbered backup (`name.bak1` is the latest), then renames the new one over the old: a crash leaves the old file or the new one,
//!   never half of each. When another program holds the file open for a moment (an antivirus, a backup), the rename is tried again.
//! * **Reading** a file that cannot be understood (damaged, or written by a newer version that has blocks this one does not know)
//!   does not throw it away: it is renamed `name.bad-<seconds>` so that the next save cannot destroy it, a warning says so, and the
//!   latest backup that can be read is used instead.

use serde::de::DeserializeOwned;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static COUNTER: AtomicU64 = AtomicU64::new(0);
/// How many files that could not be read are kept next to the good one (the oldest are deleted).
const KEEP_BAD: usize = 5;
/// The folder (next to a file) that its earlier versions are kept in.
const BACKUPS: &str = "backups";

fn name_of(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned())
}

fn beside(path: &Path, suffix: &str) -> PathBuf {
    path.with_file_name(format!("{}{suffix}", name_of(path)))
}

/// Where an earlier version of `path` (`.bak1`) or a copy that could not be read (`.bad-...`) is kept: a `backups` folder next to the
/// file, so that the folder of the file itself holds only the files in use.
fn kept(path: &Path, suffix: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let folder = if parent.file_name().is_some_and(|n| n == BACKUPS) {
        parent.to_owned()
    } else {
        parent.join(BACKUPS)
    };
    folder.join(format!("{}{suffix}", name_of(path)))
}

/// Replaces `path` by `tmp`, trying again a few times: on Windows another program that has the file open without allowing it to be
/// replaced makes the rename fail for a moment.
fn replace(tmp: &Path, path: &Path) -> io::Result<()> {
    let mut last = None;
    for attempt in 0..6u64 {
        match std::fs::rename(tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(30 * (attempt + 1)));
            }
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("cannot replace the file")))
}

/// Keeps the file as it is now as the latest backup, the older backups moving one place down. Nothing is kept when the new content is
/// the same (saving what is already there is not a new version) or when the latest backup already holds the current content.
fn rotate(path: &Path, backups: usize, new: &[u8]) {
    let Ok(current) = std::fs::read(path) else {
        return;
    };
    let latest = kept(path, ".bak1");
    if let Some(folder) = latest.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    if current == new || std::fs::read(&latest).is_ok_and(|b| b == current) {
        return;
    }
    for n in (1..backups).rev() {
        let _ = std::fs::rename(
            kept(path, &format!(".bak{n}")),
            kept(path, &format!(".bak{}", n + 1)),
        );
    }
    let _ = std::fs::write(&latest, current);
}

/// Writes `bytes` as the new content of `path`, durably, keeping `backups` earlier versions (0: none).
pub fn write(path: &Path, bytes: &[u8], backups: usize) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = beside(
        path,
        &format!(
            ".{}.{}.tmp",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ),
    );
    let written = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        // on the disk, not only in the memory of the system, before the rename makes it the file
        file.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if backups > 0 && path.is_file() {
        rotate(path, backups, bytes);
    }
    let result = replace(&tmp, path);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Writes `value` as JSON (see [`write`]).
pub fn write_json<T: serde::Serialize>(path: &Path, value: &T, backups: usize) -> io::Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    write(path, text.as_bytes(), backups)
}

/// Sets a file that cannot be understood aside, so that nothing writes over it; returns where it went.
fn set_aside(path: &Path) -> Option<PathBuf> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let aside = kept(path, &format!(".bad-{seconds}"));
    if let Some(folder) = aside.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    std::fs::rename(path, &aside).ok()?;
    // only the latest few: a file that is read wrongly at every start must not fill the disk
    let prefix = format!("{}.bad-", name_of(path));
    if let Some(dir) = aside.parent().and_then(|d| std::fs::read_dir(d).ok()) {
        let mut bad: Vec<PathBuf> = dir
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| name_of(p).starts_with(&prefix))
            .collect();
        bad.sort();
        let extra = bad.len().saturating_sub(KEEP_BAD);
        for old in bad.into_iter().take(extra) {
            let _ = std::fs::remove_file(old);
        }
    }
    Some(aside)
}

/// What reading a JSON file gave.
#[derive(Debug, PartialEq)]
pub enum Loaded<T> {
    /// There is no such file (the first run).
    Missing,
    Value(T),
    /// The file is there and cannot be understood: it was set aside as `kept_as` (when that could be done).
    Unreadable {
        kept_as: Option<PathBuf>,
        reason: String,
    },
}

/// Reads the JSON file at `path`.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Loaded<T> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Loaded::Missing,
        // not readable now (rights, a lock): it may be fine later, so it is left where it is
        Err(e) => {
            return Loaded::Unreadable {
                kept_as: None,
                reason: e.to_string(),
            }
        }
    };
    match serde_json::from_str(&text) {
        Ok(v) => Loaded::Value(v),
        Err(e) => Loaded::Unreadable {
            kept_as: set_aside(path),
            reason: e.to_string(),
        },
    }
}

/// The content of the JSON file at `path`; when it cannot be understood, the latest of its `backups` that can; `None` when there is
/// nothing usable (a file that could not be read is set aside and a warning says where, so it is never silently overwritten).
pub fn load_json<T: DeserializeOwned>(path: &Path, backups: usize) -> Option<T> {
    match read_json::<T>(path) {
        Loaded::Missing => return None,
        Loaded::Value(v) => return Some(v),
        Loaded::Unreadable { kept_as, reason } => {
            tracing::warn!(
                "{} cannot be read ({reason}); {}",
                path.display(),
                kept_as.map_or_else(
                    || "it was left where it is".to_owned(),
                    |k| format!("it was kept as {}", k.display())
                )
            );
        }
    }
    for n in 1..=backups {
        let backup = kept(path, &format!(".bak{n}"));
        if let Loaded::Value(v) = read_json::<T>(&backup) {
            tracing::warn!("the backup {} is used instead", backup.display());
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Doc {
        n: u32,
    }

    fn folder(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omu-durable-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_save_replaces_the_file_keeps_the_previous_versions_and_leaves_no_temporary_file() {
        let dir = folder("write");
        let path = dir.join("state.json");
        for n in 1..=4 {
            write_json(&path, &Doc { n }, 2).unwrap();
        }
        assert_eq!(load_json::<Doc>(&path, 2), Some(Doc { n: 4 }));
        assert_eq!(
            read_json::<Doc>(&kept(&path, ".bak1")),
            Loaded::Value(Doc { n: 3 }),
            "the latest backup is the version before"
        );
        assert_eq!(
            read_json::<Doc>(&kept(&path, ".bak2")),
            Loaded::Value(Doc { n: 2 })
        );
        assert!(!kept(&path, ".bak3").exists(), "only two are kept");
        // the same content again is not a new backup
        write_json(&path, &Doc { n: 4 }, 2).unwrap();
        assert_eq!(
            read_json::<Doc>(&kept(&path, ".bak1")),
            Loaded::Value(Doc { n: 3 })
        );
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| name_of(&e.unwrap().path()))
            .collect();
        assert!(!names.iter().any(|n| n.ends_with(".tmp")), "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_cannot_be_read_is_kept_aside_and_the_latest_good_backup_is_used() {
        let dir = folder("bad");
        let path = dir.join("state.json");
        write_json(&path, &Doc { n: 1 }, 3).unwrap();
        write_json(&path, &Doc { n: 2 }, 3).unwrap();
        // a power cut, or a version that wrote something this one does not know
        std::fs::write(&path, "{ \"n\": ").unwrap();
        assert_eq!(load_json::<Doc>(&path, 3), Some(Doc { n: 1 }), "the backup");
        assert!(
            !path.exists(),
            "the damaged file is out of the way: the next save cannot destroy it"
        );
        // it goes in the `backups` folder, not in the one of the files in use
        let aside: Vec<String> = std::fs::read_dir(dir.join("backups"))
            .unwrap()
            .map(|e| name_of(&e.unwrap().path()))
            .filter(|n| n.contains(".bad-"))
            .collect();
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert!(
            dir.join("backups").join("state.json.bak1").is_file(),
            "the earlier versions too"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("backups").join(&aside[0])).unwrap(),
            "{ \"n\": ",
            "kept as it was"
        );
        // nothing usable at all: nothing, and the file is still kept
        std::fs::write(&path, "nope").unwrap();
        std::fs::remove_file(kept(&path, ".bak1")).unwrap();
        std::fs::remove_file(kept(&path, ".bak2")).ok();
        assert_eq!(load_json::<Doc>(&path, 3), None);
        assert_eq!(read_json::<Doc>(&dir.join("missing.json")), Loaded::Missing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn many_unreadable_files_do_not_fill_the_disk() {
        let dir = folder("many");
        let path = dir.join("state.json");
        std::fs::create_dir_all(dir.join("backups")).unwrap();
        for n in 1..=8 {
            std::fs::write(
                dir.join("backups").join(format!("state.json.bad-{n:03}")),
                "x",
            )
            .unwrap();
        }
        std::fs::write(&path, "bad").unwrap();
        let kept = set_aside(&path).unwrap();
        let count = std::fs::read_dir(dir.join("backups"))
            .unwrap()
            .filter(|e| name_of(&e.as_ref().unwrap().path()).contains(".bad-"))
            .count();
        assert_eq!(count, KEEP_BAD, "the oldest are deleted");
        assert!(kept.exists(), "the one that was just set aside is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
