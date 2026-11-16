//! What the daemon remembers between runs: the graph, whether it was running, and the editors' layout.
//!
//! Plain JSON files in the data directory, written atomically (a temporary file, then a rename) so that a crash
//! or a power cut never leaves a half-written file. A missing or unreadable file means "nothing saved": the daemon
//! then starts empty instead of refusing to start.

use omu_protocol::methods::Layout;
use omu_types::GraphSpec;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// The saved audio state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedState {
    pub graph: GraphSpec,
    /// Whether the engine was running the graph (a stopped engine stays stopped after a restart).
    pub running: bool,
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Store {
        Store { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn read<T: for<'de> Deserialize<'de>>(&self, name: &str) -> Option<T> {
        let text = std::fs::read_to_string(self.dir.join(name)).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn write<T: Serialize>(&self, name: &str, value: &T) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
        let target = self.dir.join(name);
        let tmp = self.dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &target)
    }

    pub fn load_state(&self) -> SavedState {
        self.read("state.json").unwrap_or_default()
    }

    pub fn save_state(&self, state: &SavedState) -> io::Result<()> {
        self.write("state.json", state)
    }

    pub fn load_layout(&self) -> Layout {
        self.read("layout.json").unwrap_or_default()
    }

    pub fn save_layout(&self, layout: &Layout) -> io::Result<()> {
        self.write("layout.json", layout)
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
            !dir.join("state.json.tmp").exists(),
            "the temporary file is renamed away"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_is_treated_as_nothing_saved() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("state.json"), "{ not json").unwrap();
        assert_eq!(Store::new(&dir).load_state(), SavedState::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
