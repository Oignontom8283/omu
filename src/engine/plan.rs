//! The executable form of a graph: nodes in evaluation order, each with a pre-allocated output buffer. The graph
//! thread runs [`Plan::process`] once per cycle; nothing in it allocates or blocks.

use super::bridge::{SinkProducer, SourceConsumer};
use crate::dsp::{add_into, peak, SmoothedGain};
use crate::types::{NodeId, ENGINE_CHANNELS, ENGINE_SAMPLE_RATE};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const CH: usize = ENGINE_CHANNELS as usize;
/// Time a gain change takes to complete a full 0 -> 1 move.
const GAIN_RAMP_MS: f32 = 10.0;

/// Highest absolute sample seen since the last [`Meter::take`] (non-negative `f32` bits order like integers).
#[derive(Debug, Default)]
pub struct Meter(AtomicU32);

impl Meter {
    pub fn record(&self, samples: &[f32]) {
        self.0.fetch_max(peak(samples).to_bits(), Ordering::Relaxed);
    }
    /// Returns the peak since the previous call and resets it.
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

pub enum NodeOp {
    /// Audio from a device (the lock is only ever taken by the graph thread).
    Source(Arc<Mutex<SourceConsumer>>),
    /// A source whose stream is not running: silence.
    SilentSource,
    /// Audio to a device.
    Sink(Arc<Mutex<SinkProducer>>),
    /// A sink whose stream is not running: the audio is discarded.
    DiscardSink,
    Gain(SmoothedGain),
    Mixer,
}

pub struct PlanNode {
    pub id: NodeId,
    pub op: NodeOp,
    /// Positions (in this plan's order) of the nodes feeding this one.
    pub inputs: Vec<usize>,
    pub meter: Arc<Meter>,
}

pub struct Plan {
    pub nodes: Vec<PlanNode>,
    bufs: Vec<Vec<f32>>,
    silence: Vec<f32>,
    cycle_samples: usize,
}

impl Plan {
    pub fn new(nodes: Vec<PlanNode>, cycle_frames: usize) -> Plan {
        let cycle_samples = cycle_frames * CH;
        Plan {
            bufs: nodes.iter().map(|_| vec![0.0; cycle_samples]).collect(),
            nodes,
            silence: vec![0.0; cycle_samples],
            cycle_samples,
        }
    }

    pub fn cycle_samples(&self) -> usize {
        self.cycle_samples
    }

    /// A gain node of this plan, built for the engine format, with `db` as target.
    pub fn gain(db: f32) -> SmoothedGain {
        SmoothedGain::new(db, ENGINE_SAMPLE_RATE, GAIN_RAMP_MS)
    }

    /// Continues the state of the gains of `old` that exist, unchanged in kind, here (no jump when a graph is
    /// replaced).
    pub fn inherit(&mut self, old: &Plan) {
        for node in &mut self.nodes {
            if let NodeOp::Gain(g) = &mut node.op {
                if let Some(PlanNode {
                    op: NodeOp::Gain(old_g),
                    ..
                }) = old.nodes.iter().find(|n| n.id == node.id)
                {
                    g.start_from(old_g.current());
                }
            }
        }
    }

    /// Runs one cycle.
    pub fn process(&mut self) {
        for i in 0..self.nodes.len() {
            let mut out = std::mem::take(&mut self.bufs[i]);
            let node = &mut self.nodes[i];
            match &mut node.op {
                NodeOp::Source(consumer) => match consumer.try_lock() {
                    Ok(mut c) => c.pull(&mut out),
                    Err(_) => out.fill(0.0),
                },
                NodeOp::SilentSource => out.fill(0.0),
                NodeOp::Gain(g) => {
                    let input = node
                        .inputs
                        .first()
                        .map_or(&self.silence[..], |&j| &self.bufs[j][..]);
                    out.copy_from_slice(input);
                    g.apply(&mut out, CH);
                }
                NodeOp::Mixer => {
                    out.fill(0.0);
                    for &j in &node.inputs {
                        add_into(&mut out, &self.bufs[j]);
                    }
                }
                NodeOp::Sink(producer) => {
                    let input = node
                        .inputs
                        .first()
                        .map_or(&self.silence[..], |&j| &self.bufs[j][..]);
                    node.meter.record(input);
                    if let Ok(mut p) = producer.try_lock() {
                        p.push(input);
                    }
                }
                NodeOp::DiscardSink => {
                    let input = node
                        .inputs
                        .first()
                        .map_or(&self.silence[..], |&j| &self.bufs[j][..]);
                    node.meter.record(input);
                }
            }
            if !matches!(node.op, NodeOp::Sink(_) | NodeOp::DiscardSink) {
                node.meter.record(&out);
            }
            self.bufs[i] = out;
        }
    }

