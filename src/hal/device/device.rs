//! Device abstraction for the HAL.
//!
//! A device is a registry of facilities rather than a monolithic trait object.
//! Consumers ask for a `FacilityType`, receive a `FacilityHandle`, and then
//! convert that handle into the exact facility trait object they need.

use crate::hal::facilities::{FacilityHandle, FacilityType};

/// A device exposes its capabilities through a registry of facilities.
pub trait Device {
    /// Retrieves a specific facility handle if it exists.
    ///
    /// Callers usually follow this with `TryFrom<FacilityHandle>` to recover the
    /// typed facility handle they need.
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

#[cfg(feature = "plugins")]
mod plugin_adapter {
    use super::Device;
    use crate::hal::device::plugin::{DevicePluginBox, PluginFacilityHandle, PluginFacilityType};
    use crate::hal::facilities::{
        DecoderErrorCallback, EventCDCallback, EventExtTriggerCallback, EventSubscriptionFacility,
        FacilityError, FacilityHandle, FacilityType, GeometryFacility, HWIdentificationFacility,
        ROIFacility, RawDecoderFacility, RawEventStreamDecoderFacility, RawEventStreamFacility,
        SensorInfo, StreamBuffer, SystemInfo,
    };
    use crate::types::EventTimestamp;

    use abi_stable::{
        std_types::{RSlice, RString},
        type_level::downcasting::TD_Opaque,
    };
    use slotmap::DefaultKey;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Host-layer view of an ABI plugin device.
    ///
    /// This is an adapter, not a transmute: every ABI facility is explicitly
    /// converted to the matching native handle and unsupported ABI facilities
    /// are never presented as native facilities.
    pub struct PluginDevice {
        plugin: DevicePluginBox,
        registered: HashMap<FacilityType, FacilityHandle>,
    }

    impl PluginDevice {
        /// Wraps an ABI plugin in the native [`Device`] interface.
        pub fn new(plugin: DevicePluginBox) -> Self {
            Self {
                plugin,
                registered: HashMap::new(),
            }
        }

        fn native_handle(&self, kind: PluginFacilityType) -> Option<FacilityHandle> {
            let handle = self.plugin.get_facility_handle(kind).into_option()?;
            match handle {
                PluginFacilityHandle::Geometry(geometry) => Some(FacilityHandle::GeometryFacility(
                    Arc::new(PluginGeometryFacility { geometry }),
                )),
                PluginFacilityHandle::Roi(roi) => Some(FacilityHandle::ROIFacility(Arc::new(
                    std::sync::RwLock::new(PluginRoiFacility { roi }),
                ))),
                PluginFacilityHandle::RawEventStream(stream) => {
                    Some(FacilityHandle::RawEventStreamFacility(Arc::new(
                        std::sync::RwLock::new(PluginRawEventStreamFacility { stream }),
                    )))
                }
                PluginFacilityHandle::HardwareIdentification(identity) => {
                    Some(FacilityHandle::HWIdentificationFacility(Arc::new(
                        PluginHardwareIdentificationFacility { identity },
                    )))
                }
                PluginFacilityHandle::RawEventStreamDecoder(decoder) => {
                    Some(FacilityHandle::RawEventStreamDecoderFacility(Arc::new(
                        std::sync::RwLock::new(PluginRawEventStreamDecoderFacility { decoder }),
                    )))
                }
                PluginFacilityHandle::EventSubscription(decoder) => {
                    Some(FacilityHandle::EventSubscriptionFacility(Arc::new(
                        std::sync::RwLock::new(PluginEventSubscriptionFacility { decoder }),
                    )))
                }
                _ => None,
            }
        }
    }

    impl From<DevicePluginBox> for PluginDevice {
        fn from(plugin: DevicePluginBox) -> Self {
            Self::new(plugin)
        }
    }

    struct PluginGeometryFacility {
        geometry: crate::hal::device::plugin::PluginGeometryFacilityBox,
    }

    impl GeometryFacility for PluginGeometryFacility {
        fn get_width(&self) -> i32 {
            self.geometry.get_width() as i32
        }

