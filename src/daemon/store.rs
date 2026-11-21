//! What the daemon remembers between runs: the graph, whether it was running, and the editors' layout.
//!
//! Plain JSON files in the data directory, written durably (see `omu_discovery::durable`: a temporary file flushed to the disk, a
//! backup of the version replaced, then a rename) so that a crash or a power cut never leaves a half-written file. A missing file
//! means "nothing saved". A file that cannot be understood is set aside (`state.json.bad-...`), never written over, and the latest
//! readable backup is used: the daemon starts with that, or empty, instead of refusing to start.

use omu_discovery::durable;
use omu_protocol::methods::Layout;
use omu_types::GraphSpec;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How many earlier versions of the graph are kept (`state.json.bak1`...): what the user built is the most valuable thing here.
const STATE_BACKUPS: usize = 5;

/// The saved audio state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedState {
    pub graph: GraphSpec,
    /// Whether the engine was running the graph (a stopped engine stays stopped after a restart).
    pub running: bool,
}

pub struct Store {
    dir: PathBuf,
    /// Writes happen one at a time, so that two saves of the same file never meet.
    writing: Mutex<()>,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Store {
        Store {
            dir: dir.into(),
            writing: Mutex::new(()),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn write<T: Serialize>(&self, name: &str, value: &T, backups: usize) -> io::Result<()> {
        let _one_at_a_time = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        durable::write_json(&self.dir.join(name), value, backups)
    }

    pub fn load_state(&self) -> SavedState {
        durable::load_json(&self.dir.join("state.json"), STATE_BACKUPS).unwrap_or_default()
    }

    pub fn save_state(&self, state: &SavedState) -> io::Result<()> {
        self.write("state.json", state, STATE_BACKUPS)
    }

    pub fn load_layout(&self) -> Layout {
        durable::load_json(&self.dir.join("layout.json"), 0).unwrap_or_default()
    }

    /// The layout changes at every move of a block: no backups of it, the file is only written safely.
    pub fn save_layout(&self, layout: &Layout) -> io::Result<()> {
        self.write("layout.json", layout, 0)
    }
}

#[cfg(test)]
pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "omu-test-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use omu_protocol::methods::NodeLayout;
    use omu_types::{NodeKind, NodeSpec};

    #[test]
    fn nothing_saved_means_empty_defaults() {
        let s = Store::new(temp_dir("empty"));
        assert_eq!(s.load_state(), SavedState::default());
        assert!(s.load_layout().is_empty());
    }

    #[test]
    fn state_and_layout_round_trip() {
        let dir = temp_dir("roundtrip");
        let s = Store::new(&dir);
        let state = SavedState {
            graph: GraphSpec {
                nodes: vec![NodeSpec {
                    id: "m".into(),
                    display: None,
                    inputs: Default::default(),
                    muted: false,
                    kind: NodeKind::Mixer,
                }],
                edges: vec![],
            },
            running: true,
        };
        s.save_state(&state).unwrap();
        let mut layout = Layout::new();
        layout.insert("m".into(), NodeLayout::at(1.0, 2.0));
        s.save_layout(&layout).unwrap();
        assert_eq!(Store::new(&dir).load_state(), state);
        assert_eq!(Store::new(&dir).load_layout(), layout);
        assert!(
            std::fs::read_dir(&dir).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")),
            "the temporary file is renamed away"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_is_set_aside_not_lost_and_the_last_good_version_is_used() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("state.json"), "{ not json").unwrap();
        assert_eq!(
            Store::new(&dir).load_state(),
            SavedState::default(),
            "nothing else to use"
        );
        let aside = |dir: &Path| {
            std::fs::read_dir(dir)
                .unwrap()
                .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                .filter(|n| n.starts_with("state.json.bad-"))
                .count()
        };
        assert_eq!(aside(&dir), 1, "the damaged file is kept for the user");
        assert!(
            !dir.join("state.json").exists(),
            "and nothing is there to be written over"
        );

        // with an earlier good version, that one is used
        let s = Store::new(&dir);
        let first = SavedState {
            graph: GraphSpec::default(),
            running: true,
        };
        s.save_state(&first).unwrap();
        let second = SavedState {
            graph: GraphSpec::default(),
            running: false,
        };
        s.save_state(&second).unwrap();
        std::fs::write(dir.join("state.json"), "").unwrap(); // a power cut can leave an empty file
        assert_eq!(Store::new(&dir).load_state(), first);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
