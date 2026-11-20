//! The graph thread: runs the current [`Plan`] once per cycle on a steady timer, and swaps in a new plan when the
//! control side sends one (the old plan goes back through a second queue so that it is freed off the real-time
//! thread).

use super::plan::Plan;
use omu_types::ENGINE_SAMPLE_RATE;
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Counters of the graph thread.
#[derive(Debug, Default)]
pub struct RuntimeStats {
    pub cycles: AtomicU64,
    /// Cycles that started more than a whole cycle late (the thread was not scheduled in time).
    pub late_cycles: AtomicU64,
    /// Times the timer had to be re-synchronised after a long stall.
    pub resyncs: AtomicU64,
    /// Times a block panicked while processing (the plan was dropped; the supervisor builds a new one).
    pub panics: AtomicU64,
}

pub struct GraphThread {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    plans_tx: Producer<Box<Plan>>,
    graveyard_rx: Consumer<Box<Plan>>,
    pub stats: Arc<RuntimeStats>,
}

/// Called once at the start of the graph thread (raise its priority, name it for profilers...).
pub type ThreadInit = Arc<dyn Fn() + Send + Sync>;

impl GraphThread {
    pub fn spawn(cycle_frames: usize, init: Option<ThreadInit>) -> GraphThread {
        let (plans_tx, plans_rx) = RingBuffer::new(8);
        let (grave_tx, graveyard_rx) = RingBuffer::new(8);
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(RuntimeStats::default());
        let (stop2, stats2) = (stop.clone(), stats.clone());
        let thread = std::thread::Builder::new()
            .name("graph".into())
            .spawn(move || run(cycle_frames, plans_rx, grave_tx, stop2, stats2, init))
            .expect("cannot spawn the graph thread");
        GraphThread {
            stop,
            thread: Some(thread),
            plans_tx,
            graveyard_rx,
            stats,
        }
    }

    /// Hands a new plan to the graph thread (taking effect at its next cycle). Fails, returning the plan, when the
    /// queue is full (the thread is stalled).
    pub fn send(&mut self, plan: Box<Plan>) -> Result<(), Box<Plan>> {
        self.plans_tx
            .push(plan)
            .map_err(|rtrb::PushError::Full(p)| p)
    }

    /// Frees the plans the graph thread has replaced.
    pub fn collect_garbage(&mut self) {
        while self.graveyard_rx.pop().is_ok() {}
    }

    /// Frees any plan still queued by dropping the thread's end... (the thread owns them; they go with it).
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
}

impl Drop for GraphThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(
    cycle_frames: usize,
    mut plans_rx: Consumer<Box<Plan>>,
    mut grave_tx: Producer<Box<Plan>>,
    stop: Arc<AtomicBool>,
    stats: Arc<RuntimeStats>,
    init: Option<ThreadInit>,
) {
    if let Some(init) = init {
        init();
    }
    let cycle = Duration::from_secs_f64(cycle_frames as f64 / ENGINE_SAMPLE_RATE as f64);
    let mut current: Option<Box<Plan>> = None;
    // A replaced plan the graveyard had no room for yet.
    let mut stash: Option<Box<Plan>> = None;
    let mut deadline = Instant::now() + cycle;
    while !stop.load(Ordering::Relaxed) {
        if let Some(old) = stash.take() {
            if let Err(rtrb::PushError::Full(p)) = grave_tx.push(old) {
                stash = Some(p);
            }
        }
        if stash.is_none() {
            if let Ok(mut new) = plans_rx.pop() {
                if let Some(old) = current.take() {
                    new.inherit(&old);
                    if let Err(rtrb::PushError::Full(p)) = grave_tx.push(old) {
                        stash = Some(p);
                    }
                }
                current = Some(new);
            }
        }
        if let Some(plan) = current.as_mut() {
            // A panic in a block must not take the process down: it is reported (crash report, log), the plan is
            // dropped and the thread keeps its timer; the supervisor sees the counter and builds a new plan.
            // Nothing here may log or allocate on the normal path: only the panic path does.
            if omu_diagnostics::contained(|| plan.process()).is_err() {
                stats.panics.fetch_add(1, Ordering::Relaxed);
                current = None;
            }
        }
        stats.cycles.fetch_add(1, Ordering::Relaxed);

        // Pace: sleep most of the wait, spin the last stretch.
        let now = Instant::now();
        if now > deadline + cycle {
            stats.late_cycles.fetch_add(1, Ordering::Relaxed);
            if now > deadline + 5 * cycle {
                stats.resyncs.fetch_add(1, Ordering::Relaxed);
                deadline = now;
            }
        }
        deadline += cycle;
        let now = Instant::now();
        if deadline > now {
            let wait = deadline - now;
            if wait > Duration::from_micros(1500) {
                std::thread::sleep(wait - Duration::from_micros(1000));
            }
            while Instant::now() < deadline {
                std::hint::spin_loop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Meter, NodeOp, PlanNode};

    fn plan_of(op: NodeOp) -> Box<Plan> {
        Box::new(Plan::new(
            vec![PlanNode {
                id: "x".into(),
                op,
                inputs: vec![],
                meter: Arc::<Meter>::default(),
                param: None,
                net: None,
            }],
            480,
        ))
    }

    fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ok() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_block_that_panics_is_counted_and_the_thread_keeps_running_for_the_next_plan() {
        let mut thread = GraphThread::spawn(480, None);
        let stats = thread.stats.clone();
        assert!(thread.send(plan_of(NodeOp::Panic)).is_ok());
        wait_until("the panic to be counted", || {
            stats.panics.load(Ordering::Relaxed) == 1
        });
        // the bad plan is gone: the thread idles but still paces cycles
        let cycles = stats.cycles.load(Ordering::Relaxed);
        wait_until("more cycles", || {
            stats.cycles.load(Ordering::Relaxed) > cycles + 3
        });
        assert!(!thread.is_finished());
        // a new plan runs normally
        assert!(thread.send(plan_of(NodeOp::SilentSource)).is_ok());
        let cycles = stats.cycles.load(Ordering::Relaxed);
        wait_until("cycles with the new plan", || {
            stats.cycles.load(Ordering::Relaxed) > cycles + 3
        });
        assert_eq!(
            stats.panics.load(Ordering::Relaxed),
            1,
            "the new plan does not panic"
        );
    }
}