        fn get_height(&self) -> i32 {
            self.geometry.get_height() as i32
        }
    }

    struct PluginRoiFacility {
        roi: crate::hal::device::plugin::PluginROIFacilityBox,
    }

    impl ROIFacility for PluginRoiFacility {
        fn get_enabled(&self) -> crate::hal::facilities::FacilityResult<bool> {
            self.roi
                .get_enabled()
                .into_result()
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn set_enabled(&mut self, value: bool) -> crate::hal::facilities::FacilityResult<()> {
            self.roi
                .set_enabled(value)
                .into_result()
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn set_roi(
            &mut self,
            region: crate::types::Region,
        ) -> crate::hal::facilities::FacilityResult<()> {
            let region = crate::hal::device::plugin::PluginRegion {
                x: region.0,
                y: region.1,
                width: region.2,
                height: region.3,
            };
            self.roi
                .set_roi(region)
                .into_result()
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn set_rois(
            &mut self,
            regions: &[crate::types::Region],
        ) -> crate::hal::facilities::FacilityResult<()> {
            let regions: Vec<_> = regions
                .iter()
                .map(|region| crate::hal::device::plugin::PluginRegion {
                    x: region.0,
                    y: region.1,
                    width: region.2,
                    height: region.3,
                })
                .collect();
            self.roi
                .set_rois(regions.as_slice().into())
                .into_result()
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn roi(&self) -> Option<crate::types::Region> {
            self.roi
                .roi()
                .into_option()
                .map(|r| (r.x, r.y, r.width, r.height))
        }
        fn rois(&self) -> Option<Vec<crate::types::Region>> {
            let regions = self.roi.rois();
            if regions.is_empty() {
                None
            } else {
                Some(
                    regions
                        .into_iter()
                        .map(|r| (r.x, r.y, r.width, r.height))
                        .collect(),
                )
            }
        }
    }

    struct PluginEventSink {
        cd: Option<Mutex<EventCDCallback>>,
        ext: Option<Mutex<EventExtTriggerCallback>>,
    }

    impl crate::hal::device::plugin::EventBatchSink for PluginEventSink {
        fn on_cd_events(&self, events: RSlice<'_, crate::types::EventCD>) {
            if let Some(callback) = &self.cd {
                (callback.lock().unwrap())(events.as_slice());
            }
        }
        fn on_ext_events(&self, events: RSlice<'_, crate::types::EventExtTrigger>) {
            if let Some(callback) = &self.ext {
                (callback.lock().unwrap())(events.as_slice());
            }
        }
    }

    struct PluginEventSubscriptionFacility {
        decoder: crate::hal::device::plugin::PluginEventSubscriptionFacilityBox,
    }

    impl EventSubscriptionFacility for PluginEventSubscriptionFacility {
        fn subscribe_to_cd_events(
            &mut self,
            callback: EventCDCallback,
        ) -> crate::hal::facilities::FacilityResult<()> {
            let sink = crate::hal::device::plugin::EventBatchSink_TO::from_value(
                PluginEventSink {
                    cd: Some(Mutex::new(callback)),
                    ext: None,
                },
                TD_Opaque,
            );
            self.decoder
                .subscribe_to_cd_events(sink)
                .into_result()
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn subscribe_to_ext_events(
            &mut self,
            callback: EventExtTriggerCallback,
        ) -> crate::hal::facilities::FacilityResult<()> {
            let sink = crate::hal::device::plugin::EventBatchSink_TO::from_value(
                PluginEventSink {
                    cd: None,
                    ext: Some(Mutex::new(callback)),
                },
                TD_Opaque,
            );
            self.decoder
                .subscribe_to_ext_events(sink)
                .into_result()
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
    }

    struct PluginDecoderErrorSink {
        callback: Mutex<DecoderErrorCallback>,
    }

    impl crate::hal::device::plugin::PluginDecoderErrorSink for PluginDecoderErrorSink {
        fn on_error(&self, message: RString) {
            let error: crate::hal::errors::SharedError =
                Arc::new(std::io::Error::other(message.to_string()));
            (self.callback.lock().unwrap())(error);
        }
    }

    struct PluginRawEventStreamDecoderFacility {
        decoder: crate::hal::device::plugin::PluginRawEventStreamDecoderFacilityBox,
    }

    impl RawDecoderFacility for PluginRawEventStreamDecoderFacility {
        fn subscribe_to_protocol_violation(
            &mut self,
            callback: DecoderErrorCallback,
        ) -> crate::hal::facilities::FacilityResult<()> {
            let sink = crate::hal::device::plugin::PluginDecoderErrorSink_TO::from_value(
                PluginDecoderErrorSink {
                    callback: Mutex::new(callback),
                },
                TD_Opaque,
            );
            self.decoder
                .subscribe_to_protocol_violation(sink)
                .into_result()
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn get_raw_event_size_bytes(&self) -> crate::hal::facilities::FacilityResult<u8> {
            Ok(self.decoder.get_raw_event_size_bytes())
        }
    }

    impl RawEventStreamDecoderFacility for PluginRawEventStreamDecoderFacility {
        fn decode(&mut self, raw_data: &[u8]) -> crate::hal::facilities::FacilityResult<()> {
            self.decoder
                .decode(raw_data.into())
                .into_result()
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn get_last_timestamp(&self) -> EventTimestamp {
            self.decoder.get_last_timestamp()
        }
        fn get_timestamp_shift(&self) -> Option<EventTimestamp> {
            self.decoder.get_timestamp_shift().into_option()
        }
        fn is_time_shifting_enabled(&self) -> bool {
            self.decoder.is_time_shifting_enabled()
        }
        fn reset_last_timestamp(&mut self, timestamp: EventTimestamp) {
            self.decoder.reset_last_timestamp(timestamp);
        }
        fn reset_timestamp_shift(&mut self, shift: EventTimestamp) {
            self.decoder.reset_timestamp_shift(shift);
        }
        fn is_decoded_event_stream_indexable(&self) -> bool {
            self.decoder.is_decoded_event_stream_indexable()
        }

        fn add_marker(&mut self, timestamp: EventTimestamp) -> slotmap::DefaultKey {
            self.decoder.add_marker(timestamp).into()
        }

        fn remove_marker(&mut self, key: DefaultKey) -> Option<EventTimestamp> {
            self.decoder.remove_marker(key.into()).into()
        }
    }

    struct PluginHardwareIdentificationFacility {
        identity: crate::hal::device::plugin::PluginHardwareIdentificationFacilityBox,
    }

    impl HWIdentificationFacility for PluginHardwareIdentificationFacility {
        fn get_serial(&self) -> crate::hal::facilities::FacilityResult<String> {
            self.identity
                .get_serial()
                .into_result()
                .map(Into::into)
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn get_system_id(&self) -> crate::hal::facilities::FacilityResult<i64> {
            self.identity
                .get_system_id()
                .into_result()
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn get_sensor_info(&self) -> crate::hal::facilities::FacilityResult<SensorInfo> {
            self.identity
                .get_sensor_info()
                .into_result()
                .map(|info| SensorInfo {
                    name: info.name.into(),
                    integrator: info.integrator.into(),
                    version: info.version.into(),
                })
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn get_system_info(&self) -> crate::hal::facilities::FacilityResult<SystemInfo> {
            self.identity
                .get_system_info()
                .into_result()
                .map(|info| SystemInfo {
                    serial_number: info.serial_number.into(),
                    firmware_version: info.firmware_version.into(),
                })
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn get_connection_type(
            &self,
        ) -> crate::hal::facilities::FacilityResult<crate::hal::facilities::ConnectionType>
        {
            self.identity
                .get_connection_type()
                .into_result()
                .map(|kind| match kind {
                    crate::hal::device::discovery::ConnectionType::Usb => {
                        crate::hal::facilities::ConnectionType::Usb
                    }
                    crate::hal::device::discovery::ConnectionType::Mipi => {
                        crate::hal::facilities::ConnectionType::Mipi
                    }
                    crate::hal::device::discovery::ConnectionType::Proprietary => {
                        crate::hal::facilities::ConnectionType::Proprietary
                    }
                    _ => crate::hal::facilities::ConnectionType::Unknown,
                })
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn get_available_data_encoding_formats(
            &self,
        ) -> crate::hal::facilities::FacilityResult<Vec<String>> {
            self.identity
                .get_available_data_encoding_formats()
                .into_result()
                .map(|formats| formats.into_iter().map(Into::into).collect())
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn get_current_data_encoding_format(
            &self,
        ) -> crate::hal::facilities::FacilityResult<String> {
            self.identity
                .get_current_data_encoding_format()
                .into_result()
                .map(Into::into)
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
    }

    struct PluginRawEventStreamFacility {
        stream: crate::hal::device::plugin::PluginRawEventStreamFacilityBox,
    }

    impl RawEventStreamFacility for PluginRawEventStreamFacility {
        fn start(&mut self) -> crate::hal::facilities::FacilityResult<()> {
            self.stream
                .start()
                .into_result()
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn stop(&mut self) -> crate::hal::facilities::FacilityResult<()> {
            self.stream
                .stop()
                .into_result()
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn poll_buffer(&mut self) -> crate::hal::facilities::FacilityResult<StreamBuffer> {
            self.stream
                .poll_buffer()
                .into_result()
                .map(|b| (b.data.into(), b.valid_len))
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
        fn wait_next_buffer(&mut self) -> crate::hal::facilities::FacilityResult<StreamBuffer> {
            self.stream
                .wait_next_buffer()
                .into_result()
                .map(|b| (b.data.into(), b.valid_len))
                .map_err(|e| FacilityError::Plugin(e.to_string()))
        }
    }

    impl Device for PluginDevice {
        fn get_facility(&self, facility_type: FacilityType) -> Option<FacilityHandle> {
            self.registered.get(&facility_type).cloned().or_else(|| {
                let kind = match facility_type {
                    FacilityType::GeometryFacility => PluginFacilityType::Geometry,
                    FacilityType::ROIFacility => PluginFacilityType::Roi,
                    FacilityType::RawEventStreamFacility => PluginFacilityType::RawEventStream,
                    FacilityType::HWIdentificationFacility => {
                        PluginFacilityType::HardwareIdentification
                    }
                    FacilityType::RawEventStreamDecoderFacility => {
                        PluginFacilityType::RawEventStreamDecoder
                    }
                    FacilityType::EventSubscriptionFacility => {
                        PluginFacilityType::EventSubscription
                    }
                    _ => return None,
                };
                self.native_handle(kind)
            })
        }

        fn get_facilities(&self) -> Vec<FacilityType> {
            self.plugin
                .get_facilities()
                .into_iter()
                .filter_map(|kind| {
                    let native = match kind {
                        PluginFacilityType::Geometry => FacilityType::GeometryFacility,
                        PluginFacilityType::Roi => FacilityType::ROIFacility,
                        PluginFacilityType::RawEventStream => FacilityType::RawEventStreamFacility,
                        PluginFacilityType::HardwareIdentification => {
                            FacilityType::HWIdentificationFacility
                        }
                        PluginFacilityType::RawEventStreamDecoder => {
                            FacilityType::RawEventStreamDecoderFacility
                        }
                        PluginFacilityType::EventSubscription => {
                            FacilityType::EventSubscriptionFacility
                        }
                        _ => return None,
                    };
                    self.get_facility(native).map(|_| native)
                })
                .collect()
        }

        fn register_facility(
            &mut self,
            facility_type: FacilityType,
            facility_handle: FacilityHandle,
        ) -> Option<FacilityHandle> {
            self.registered.insert(facility_type, facility_handle)
        }
    }

    pub use PluginDevice as HostPluginDevice;
}

#[cfg(feature = "plugins")]
pub use plugin_adapter::HostPluginDevice;
