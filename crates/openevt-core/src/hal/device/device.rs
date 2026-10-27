//! Device abstraction for the HAL.
//!
//! A device is a registry of facilities rather than a single trait object.
//! Consumers ask for a `FacilityType`, receive a `FacilityHandle`, and then
//! convert that handle into the exact facility trait object they need.

use crate::hal::facilities::{FacilityHandle, FacilityType};

/// A device exposes its capabilities through a registry of facilities.
pub trait Device {
    /// Retrieves a specific facility handle if it exists.
    ///
    /// Callers usually follow this with `TryFrom<FacilityHandle>` to recover
    /// the typed facility trait object they need.
    fn get_facility(&self, facility_type: FacilityType) -> Option<FacilityHandle>;

    /// Returns the list of facilities currently registered on the device.
    fn get_facilities(&self) -> Vec<FacilityType>;

    /// Registers a new facility and returns the previous value, if any.
    fn register_facility(
        &mut self,
        facility_type: FacilityType,
        facility_handle: FacilityHandle,
    ) -> Option<FacilityHandle>;
}
