//! Audio input/output device access.

mod dsp;
#[cfg(target_arch = "wasm32")]
mod dsp_web;
mod input;
mod output;

pub use dsp::{DspOutput, DspOutputConfig, DspProcessor, open_dsp_output};
pub use input::*;
pub use output::*;
