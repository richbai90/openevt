//! OpenEB-inspired event camera abstractions and file-backed devices.
//!
//! `openevt` is independently maintained and is not endorsed or sponsored by
//! Prophesee.
//!
//! The core HAL is always available. File-backed raw devices are enabled by
//! the `devices` feature; the `all` feature enables every optional component.

pub use derive_new::new;
pub use paste;

// Compatibility aliases keep the folded modules' internal paths stable while
// allowing downstream code to use this crate as one package.
extern crate self as macros;
extern crate self as openevt_core;
extern crate self as utilities;

/// Declares getter/setter methods for facility properties.
#[macro_export]
macro_rules! property {
    (ro $name:ident : $ty:ty ; $($rest:tt)*) => {
        paste::paste! {
            #[doc = "Returns the current facility property."]
            fn [<get_ $name>](&self) -> FacilityResult<$ty>;
        }
        property!($($rest)*);
    };
    ($name:ident : $ty:ty ; $($rest:tt)*) => {
        paste::paste! {
            #[doc = "Returns the current facility property."]
            fn [<get_ $name>](&self) -> FacilityResult<$ty>;
            #[doc = "Updates the facility property."]
            fn [<set_ $name>](&mut self, value: $ty) -> FacilityResult<()>;
        }
        property!($($rest)*);
    };
    () => {};
}

/// Wraps a concrete facility in the matching `FacilityHandle` variant.
#[macro_export]
macro_rules! pack_facility {
    (mut $variant:ident, $instance:expr) => {
        FacilityHandle::$variant(std::sync::Arc::new(std::sync::RwLock::new($instance)))
    };
    (ro $variant:ident, $instance:expr) => {
        FacilityHandle::$variant(std::sync::Arc::new($instance))
    };
}

/// Recyclable buffers used by event decoders and dispatchers.
pub mod buffer;
/// ABI-stable event-to-frame generation interfaces.
#[cfg(feature = "framegen")]
pub mod framegen;
/// Shared HAL abstractions.
pub mod hal;
/// Global event, scalar, callback, and geometry types shared across features.
pub mod types;

#[cfg(feature = "devices")]
#[path = "devices/device_macros.rs"]
/// Macros used to implement common device plumbing.
pub mod device_macros;
#[cfg(feature = "devices")]
#[path = "devices/header.rs"]
/// Raw event-file header parsing and metadata conversion.
pub mod header;
#[cfg(feature = "devices")]
#[path = "devices/raw/mod.rs"]
mod raw;
#[cfg(feature = "devices")]
#[path = "devices/types.rs"]
pub mod device_types;

pub mod algorithms;

#[cfg(feature = "python")]
/// Python bindings for the device and event APIs.
pub mod python;

#[cfg(feature = "devices")]
pub use raw::{
    BufferReplenisher, EventWindowIterator, IterAsync, IterSync, RawEventStreamDecoder,
    RawFileReader,
};
