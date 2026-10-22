//! File-backed raw event device implementations for the openevt stack.
//!
//! `openevt-devices` hosts the concrete device drivers and file readers: the
//! raw event-file handler, header parsing, decoder wiring, and batch/window
//! iteration over decoded CD events. It depends only on `openevt-core`.
//!
//! The `sample.raw` test fixture lives in this crate's `tests/` directory.

#[path = "devices/device_macros.rs"]
/// Macros used to implement common device plumbing.
pub mod device_macros;
#[path = "devices/header.rs"]
/// Raw event-file header parsing and metadata conversion.
pub mod header;
#[path = "devices/raw/mod.rs"]
/// File-backed raw event device and readers.
pub mod raw;
#[path = "devices/types.rs"]
/// Device-specific types used by the raw file reader and iterator layers.
pub mod device_types;

pub use raw::{
    BufferReplenisher, EventWindowIterator, IterAsync, IterSync, RawEventStreamDecoder,
    RawFileReader,
};
