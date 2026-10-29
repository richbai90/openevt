//! Global types shared across openevt.
//!
//! These types are always available regardless of feature selection. They are
//! the single home for types needed by multiple modules or features: the event
//! scalars, the decoded event structs, and the shared callback/geometry types.
//!
//! Types that cross the plugin ABI additionally derive `StableAbi` and are
//! marked `#[repr(C)]` behind the relevant feature, so the core can be built
//! without `abi_stable`. Device-specific types live in [`crate::device_types`]
//! (gated by `devices`); algorithm types live in [`crate::algorithms`].

use crate::new;

#[cfg(feature = "plugins")]
use abi_stable::StableAbi;
#[cfg(feature = "plugins")]
use slotmap::{DefaultKey, Key, KeyData};

/// Fixed-width scalar used for decoded event coordinates.
pub type EventCoordinate = u64;
/// Fixed-width scalar used for event timestamps in device time units.
pub type EventTimestamp = u64;
/// Fixed-width scalar used for decoded event identifiers.
pub type EventId = u64;
/// Fixed-width scalar used for callback registration identifiers.
pub type CallbackId = u64;
/// Fixed-width scalar used for byte counts crossing public/plugin APIs.
pub type StreamLength = u64;
/// Fixed-width scalar used for event counts crossing public APIs.
pub type EventCount = u64;

/// Polarity of a change-detection or external-trigger event.
///
/// A `Polarity` is one well-defined byte: `Negative` is `0` and `Positive`
/// is `1`. Unlike `bool`, whose in-memory bit patterns are only partially
/// defined, every byte value of `Polarity` is a defined discriminant, and
/// values outside `{0, 1}` are rejected by [`zerocopy::TryFromBytes`]. That
/// checked validity is what makes byte-level reinterpretation sound.
#[cfg_attr(
    any(feature = "framegen", feature = "plugins"),
    derive(abi_stable::StableAbi)
)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Default,
    zerocopy::KnownLayout,
    zerocopy::Immutable,
    zerocopy::FromZeros,
    zerocopy::IntoBytes,
    zerocopy::Unaligned,
)]
#[repr(u8)]
pub enum Polarity {
    /// OFF event: brightness decreased.
    #[default]
    Negative = 0,
    /// ON event: brightness increased.
    Positive = 1,
}

impl Polarity {
    /// Returns `true` for [`Polarity::Positive`].
    #[inline]
    pub fn is_positive(self) -> bool {
        self == Self::Positive
    }

    /// Returns `true` for [`Polarity::Negative`].
    #[inline]
    pub fn is_negative(self) -> bool {
        self == Self::Negative
    }
}

impl From<bool> for Polarity {
    #[inline]
    fn from(value: bool) -> Self {
        if value { Self::Positive } else { Self::Negative }
    }
}

impl From<Polarity> for bool {
    #[inline]
    fn from(value: Polarity) -> Self {
        value.is_positive()
    }
}

impl std::ops::Not for Polarity {
    type Output = Self;

    #[inline]
    fn not(self) -> Self {
        match self {
            Self::Positive => Self::Negative,
            Self::Negative => Self::Positive,
        }
    }
}

/// Returned when a byte value is not a valid [`Polarity`] discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid polarity byte: {value}")]
pub struct InvalidPolarity {
    /// The rejected byte value.
    pub value: u8,
}

impl TryFrom<u8> for Polarity {
    type Error = InvalidPolarity;

    #[inline]
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Negative),
            1 => Ok(Self::Positive),
            other => Err(InvalidPolarity { value: other }),
        }
    }
}

#[cfg_attr(
    any(feature = "framegen", feature = "plugins"),
    derive(abi_stable::StableAbi)
)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    new,
    Default,
    Hash,
    zerocopy::KnownLayout,
    zerocopy::Immutable,
)]
#[repr(C)]
/// A decoded change-detection event.
pub struct EventCD {
    /// Horizontal pixel coordinate.
    pub x: EventCoordinate,
    /// Vertical pixel coordinate.
    pub y: EventCoordinate,
    /// Polarity of the change.
    pub p: Polarity,
    /// Event timestamp in device time units.
    pub t: EventTimestamp,
}

#[cfg_attr(feature = "plugins", derive(abi_stable::StableAbi))]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    new,
    zerocopy::KnownLayout,
    zerocopy::Immutable,
)]
#[repr(C)]
/// A decoded external-trigger event.
pub struct EventExtTrigger {
    /// Trigger polarity.
    pub p: Polarity,
    /// Event timestamp in device time units.
    pub t: EventTimestamp,
    /// Trigger identifier.
    pub id: EventId,
}

