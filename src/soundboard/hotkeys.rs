//! Keyboard shortcuts that play pads. The daemon listens for them while the pad's soundboard is in the running graph
//! (and the graph is running): the same global shortcuts the `hotkey` block uses, owned here instead of by a block.

use super::SoundEngine;
use omu_hal::{Backend, Running};
use omu_types::{Board, GraphSpec, HotkeyProblem, KeyCombo, NodeKind};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A pad and the shortcut that plays it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadKey {
    pub board: String,
    pub pad: String,
    pub keys: String,
    /// Plays while the key is held.
    pub hold: bool,
}

/// The shortcuts that should be listening: those of the pads of the soundboards that have a block in the graph.
pub fn wanted(boards: &[Board], graph: &GraphSpec) -> Vec<PadKey> {
    let used = |id: &str| {
        // a soundboard inside a group plays, so its shortcuts must listen
        graph
            .leaf_nodes()
            .into_iter()
            .any(|n| matches!(&n.kind, NodeKind::Soundboard { board, .. } if board == id))
    };
    boards
        .iter()
        .filter(|b| used(&b.id))
        .flat_map(|b| {
            b.pads.iter().filter_map(|p| {
                p.key.as_ref().map(|keys| PadKey {
                    board: b.id.clone(),
                    pad: p.id.clone(),
                    keys: keys.clone(),
                    hold: p.hold,
                })
            })
        })
        .collect()
}

struct Held {
    keys: String,
    hold: bool,
    _listening: Box<dyn Running>,
}

/// The shortcuts that are registered with the system, and why the others are not.
#[derive(Default)]
pub struct PadHotkeys {
    held: Mutex<HashMap<(String, String), Held>>,
    problems: Mutex<Vec<HotkeyProblem>>,
}

impl PadHotkeys {
    /// Makes the registered shortcuts the wanted ones: the ones no longer wanted (or whose keys changed) are released
    /// first, so that a pad can take the keys another one just left.
    pub fn sync(&self, backend: &dyn Backend, sounds: &Arc<SoundEngine>, wanted: &[PadKey]) {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        held.retain(|(board, pad), h| {
            wanted
                .iter()
                .any(|w| &w.board == board && &w.pad == pad && w.keys == h.keys && w.hold == h.hold)
        });
        let mut problems = vec![];
        for w in wanted {
            let id = (w.board.clone(), w.pad.clone());
            if held.contains_key(&id) {
                continue;
            }
            let fail = |message: String| HotkeyProblem {
                board: w.board.clone(),
                pad: w.pad.clone(),
                message,
            };
            let combo = match KeyCombo::parse(&w.keys) {
                Ok(c) => c,
                Err(e) => {
                    problems.push(fail(e));
                    continue;
                }
            };
            let (engine, board, pad) = (sounds.clone(), w.board.clone(), w.pad.clone());
            let press: omu_hal::HotkeyCallback = Box::new(move || {
                // pressed while the soundboard is not running, or a sound that is gone: nothing to play
                if let Err(e) = engine.play(&board, &pad) {
                    tracing::debug!("pad shortcut: {e}");
                }
            });
            let opened = if w.hold {
                let (engine, board, pad) = (sounds.clone(), w.board.clone(), w.pad.clone());
                backend.open_hotkey_hold(
                    &combo,
                    press,
                    Box::new(move || {
                        let _ = engine.stop(&board, Some(&pad));
                    }),
                )
            } else {
                backend.open_hotkey(&combo, press)
            };
            match opened {
                Ok(listening) => {
                    held.insert(
                        id,
                        Held {
                            keys: w.keys.clone(),
                            hold: w.hold,
                            _listening: listening,
                        },
                    );
                }
                Err(e) => problems.push(fail(e.to_string())),
            }
        }
        *self.problems.lock().unwrap_or_else(|e| e.into_inner()) = problems;
    }

    /// The shortcuts that could not be registered at the last [`PadHotkeys::sync`].
    pub fn problems(&self) -> Vec<HotkeyProblem> {
        self.problems
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Releases every shortcut (the program is ending, or nothing runs).
    pub fn release_all(&self) {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.problems
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omu_types::{NodeSpec, Pad};

    fn board(id: &str, pads: &[(&str, Option<&str>)]) -> Board {
        Board {
            id: id.into(),
            name: id.into(),
            pads: pads
                .iter()
                .map(|(p, k)| Pad {
                    id: (*p).into(),
                    name: (*p).into(),
                    sound: None,
                    gain_db: 0.0,
                    looped: false,
                    exclusive: false,
                    fade_ms: 0,
                    start_ms: 0,
                    end_ms: 0,
                    color: None,
                    hold: false,
                    key: k.map(str::to_owned),
                })
                .collect(),
        }
    }

    fn graph_with(boards: &[&str]) -> GraphSpec {
        GraphSpec {
            nodes: boards
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    NodeSpec::new(
                        &format!("sb{i}"),
                        NodeKind::Soundboard { board: (*b).into() },
                    )
                })
                .collect(),
            edges: vec![],
        }
    }

    #[test]
    fn only_the_pads_with_keys_of_soundboards_in_the_graph_are_wanted() {
        let boards = vec![
            board("a", &[("p1", Some("F8")), ("p2", None)]),
            board("b", &[("q1", Some("F9"))]),
        ];
        let w = wanted(&boards, &graph_with(&["a"]));
        assert_eq!(
            w,
            vec![PadKey {
                board: "a".into(),
                pad: "p1".into(),
                keys: "F8".into(),
                hold: false
            }]
        );
        assert!(wanted(&boards, &graph_with(&[])).is_empty());
        assert_eq!(wanted(&boards, &graph_with(&["a", "b"])).len(), 2);
    }
}
