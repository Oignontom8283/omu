//! Things that must be undone even when the program dies: a leftover discovery file points clients at a daemon that
//! is gone, a virtual microphone that stays attached keeps a ghost device in Windows until the next reboot.
//!
//! The owner of such a thing registers how to undo it and drops the [`Registration`] when it cleans up the normal
//! way. On a crash (a panic outside any guarded block, an unhandled exception, the console window being closed, the
//! user logging off) [`run_all`] runs what is still registered, most recent first, each in isolation: a hook that
//! panics does not stop the others (and [`super::crash::fatal`] puts a time limit on the whole).

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

type Hook = Box<dyn FnOnce() + Send>;
type Entry = (u64, &'static str, Hook);

static HOOKS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
static NEXT: AtomicU64 = AtomicU64::new(1);

fn hooks() -> std::sync::MutexGuard<'static, Vec<Entry>> {
    HOOKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Proof that a cleanup is registered; dropping it forgets the hook (the owner has cleaned up itself).
#[must_use = "dropping the registration right away forgets the cleanup"]
pub struct Registration(u64);

impl Drop for Registration {
    fn drop(&mut self) {
        hooks().retain(|(id, _, _)| *id != self.0);
    }
}

/// Registers `cleanup`, to run if the program has to die before the returned registration is dropped.
pub fn register(name: &'static str, cleanup: impl FnOnce() + Send + 'static) -> Registration {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    hooks().push((id, name, Box::new(cleanup)));
    Registration(id)
}

/// Runs every registered cleanup, most recent first; returns how many ran. Each hook runs once, whatever the number
/// of callers, and a panicking hook is logged and skipped.
pub fn run_all() -> usize {
    let taken: Vec<Entry> = std::mem::take(&mut *hooks());
    let count = taken.len();
    for (_, name, hook) in taken.into_iter().rev() {
        let started = Instant::now();
        match catch_unwind(AssertUnwindSafe(hook)) {
            Ok(()) => tracing::info!(
                cleanup = name,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "cleanup done"
            ),
            Err(_) => tracing::error!(cleanup = name, "cleanup panicked; going on with the others"),
        }
    }
    count
}

/// Names of the cleanups still registered (for crash reports).
pub fn pending() -> Vec<&'static str> {
    hooks().iter().map(|(_, n, _)| *n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as M};

    #[test]
    fn cleanups_run_most_recent_first_once_and_a_panic_does_not_stop_the_others() {
        let order = Arc::new(M::new(vec![]));
        let (o1, o2, o4) = (order.clone(), order.clone(), order.clone());
        let _a = register("test-first", move || o1.lock().unwrap().push("first"));
        let b = register("test-forgotten", move || {
            o2.lock().unwrap().push("forgotten")
        });
        let _c = register("test-panics", move || panic!("boom in a cleanup"));
        let _d = register("test-last", move || o4.lock().unwrap().push("last"));
        drop(b); // the owner cleaned up by itself
        let names: Vec<_> = pending()
            .into_iter()
            .filter(|n| n.starts_with("test-"))
            .collect();
        assert_eq!(names, vec!["test-first", "test-panics", "test-last"]);

        let ran = run_all();
        assert!(ran >= 3);
        assert_eq!(*order.lock().unwrap(), vec!["last", "first"]);
        assert_eq!(run_all(), 0, "a hook never runs twice");
    }
}
