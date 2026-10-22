//! Marker trait for raw format decoders.
//!
//! `RawFormatDecoder` is the trait object used by file-backed readers to treat
//! concrete decoders uniformly. A raw decoder must be able to:
//!
//! - accept raw byte buffers
//! - expose event subscription facilities
//! - expose the base decoder error stream and raw-event size information

use crate::hal::facilities::{
    EventSubscriptionFacility, RawDecoderFacility, RawEventStreamDecoderFacility,
};

/// Trait alias for the decoder capabilities required by raw file readers.
pub trait RawFormatDecoder:
    RawEventStreamDecoderFacility + RawDecoderFacility + EventSubscriptionFacility
{
}

// Blanket implementation for any type meeting the bounds
impl<T> RawFormatDecoder for T where
    T: RawEventStreamDecoderFacility + RawDecoderFacility + EventSubscriptionFacility
{
}
