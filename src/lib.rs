//! Omu: a node-graph audio router. Capture the sound of an application, process it, play it into a virtual microphone.
//!
//! Every component is a module of this library (a folder of `src/`); the binaries in `src/bin/` are thin entry points.
//!
//! | module | role | needs feature |
//! |---|---|---|
//! | [`branding`] | product name and identifiers, resolved at compile time from `branding.toml` | |
//! | [`types`] | graph schema, ports, audio format, device references | |
//! | [`graph`] | graph validation and ordering | |
//! | [`dsp`] | allocation-free audio building blocks | |
//! | [`hal`] | platform backend traits (+ simulated backend, + Windows backend) | |
//! | [`protocol`] | JSON-RPC control protocol: envelopes, typed methods and events | |
//! | `engine` | real-time engine: bridges, plan, graph thread, supervision | `engine` |

pub mod branding;
pub mod dsp;
pub mod graph;
pub mod hal;
pub mod protocol;
pub mod types;

#[cfg(feature = "engine")]
pub mod engine;