/// Callback that may mutate captured state.
pub type Cb<P> = Box<dyn for<'a> FnMut(P) + Send + Sync + 'static>;
/// Read-only callback.
pub type CbRo<P> = Box<dyn for<'a> Fn(P) + Send + Sync + 'static>;
/// Region represented as `(x, y, width, height)`.
pub type Region = (u32, u32, u32, u32);
/// Borrowed slice of change-detection events.
pub type EventSlice<'a> = &'a [EventCD];
/// Pixel enable/disable state used by pixel-mask facilities.
pub struct PixelMask {
    /// Horizontal pixel coordinate.
    x: u32,
    /// Vertical pixel coordinate.
    y: u32,
    /// Whether events from this pixel are enabled.
    enabled: bool,
}

impl PixelMask {
    /// Creates a pixel mask entry for `(x, y)`.
    pub fn new(x: u32, y: u32, enabled: bool) -> Self {
        PixelMask { x, y, enabled }
    }

    /// Enables the pixel.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disables the pixel.
    pub fn disable(&mut self) {
        self.enabled = false;
    }
}

/// FFI-safe wrapper around a `slotmap` key, used to cross the plugin ABI.
///
/// Only available when the `plugins` feature is enabled, since it exists solely
/// to move marker keys across the stable-ABI boundary.
#[cfg(feature = "plugins")]
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, StableAbi)]
pub struct FfiKey {
    pub data: u64,
}

/// Public re-export so external crates (e.g. plugins) can convert an
/// [`FfiKey`] back to a `slotmap::DefaultKey` without depending on
/// `slotmap` themselves.
#[cfg(feature = "plugins")]
pub use slotmap::DefaultKey as SlotmapKey;

#[cfg(feature = "plugins")]
impl From<DefaultKey> for FfiKey {
    #[inline]
    fn from(key: DefaultKey) -> Self {
        Self {
            data: key.data().as_ffi(),
        }
    }
}

#[cfg(feature = "plugins")]
impl From<FfiKey> for DefaultKey {
    #[inline]
    fn from(ffi_key: FfiKey) -> Self {
        KeyData::from_ffi(ffi_key.data).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zerocopy::{IntoBytes, TryFromBytes};

    #[test]
    fn polarity_round_trips_with_bool() {
        assert_eq!(Polarity::from(true), Polarity::Positive);
        assert_eq!(Polarity::from(false), Polarity::Negative);
        assert!(bool::from(Polarity::Positive));
        assert!(!bool::from(Polarity::Negative));
    }

    #[test]
    fn polarity_default_is_negative() {
        assert_eq!(Polarity::default(), Polarity::Negative);
    }

    #[test]
    fn not_flips_polarity() {
        assert_eq!(!Polarity::Positive, Polarity::Negative);
        assert_eq!(!Polarity::Negative, Polarity::Positive);
    }

    #[test]
    fn try_from_u8_accepts_only_valid_discriminants() {
        assert_eq!(Polarity::try_from(0u8), Ok(Polarity::Negative));
        assert_eq!(Polarity::try_from(1u8), Ok(Polarity::Positive));
        assert!(Polarity::try_from(2u8).is_err());
    }

    #[test]
    fn polarity_is_a_single_defined_byte() {
        assert_eq!(Polarity::Positive.as_bytes(), &[1]);
        assert_eq!(Polarity::Negative.as_bytes(), &[0]);
        assert_eq!(
            Polarity::try_read_from_bytes(&[0]),
            Ok(Polarity::Negative)
        );
        assert_eq!(
            Polarity::try_read_from_bytes(&[1]),
            Ok(Polarity::Positive)
        );
        assert!(Polarity::try_read_from_bytes(&[2]).is_err());
    }

    #[test]
    fn event_structs_keep_their_byte_layout() {
        assert_eq!(std::mem::size_of::<EventCD>(), 32);
        assert_eq!(std::mem::offset_of!(EventCD, x), 0);
        assert_eq!(std::mem::offset_of!(EventCD, y), 8);
        assert_eq!(std::mem::offset_of!(EventCD, p), 16);
        assert_eq!(std::mem::offset_of!(EventCD, t), 24);

        assert_eq!(std::mem::size_of::<EventExtTrigger>(), 24);
        assert_eq!(std::mem::offset_of!(EventExtTrigger, p), 0);
        assert_eq!(std::mem::offset_of!(EventExtTrigger, t), 8);
        assert_eq!(std::mem::offset_of!(EventExtTrigger, id), 16);
    }
}
