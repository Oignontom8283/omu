//! The soundboards people made, the sounds in them and the settings, saved in the daemon's data folder.
//!
//! `soundboards.json` holds the soundboards and the list of imported sounds; each sound is a file `sounds/<id>.wav`
//! in the format of [`omu_audiofile::wav`]. Everything is written atomically (a temporary file, then a rename), and a missing
//! or unreadable file means an empty library: the daemon never refuses to start because of it.

use omu_types::{Board, Settings, SoundInfo, SoundLibrary};
use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// The longest fade-out a pad can ask for (milliseconds).
pub const MAX_FADE_MS: u32 = 10_000;

/// A random identifier: 32 hexadecimal digits. (The standard library's hasher is seeded by the system's randomness.)
pub fn new_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let part = |salt: u64| {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(salt);
        h.finish()
    };
    format!("{:016x}{:016x}", part(1), part(2))
}

pub struct Library {
    dir: PathBuf,
    data: Mutex<SoundLibrary>,
    settings: Mutex<Settings>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Library {
    /// Reads what `dir` holds (nothing, the first time).
    pub fn load(dir: impl Into<PathBuf>) -> Library {
        let dir = dir.into();
        // a file that cannot be read is set aside (never written over) and the latest readable backup is used
        let layout = omu_discovery::DataLayout::new(&dir);
        let data =
            omu_discovery::durable::load_json(&layout.soundboards_file(), 3).unwrap_or_default();
        let settings =
            omu_discovery::durable::load_json(&layout.settings_file(), 3).unwrap_or_default();
        Library {
            dir,
            data: Mutex::new(data),
            settings: Mutex::new(settings),
        }
    }

    fn layout(&self) -> omu_discovery::DataLayout {
        omu_discovery::DataLayout::new(&self.dir)
    }

    fn write(&self, file: &std::path::Path, text: &str) -> io::Result<()> {
        omu_discovery::durable::write(file, text.as_bytes(), 3)
    }

    fn save(&self, data: &SoundLibrary) -> io::Result<()> {
        self.write(
            &self.layout().soundboards_file(),
            &serde_json::to_string_pretty(data).map_err(io::Error::other)?,
        )
    }

    /// Where the sounds are stored.
    pub fn sounds_dir(&self) -> PathBuf {
        self.layout().sounds()
    }

    /// Where a sound is first stored: a plain WAV (the importer's output).
    pub fn path_of(&self, sound: &str) -> PathBuf {
        self.sounds_dir().join(format!("{sound}.wav"))
    }

    /// Where a compressed sound is stored: an ordinary FLAC file.
    pub fn packed_path_of(&self, sound: &str) -> PathBuf {
        self.sounds_dir().join(format!("{sound}.flac"))
    }

    /// Where a sound compressed by an earlier version is (`.omuf`): it is read, then made a FLAC file.
    pub fn old_packed_path_of(&self, sound: &str) -> PathBuf {
        self.sounds_dir().join(format!("{sound}.omuf"))
    }

    /// The file a sound is read from: the compressed one when there is one, else the plain one.
    pub fn stored_path(&self, sound: &str) -> PathBuf {
        [self.packed_path_of(sound), self.old_packed_path_of(sound)]
            .into_iter()
            .find(|p| p.exists())
            .unwrap_or_else(|| self.path_of(sound))
    }

    pub fn get(&self) -> SoundLibrary {
        lock(&self.data).clone()
    }

    pub fn sound(&self, id: &str) -> Option<SoundInfo> {
        lock(&self.data).sounds.iter().find(|s| s.id == id).cloned()
    }

    pub fn board(&self, id: &str) -> Option<Board> {
        lock(&self.data).boards.iter().find(|b| b.id == id).cloned()
    }

    /// Replaces the soundboards (the sounds are left as they are). Refuses a list that repeats an identifier or whose
    /// pad plays a sound that does not exist.
    pub fn set_boards(&self, mut boards: Vec<Board>) -> Result<(), String> {
        let mut data = lock(&self.data);
        let mut ids = HashSet::new();
        for b in &mut boards {
            if b.id.is_empty() || !ids.insert(b.id.as_str()) {
                return Err(format!(
                    "the soundboard id {:?} is empty or used twice",
                    b.id
                ));
            }
            let mut pads = HashSet::new();
            for p in &mut b.pads {
                if p.end_ms != 0 && p.end_ms <= p.start_ms {
                    return Err(format!(
                        "pad {:?}: the part that plays must end after it starts",
                        p.name
                    ));
                }
                if p.fade_ms > MAX_FADE_MS {
                    return Err(format!(
                        "pad {:?}: the fade is at most {MAX_FADE_MS} ms",
                        p.name
                    ));
                }
                if let Some(c) = &p.color {
                    let ok = c.len() == 7
                        && c.starts_with('#')
                        && c[1..].chars().all(|d| d.is_ascii_hexdigit());
                    if !ok {
                        return Err(format!(
                            "pad {:?}: the colour {c:?} is not written #rrggbb",
                            p.name
                        ));
                    }
                }
                // a shortcut is kept the way it is written normally ("ctrl+alt+1" -> "Ctrl+Alt+1"); none if empty
                p.key = match p.key.as_deref().map(str::trim) {
                    None | Some("") => None,
                    Some(text) => Some(
                        omu_types::KeyCombo::parse(text)
                            .map_err(|e| format!("pad {:?}: {e}", p.name))?
                            .to_string(),
                    ),
                };
                if p.id.is_empty() || !pads.insert(p.id.as_str()) {
                    return Err(format!(
                        "the pad id {:?} of {:?} is empty or used twice",
                        p.id, b.name
                    ));
                }
                if let Some(s) = &p.sound {
                    if !data.sounds.iter().any(|x| &x.id == s) {
                        return Err(format!(
                            "pad {:?} plays a sound that does not exist",
                            p.name
                        ));
                    }
                }
            }
        }
        let mut next = data.clone();
        next.boards = boards;
        self.save(&next)
            .map_err(|e| format!("cannot save the soundboards: {e}"))?;
        *data = next;
        Ok(())
    }

    pub fn add_sound(&self, info: SoundInfo) -> io::Result<()> {
        let mut data = lock(&self.data);
        let mut next = data.clone();
        next.sounds.push(info);
        self.save(&next)?;
        *data = next;
        Ok(())
    }

    /// Replaces what is known of a sound (its size, once compressed).
    pub fn update_sound(&self, info: SoundInfo) -> io::Result<()> {
        let mut data = lock(&self.data);
        let mut next = data.clone();
        match next.sounds.iter_mut().find(|s| s.id == info.id) {
            Some(s) => *s = info,
            None => return Ok(()),
        }
        self.save(&next)?;
        *data = next;
        Ok(())
    }

    /// Forgets a sound: pads that played it have none, and its stored file is deleted. Returns whether it existed.
    pub fn remove_sound(&self, id: &str) -> io::Result<bool> {
        let mut data = lock(&self.data);
        if !data.sounds.iter().any(|s| s.id == id) {
            return Ok(false);
        }
        let mut next = data.clone();
        next.sounds.retain(|s| s.id != id);
        for p in next.boards.iter_mut().flat_map(|b| b.pads.iter_mut()) {
            if p.sound.as_deref() == Some(id) {
                p.sound = None;
            }
        }
        self.save(&next)?;
        *data = next;
        let _ = std::fs::remove_file(self.path_of(id));
        let _ = std::fs::remove_file(self.packed_path_of(id));
        let _ = std::fs::remove_file(self.old_packed_path_of(id));
        Ok(true)
    }

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    pub fn set_settings(&self, settings: Settings) -> io::Result<()> {
        self.write(
            &self.layout().settings_file(),
            &serde_json::to_string_pretty(&settings).map_err(io::Error::other)?,
        )?;
        *lock(&self.settings) = settings;
        Ok(())
    }

    /// Bytes the stored sounds take on the disk.
    pub fn disk_bytes(&self) -> u64 {
        lock(&self.data).sounds.iter().map(|s| s.bytes).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omu_types::Pad;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("omu-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn sound(id: &str) -> SoundInfo {
        SoundInfo {
            id: id.into(),
            name: format!("{id}.mp3"),
            channels: 2,
            frames: 48_000,
            bytes: 192_000,
            compressed: false,
        }
    }

    fn board(id: &str, pads: &[(&str, Option<&str>)]) -> Board {
        Board {
            id: id.into(),
            name: format!("Board {id}"),
            pads: pads
                .iter()
                .map(|(p, s)| Pad {
                    id: (*p).into(),
                    name: format!("Pad {p}"),
                    sound: s.map(str::to_owned),
                    gain_db: 0.0,
                    looped: false,
                    exclusive: false,
                    fade_ms: 0,
                    start_ms: 0,
                    end_ms: 0,
                    color: None,
                    hold: false,
                    key: None,
                })
                .collect(),
        }
    }

    #[test]
    fn identifiers_are_random_and_look_alike() {
        let (a, b) = (new_id(), new_id());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn what_is_saved_is_there_after_a_restart_and_a_missing_file_is_an_empty_library() {
        let dir = temp("save");
        assert_eq!(Library::load(&dir).get(), SoundLibrary::default());
        let lib = Library::load(&dir);
        lib.add_sound(sound("s1")).unwrap();
        lib.set_boards(vec![board("b1", &[("p1", Some("s1")), ("p2", None)])])
            .unwrap();
        lib.set_settings(Settings {
            sound_cache_mb: 64,
            ..Default::default()
        })
        .unwrap();
        let again = Library::load(&dir);
        assert_eq!(again.get(), lib.get());
        assert_eq!(again.settings().sound_cache_mb, 64);
        assert_eq!(again.sound("s1").unwrap().bytes, 192_000);
        assert_eq!(again.disk_bytes(), 192_000);
        assert!(
            std::fs::read_dir(dir.join("data")).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")),
            "no temporary file left"
        );
        // an unreadable file is set aside, not lost; with no earlier version to use the library is empty, not a refusal to start
        for n in 1..=3 {
            let _ = std::fs::remove_file(dir.join(format!("data/backups/soundboards.json.bak{n}")));
        }
        std::fs::write(dir.join("data/soundboards.json"), "{ not json").unwrap();
        assert_eq!(Library::load(&dir).get(), SoundLibrary::default());
        assert!(
            std::fs::read_dir(dir.join("data/backups"))
                .unwrap()
                .any(|e| e
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("soundboards.json.bad-")),
            "the file that could not be read is kept"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_list_of_soundboards_that_does_not_hold_together_is_refused_and_nothing_changes() {
        let dir = temp("refuse");
        let lib = Library::load(&dir);
        lib.add_sound(sound("s1")).unwrap();
        lib.set_boards(vec![board("b1", &[("p1", Some("s1"))])])
            .unwrap();
        let before = lib.get();
        for bad in [
            vec![board("b", &[]), board("b", &[])],
            vec![board("", &[])],
            vec![board("b", &[("p", None), ("p", None)])],
            vec![board("b", &[("p", Some("missing"))])],
        ] {
            assert!(lib.set_boards(bad).is_err());
            assert_eq!(lib.get(), before, "nothing changed");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_a_sound_empties_the_pads_that_played_it_and_deletes_its_file() {
        let dir = temp("remove");
        let lib = Library::load(&dir);
        lib.add_sound(sound("s1")).unwrap();
        lib.add_sound(sound("s2")).unwrap();
        std::fs::create_dir_all(lib.sounds_dir()).unwrap();
        std::fs::write(lib.path_of("s1"), b"x").unwrap();
        lib.set_boards(vec![board("b", &[("p1", Some("s1")), ("p2", Some("s2"))])])
            .unwrap();
        assert!(lib.remove_sound("s1").unwrap());
        assert!(!lib.remove_sound("s1").unwrap(), "already gone");
        let b = lib.board("b").unwrap();
        assert_eq!(b.pads[0].sound, None);
        assert_eq!(b.pads[1].sound.as_deref(), Some("s2"));
        assert!(!lib.path_of("s1").exists());
        assert_eq!(lib.get().sounds.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pad_options_are_checked_and_shortcuts_are_written_the_normal_way() {
        let dir = temp("options");
        let lib = Library::load(&dir);
        let with = |change: &dyn Fn(&mut omu_types::Pad)| {
            let mut b = board("b", &[("p", None)]);
            change(&mut b.pads[0]);
            vec![b]
        };
        assert!(lib
            .set_boards(with(&|p| p.color = Some("#12aBcd".into())))
            .is_ok());
        for bad in ["red", "#12345", "#12345g", "123456 "] {
            assert!(
                lib.set_boards(with(&|p| p.color = Some(bad.into())))
                    .is_err(),
                "{bad}"
            );
        }
        assert!(lib
            .set_boards(with(&|p| {
                p.start_ms = 1000;
                p.end_ms = 2000
            }))
            .is_ok());
        assert!(lib
            .set_boards(with(&|p| {
                p.start_ms = 2000;
                p.end_ms = 2000
            }))
            .is_err());
        assert!(lib.set_boards(with(&|p| p.fade_ms = MAX_FADE_MS)).is_ok());
        assert!(lib
            .set_boards(with(&|p| p.fade_ms = MAX_FADE_MS + 1))
            .is_err());
        assert!(lib
            .set_boards(with(&|p| p.key = Some("ctrl+nope".into())))
            .is_err());
        lib.set_boards(with(&|p| p.key = Some(" alt+ctrl+f8 ".into())))
            .unwrap();
        assert_eq!(
            lib.get().boards[0].pads[0].key.as_deref(),
            Some("Ctrl+Alt+F8")
        );
        lib.set_boards(with(&|p| p.key = Some("  ".into())))
            .unwrap();
        assert_eq!(
            lib.get().boards[0].pads[0].key,
            None,
            "an empty shortcut is none"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
