//! The soundboards people made, the sounds in them and the settings, saved in the daemon's data folder.
//!
//! `soundboards.json` holds the soundboards and the list of imported sounds; each sound is a file `sounds/<id>.wav`
//! in the format of [`super::wav`]. Everything is written atomically (a temporary file, then a rename), and a missing
//! or unreadable file means an empty library: the daemon never refuses to start because of it.

use crate::types::{Board, Settings, SoundInfo, SoundLibrary};
use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

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
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
        let data = read("soundboards.json")
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let settings = read("settings.json")
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Library {
            dir,
            data: Mutex::new(data),
            settings: Mutex::new(settings),
        }
    }

    fn write(&self, name: &str, text: &str) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, self.dir.join(name))
    }

    fn save(&self, data: &SoundLibrary) -> io::Result<()> {
        self.write(
            "soundboards.json",
            &serde_json::to_string_pretty(data).map_err(io::Error::other)?,
        )
    }

    /// Where the sounds are stored.
    pub fn sounds_dir(&self) -> PathBuf {
        self.dir.join("sounds")
    }

    /// The stored file of a sound.
    pub fn path_of(&self, sound: &str) -> PathBuf {
        self.sounds_dir().join(format!("{sound}.wav"))
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
                // a shortcut is kept the way it is written normally ("ctrl+alt+1" -> "Ctrl+Alt+1"); none if empty
                p.key = match p.key.as_deref().map(str::trim) {
                    None | Some("") => None,
                    Some(text) => Some(
                        crate::types::KeyCombo::parse(text)
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
        Ok(true)
    }

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    pub fn set_settings(&self, settings: Settings) -> io::Result<()> {
        self.write(
            "settings.json",
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
    use crate::types::Pad;

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
        lib.set_settings(Settings { sound_cache_mb: 64 }).unwrap();
        let again = Library::load(&dir);
        assert_eq!(again.get(), lib.get());
        assert_eq!(again.settings().sound_cache_mb, 64);
        assert_eq!(again.sound("s1").unwrap().bytes, 192_000);
        assert_eq!(again.disk_bytes(), 192_000);
        assert!(
            !dir.join("soundboards.json.tmp").exists(),
            "no temporary file left"
        );
        // an unreadable file is an empty library, not a refusal to start
        std::fs::write(dir.join("soundboards.json"), "{ not json").unwrap();
        assert_eq!(Library::load(&dir).get(), SoundLibrary::default());
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
}
