//! The last log lines, kept in memory: they go into crash reports (the log file may not have caught up with the
//! moment of the crash) and are served to clients (`logs.recent`) for a live view.

use std::collections::VecDeque;
use std::sync::Mutex;

/// How many lines are kept.
pub const CAPACITY: usize = 2000;

/// One log line, numbered in order of arrival so that a client can ask for "what came after the last one I saw".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Line {
    pub seq: u64,
    pub text: String,
}

struct Ring {
    lines: VecDeque<Line>,
    next_seq: u64,
}

impl Ring {
    const fn new() -> Ring {
        Ring {
            lines: VecDeque::new(),
            next_seq: 1,
        }
    }

    fn push(&mut self, text: String) {
        let seq = self.next_seq;
        self.next_seq += 1;
        if self.lines.len() == CAPACITY {
            self.lines.pop_front();
        }
        self.lines.push_back(Line { seq, text });
    }

    fn recent(&self, n: usize) -> Vec<Line> {
        let skip = self.lines.len().saturating_sub(n);
        self.lines.iter().skip(skip).cloned().collect()
    }

    fn since(&self, seq: u64, limit: usize) -> Vec<Line> {
        self.lines
            .iter()
            .filter(|l| l.seq > seq)
            .take(limit)
            .cloned()
            .collect()
    }
}

static RING: Mutex<Ring> = Mutex::new(Ring::new());

fn ring() -> std::sync::MutexGuard<'static, Ring> {
    RING.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn push(text: String) {
    ring().push(text);
}

/// The most recent `n` lines, oldest first.
pub fn recent(n: usize) -> Vec<Line> {
    ring().recent(n)
}

/// The lines that arrived after `seq` (at most `limit`, oldest first).
pub fn since(seq: u64, limit: usize) -> Vec<Line> {
    ring().since(seq, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_numbered_bounded_and_can_be_followed() {
        let mut ring = Ring::new();
        for i in 0..CAPACITY + 10 {
            ring.push(format!("line {i}"));
        }
        assert_eq!(
            ring.recent(usize::MAX).len(),
            CAPACITY,
            "the oldest lines are dropped"
        );
        let tail = ring.recent(3);
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[2].text, format!("line {}", CAPACITY + 9));
        assert!(tail[0].seq < tail[1].seq && tail[1].seq < tail[2].seq);
        let after = ring.since(tail[0].seq, 10);
        assert_eq!(
            after.iter().map(|l| l.seq).collect::<Vec<_>>(),
            vec![tail[1].seq, tail[2].seq]
        );
        assert!(ring.since(tail[2].seq, 10).is_empty());
    }
}
