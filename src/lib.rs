//! Omu: a node-graph audio router. Capture the sound of an application, process it, play it into a virtual microphone.
//!
//! Every component is a module of this library (a folder of `src/`); the binaries in `src/bin/` are thin entry points.
//!
//! | module | role | needs feature |
//! |---|---|---|
//! | [`branding`] | product name and identifiers, resolved at compile time from `branding.toml` | |
//! | [`types`] | graph schema, ports, audio format, device references | |
//! | [`graph`] | graph validation and ordering | |
//! | `cli` | the command-line client's logic | `cli` |
//! | `client` | typed calls to a daemon over HTTP | `client` |
//! | [`diagnostics`] | logs, crash reports, cleanup on crash | `logging` for the logs |
//! | [`discovery`] | how clients find the local daemon (address and token) | |
//! | `dsp` | allocation-free audio building blocks | `engine` |
//! | `hal` | platform backend traits (+ simulated backend, + Windows backend) | `engine` |
//! | [`protocol`] | JSON-RPC control protocol: envelopes, typed methods and events | |
//! | `engine` | real-time engine: bridges, plan, graph thread, supervision | `engine` |
//! | `virtual_devices` | virtual microphones through emulated USB audio devices | `engine` |
//! | `daemon` | the daemon core: request dispatch, persistence, events | `daemon` |
//! | `gui` | the desktop window's commands | `gui` |

pub mod branding;
#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "client")]
pub mod client;
pub mod diagnostics;
pub mod discovery;
#[cfg(feature = "engine")]
pub mod dsp;
pub mod graph;
#[cfg(feature = "gui")]
pub mod gui;
#[cfg(feature = "engine")]
pub mod hal;
pub mod protocol;
pub mod types;

#[cfg(feature = "daemon")]
pub mod daemon;
#[cfg(feature = "engine")]
pub mod engine;
#[cfg(feature = "engine")]
pub mod virtual_devices;
