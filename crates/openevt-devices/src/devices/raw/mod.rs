//! File-backed raw event device implementation.
//!
//! This module parses raw-file headers, exposes stream and decoder facilities,
//! and provides batch/window iteration over decoded CD events.

mod decoder;
mod device;
mod facilities;
mod index;
mod iterator;
mod reader;
mod stream;

pub use decoder::RawEventStreamDecoder;
pub use iterator::*;
/// High-level convenience reader for raw event files.
pub use reader::RawFileReader;

#[cfg(test)]
mod tests;
