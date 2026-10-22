//! OpenEB-inspired event camera abstractions and file-backed devices.
//!
//! `openevt` is independently maintained and is not endorsed or sponsored by
//! Prophesee.
//!
//! This crate is a facade over the workspace crates:
//!
//! - `openevt-core` — HAL abstractions, types, algorithms, decoders (always on)
//! - `openevt-devices` — file-backed raw devices (the `devices` feature)
//! - `openevt-abi` — the stable plugin ABI and host adapters (the `plugins`
//!   and `framegen` features)
//! - `openevt-py` — Python bindings (the `python` feature)
//!
//! The `all` feature enables every optional component.

pub use derive_new::new;
pub use paste;

// Compatibility aliases from the pre-split monolith are gone; the modules
// now live in their dedicated workspace crates and are re-exported below.

// Macros moved to openevt-core; re-exported here so `openevt::property!` and
// `openevt::pack_facility!` keep working for downstream users.
pub use openevt_core::pack_facility;
pub use openevt_core::property;

pub use openevt_core::{algorithms, buffer, hal, types};

#[cfg(feature = "framegen")]
pub use openevt_abi::framegen;

#[cfg(feature = "devices")]
pub use openevt_devices as devices;
#[cfg(feature = "devices")]
pub use openevt_devices::device_macros;
#[cfg(feature = "devices")]
pub use openevt_devices::device_types;
#[cfg(feature = "devices")]
pub use openevt_devices::header;
#[cfg(feature = "devices")]
pub use openevt_devices::raw as raw;
#[cfg(feature = "devices")]
pub use raw::{BufferReplenisher, EventWindowIterator, IterAsync, IterSync, RawEventStreamDecoder, RawFileReader};

#[cfg(feature = "plugins")]
pub use openevt_abi::plugin::DevicePluginBox;
#[cfg(feature = "plugins")]
pub use openevt_abi::discovery::PluginRegistry;
#[cfg(feature = "plugins")]
pub use openevt_abi::device::PluginDevice;

#[cfg(feature = "python")]
pub use openevt_py as python;
