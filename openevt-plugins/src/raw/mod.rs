//! EVT3 raw-file replay device.
//!
//! Exposes a recorded event file through the device plugin ABI, so an
//! application can replay a capture anywhere it would otherwise open a camera.

pub mod plugin;

pub use plugin::{RAW_FILE_PLUGIN_NAME, RawFileDiscovery, RawFilePlugin, create_raw_discovery};
