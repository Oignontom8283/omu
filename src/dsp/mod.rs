//! Audio building blocks. Everything operates on interleaved `f32` slices, allocates nothing once built, and has
//! no OS dependency: it can run in a real-time callback.

pub mod gain;
pub mod mix;
pub mod remix;
pub mod resample;

pub use gain::{db_to_linear, linear_to_db, SmoothedGain};
pub use mix::{add_into, peak};
pub use remix::remix;
pub use resample::{RateController, StreamResampler};
