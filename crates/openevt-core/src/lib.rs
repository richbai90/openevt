//! Fundamental types, HAL traits, and event decoders for the openevt stack.
//!
//! `openevt-core` is the foundation of the workspace: it is self-contained and
//! depends on no other openevt crate. It hosts the shared event types, the
//! facility-based HAL abstraction, the stable-ABI-safe scalar wrappers, and the
//! event stream decoders (EVT3.0 and friends). Hardware implementations,
//! plugin bridges, and Python bindings build on top of this crate.
//!
//! Domain features keep decoders optional: `evt3` gates the EVT3.0 decoder,
//! `decoder` gates the remaining decoder infrastructure, and `full` enables
//! both. The `plugins` feature enables `StableAbi` derivations for types that
//! cross the plugin ABI; `framegen` does the same for frame generation.

pub use derive_new::new;
pub use paste;

/// Recyclable buffers used by event decoders and dispatchers.
pub mod buffer;
/// Event transformations and filters over decoded event streams.
pub mod algorithms;
/// Shared HAL abstractions.
pub mod hal;
/// Global event, scalar, callback, and geometry types shared across features.
pub mod types;

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
