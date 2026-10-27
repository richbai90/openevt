//! Decoder implementations and decoder-related traits.
//!
//! The workspace currently treats decoders as the part of the HAL that turns
//! raw stream buffers into typed event batches. Decoders are gated by domain
//! features so builds can stay small: `evt3` enables the EVT3.0 decoder and
//! `decoder` enables the remaining decoder infrastructure. `full` enables
//! both.

pub mod dat;
#[cfg(feature = "decoder")]
pub mod evt2;
#[cfg(feature = "evt3")]
pub mod evt3;
#[cfg(feature = "decoder")]
pub mod raw_fmt_decoder;