    /// The output buffer of node `i` after the last cycle (for tests and metering).
    pub fn output(&self, i: usize) -> &[f32] {
        &self.bufs[i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bridge::{sink_bridge, source_bridge};
    use crate::types::AudioFormat;

    fn node(id: &str, op: NodeOp, inputs: Vec<usize>) -> PlanNode {
        PlanNode {
            id: id.into(),
            op,
            inputs,
            meter: Arc::default(),
        }
    }

    #[test]
    fn gain_node_scales_its_input_and_meters_it() {
        let (mut sp, sc, _) = source_bridge(AudioFormat::ENGINE, 100.0);
        for _ in 0..14 {
            sp.push(&vec![0.5f32; 960]); // 14 cycles of audio for the 10 cycles run below
        }
        let mut plan = Plan::new(
            vec![
                node("src", NodeOp::Source(Arc::new(Mutex::new(sc))), vec![]),
                node("g", NodeOp::Gain(Plan::gain(-6.0206)), vec![0]),
            ],
            480,
        );
        for _ in 0..10 {
            plan.process();
        }
        let tail = plan.output(1);
        assert!(tail.iter().all(|s| (*s - 0.25).abs() < 1e-2), "{}", tail[0]);
        assert!(plan.nodes[1].meter.take() > 0.2);
        assert_eq!(plan.nodes[1].meter.take(), 0.0, "take() resets the peak");
    }

    #[test]
    fn mixer_sums_and_unconnected_gain_is_silent() {
        let mut plan = Plan::new(
            vec![
                node("a", NodeOp::SilentSource, vec![]),
                node("b", NodeOp::SilentSource, vec![]),
                node("m", NodeOp::Mixer, vec![0, 1]),
                node("g", NodeOp::Gain(Plan::gain(0.0)), vec![]),
            ],
            480,
        );
        plan.bufs[0].fill(0.0); // sources overwrite with silence anyway
        plan.process();
        assert!(plan.output(2).iter().all(|s| *s == 0.0));
        assert!(plan.output(3).iter().all(|s| *s == 0.0));
    }

    #[test]
    fn sink_pushes_its_input_into_the_bridge() {
        let (sp, mut sc, _) = sink_bridge(AudioFormat::ENGINE, 10.0);
        let (mut src_p, src_c, _) = source_bridge(AudioFormat::ENGINE, 100.0);
        for _ in 0..12 {
            src_p.push(&vec![0.3f32; 960]); // enough to prime the source (100 ms) and to outlast the 6 cycles below
        }
        let mut plan = Plan::new(
            vec![
                node("src", NodeOp::Source(Arc::new(Mutex::new(src_c))), vec![]),
                node("out", NodeOp::Sink(Arc::new(Mutex::new(sp))), vec![0]),
            ],
            480,
        );
        for _ in 0..6 {
            plan.process();
        }
        let mut out = vec![0.0f32; 960];
        sc.pull(&mut out);
        assert!(out.iter().any(|s| (*s - 0.3).abs() < 1e-2));
    }

    #[test]
    fn inherit_continues_the_gain_instead_of_jumping() {
        let mut old = Plan::new(
            vec![node("g", NodeOp::Gain(Plan::gain(-20.0)), vec![])],
            480,
        );
        old.process();
        let before = match &old.nodes[0].op {
            NodeOp::Gain(g) => g.current(),
            _ => unreachable!(),
        };
        let mut new = Plan::new(vec![node("g", NodeOp::Gain(Plan::gain(0.0)), vec![])], 480);
        new.inherit(&old);
        let after = match &new.nodes[0].op {
            NodeOp::Gain(g) => g.current(),
            _ => unreachable!(),
        };
        assert_eq!(before, after);
    }
}
