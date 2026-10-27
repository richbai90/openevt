//! Standard device plugins for the `openevt` crate.
//!
//! This crate bundles the devices most applications want on hand — a software
//! event simulator and an EVT3 raw-file replay device — into a single plugin
//! library discovered through `OPENEVT_PLUGIN_PATH`.

#[global_allocator]
static GLOBAL: hotpath::CountingAllocator = hotpath::CountingAllocator::new();

pub mod module;
pub mod raw;
pub mod simulator;

#[cfg(feature = "python")]
mod python;
