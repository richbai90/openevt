//! File-backed implementations of core HAL facilities.
//!
//! These types adapt metadata found in a raw file header into the facility
//! interfaces expected by the OpenEB-inspired core. They expose what can be derived
//! directly from the file contents.

use crate::header::{Header, sensor_info_from_header};
use openevt_core::{
    hal::facilities::{
        ConnectionType, FacilityResult, GeometryFacility, HWIdentificationFacility, ROIFacility,
        SensorInfo, SystemInfo,
    },
    types::Region,
};
use std::sync::Arc;

/// File-backed ROI facility.
///
/// Raw files do not contain mutable camera ROI state. This implementation keeps
/// ROI values as local reader state so clients can use the common facility API
/// while processing a file.
#[derive(Clone, Default)]
pub(crate) struct RawReaderROI {
    enabled: bool,
    roi_: Option<Region>,
    rois_: Option<Vec<Region>>,
}

impl ROIFacility for RawReaderROI {
    fn get_enabled(&self) -> FacilityResult<bool> {
        Ok(self.enabled)
    }

    fn set_enabled(&mut self, value: bool) -> FacilityResult<()> {
        self.enabled = value;
        Ok(())
    }

    fn set_roi(&mut self, region: openevt_core::types::Region) -> FacilityResult<()> {
        self.roi_ = Some(region);
        Ok(())
    }

    fn set_rois(&mut self, regions: &[openevt_core::types::Region]) -> FacilityResult<()> {
        self.rois_ = Some(regions.to_vec());
        Ok(())
    }

    fn roi(&self) -> Option<Region> {
        self.roi_
    }

    fn rois(&self) -> Option<Vec<Region>> {
        self.rois_.clone()
    }
}

/// File-backed geometry facility backed by header dimensions.
pub(crate) struct RawReaderGeometry {
    width: i32,
    height: i32,
}

impl RawReaderGeometry {
    pub(crate) fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

impl GeometryFacility for RawReaderGeometry {
    fn get_width(&self) -> i32 {
        self.width
    }

    fn get_height(&self) -> i32 {
        self.height
    }
}

/// File-backed hardware-identification facility backed by header metadata.
pub(crate) struct RawReaderHWIdentification {
    header: Arc<Header>,
}

impl RawReaderHWIdentification {
    pub(crate) fn new(header: Arc<Header>) -> Self {
        Self { header }
    }
}

impl HWIdentificationFacility for RawReaderHWIdentification {
    fn get_system_id(&self) -> FacilityResult<i64> {
        let id = self
            .header
            .metadata
            .get("system_ID")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        Ok(id)
    }

    fn get_serial(&self) -> FacilityResult<String> {
        let serial = self
            .header
            .metadata
            .get("serial_number")
            .cloned()
            .unwrap_or_else(|| "ffffffffffffffff".to_string());
        Ok(serial)
    }

    fn get_sensor_info(&self) -> FacilityResult<SensorInfo> {
        Ok(sensor_info_from_header(&self.header))
    }

    fn get_system_info(&self) -> FacilityResult<SystemInfo> {
        Ok(SystemInfo {
            serial_number: self.get_serial().unwrap_or_default(),
            firmware_version: self
                .header
                .metadata
                .get("firmaware_version")
                .or_else(|| self.header.metadata.get("firmware_version"))
                .cloned()
                .unwrap_or_else(|| "x.x".to_string()),
        })
    }

    fn get_connection_type(&self) -> FacilityResult<ConnectionType> {
        Ok(ConnectionType::Unknown)
    }

    fn get_available_data_encoding_formats(&self) -> FacilityResult<Vec<String>> {
        Ok(vec![self.header.format.to_string()])
    }

    fn get_current_data_encoding_format(&self) -> FacilityResult<String> {
        Ok(self.header.format.to_string())
    }
}
