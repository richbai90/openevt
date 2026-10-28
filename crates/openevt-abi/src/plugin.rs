//! Stable ABI contract for third-party device plugins.
//!
//! This module is intentionally separate from the native HAL traits. Native
//! facilities currently expose Rust-only types (crossbeam receivers, borrowed
//! slices and `Any` downcasts). Plugins therefore use ABI-safe, type-erased
//! facility traits whose implementations remain entirely inside the plugin.

use abi_stable::{
    StableAbi,
    library::RootModule,
    package_version_strings, sabi_trait,
    sabi_types::VersionStrings,
    std_types::{RBox, ROption, RResult, RSlice, RStr, RString, RVec},
};

use openevt_core::hal::device::discovery::ConnectionType;
use openevt_core::types::{EventCD, EventExtTrigger, EventTimestamp, StreamLength};
use openevt_core::{hal::facilities::FacilityType, types::FfiKey};

/// One named plugin creation value.
///
/// The value is intentionally optional even when a schema marks it required:
/// the application can build a complete form before the user has filled it in,
/// and the plugin performs the final validation at its boundary.
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
#[repr(C)]
pub struct PluginConfigurationValue {
    /// Schema parameter name.
    pub name: RString,
    /// User-provided value, or `None` while it has not been supplied.
    pub value: ROption<RString>,
}

/// Device identity plus the optional values used to create that device.
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
#[repr(C)]
pub struct PluginConfiguration {
    /// Discovery identity of the device to open.
    pub serial: RString,
    /// Configuration values keyed by the plugin's schema parameter names.
    pub values: RVec<PluginConfigurationValue>,
}

/// ABI representation of [`openevt_core::hal::device::discovery::PluginCameraDescription`]. Keeping this separate
/// means the native HAL remains usable without enabling the plugin feature.
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
#[repr(C)]
pub struct PluginCameraDescriptionAbi {
    /// Device serial number.
    pub serial: RString,
    /// Transport used by the device.
    pub connection: ConnectionType,
}

/// Sensor geometry exposed over the plugin ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi)]
#[repr(C)]
pub struct PluginGeometry {
    /// Sensor width in pixels.
    pub width: u32,
    /// Sensor height in pixels.
    pub height: u32,
}

/// Sensor identity metadata exposed over the plugin ABI.
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
#[repr(C)]
pub struct PluginSensorInfo {
    /// Sensor name.
    pub name: RString,
    /// Sensor integrator or manufacturer.
    pub integrator: RString,
    /// Sensor generation or version.
    pub version: RString,
}

/// System and firmware metadata exposed over the plugin ABI.
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
#[repr(C)]
pub struct PluginSystemInfo {
    /// Device serial number reported by the system.
    pub serial_number: RString,
    /// Firmware version.
    pub firmware_version: RString,
}

/// Region of interest represented in ABI-safe scalar fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi)]
#[repr(C)]
pub struct PluginRegion {
    /// Region origin on the x axis.
    pub x: u32,
    /// Region origin on the y axis.
    pub y: u32,
    /// Region width.
    pub width: u32,
    /// Region height.
    pub height: u32,
}

/// A buffer returned by a plugin stream facility.
///
/// Native stream facilities return borrowed buffers. A plugin cannot return a
/// borrow tied to its internal stream across the ABI, so the ABI form owns the
/// bytes and reports how many contain valid data.
#[derive(Debug, Clone, PartialEq, Eq, StableAbi)]
#[repr(C)]
pub struct PluginStreamBuffer {
    /// ABI-safe storage containing the raw bytes.
    pub data: RVec<u8>,
    /// Number of valid bytes in `data`.
    pub valid_len: StreamLength,
}

impl From<PluginCameraDescriptionAbi> for openevt_core::hal::device::discovery::PluginCameraDescription {
    fn from(value: PluginCameraDescriptionAbi) -> Self {
        Self {
            serial: value.serial.into(),
            connection: value.connection,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi)]
#[repr(C)]
/// ABI-visible capability identifier advertised by a device plugin.
pub enum PluginFacilityType {
    /// Anti-flicker controls.
    AntiFlicker,
    /// Raw decoder protocol support.
    RawDecoder,
    /// Camera synchronization controls.
    CameraSync,
    /// Typed raw-event decoder capability.
    RawEventDecoder,
    /// Digital crop controls.
    DigitalCrop,
    /// Per-pixel event masking.
    DigitalEventMask,
    /// Event-rate controller.
    ERCModule,
    /// CD-event decoder.
    CDEventDecoder,
    /// External-trigger decoder.
    TriggerEventDecoder,
    /// ERC counter decoder.
    ERCCounterEventDecoder,
    /// RGB frame decoder.
    RGBEventFrameDecoder,
    /// Monochrome frame decoder.
    MonoEventFrameDecoder,
    /// Event-rate activity filter.
    EventRateActivityFilterModule,
    /// Event-trail filter.
    EventTrailFilterModule,
    /// Hardware register access.
    HWRegister,
    /// Low-level bias controls.
    LLBiases,
    /// HAL software metadata.
    HALSoftwareInfo,
    /// Plugin software metadata.
    PluginSoftwareInfo,
    /// Monitoring data.
    Monitoring,
    /// ROI pixel masks.
    ROIPixelMask,
    /// External trigger input.
    TriggerIn,
    /// External trigger output.
    TriggerOut,
    /// Sensor geometry.
    Geometry,
    /// Hardware identification.
    HardwareIdentification,
    /// Raw event stream input.
    RawEventStream,
    /// Raw event stream decoder.
    RawEventStreamDecoder,
    /// Decoded-event subscription callbacks.
    EventSubscription,
    /// Region-of-interest controls.
    Roi,
    /// Timestamp indexing.
    Index,
    /// Timestamp seeking.
    Seek,
    /// Seeking to external triggers.
    ExternalTriggerSeek,
    /// Unknown or plugin-specific capability.
    Other,
}

impl PluginFacilityType {
    /// Every native facility key understood by the plugin ABI.
    ///
    /// Plugin-only capabilities such as indexing and seeking are deliberately
    /// excluded because they do not correspond to native HAL facilities.
    pub const ALL: [Self; 28] = [
        Self::AntiFlicker,
        Self::RawDecoder,
        Self::CameraSync,
        Self::RawEventDecoder,
        Self::DigitalCrop,
        Self::DigitalEventMask,
        Self::ERCModule,
        Self::CDEventDecoder,
        Self::TriggerEventDecoder,
        Self::ERCCounterEventDecoder,
        Self::RGBEventFrameDecoder,
        Self::MonoEventFrameDecoder,
        Self::EventRateActivityFilterModule,
        Self::EventTrailFilterModule,
        Self::RawEventStream,
        Self::RawEventStreamDecoder,
        Self::EventSubscription,
        Self::Geometry,
        Self::HALSoftwareInfo,
        Self::HardwareIdentification,
        Self::HWRegister,
        Self::LLBiases,
        Self::Monitoring,
        Self::PluginSoftwareInfo,
        Self::Roi,
        Self::ROIPixelMask,
        Self::TriggerIn,
        Self::TriggerOut,
    ];

    /// Returns whether this capability has a native HAL counterpart.
    pub const fn is_native(self) -> bool {
        matches!(
            self,
            Self::AntiFlicker
                | Self::RawDecoder
                | Self::CameraSync
                | Self::RawEventDecoder
                | Self::DigitalCrop
                | Self::DigitalEventMask
                | Self::ERCModule
                | Self::CDEventDecoder
                | Self::TriggerEventDecoder
                | Self::ERCCounterEventDecoder
                | Self::RGBEventFrameDecoder
                | Self::MonoEventFrameDecoder
                | Self::EventRateActivityFilterModule
                | Self::EventTrailFilterModule
                | Self::RawEventStream
                | Self::RawEventStreamDecoder
                | Self::EventSubscription
                | Self::Geometry
                | Self::HALSoftwareInfo
                | Self::HardwareIdentification
                | Self::HWRegister
                | Self::LLBiases
                | Self::Monitoring
                | Self::PluginSoftwareInfo
                | Self::Roi
                | Self::ROIPixelMask
                | Self::TriggerIn
                | Self::TriggerOut
        )
    }
}

impl From<FacilityType> for PluginFacilityType {
    fn from(value: FacilityType) -> Self {
        match value {
            FacilityType::AntiFlickerFacility => Self::AntiFlicker,
            FacilityType::RawDecoderFacility => Self::RawDecoder,
            FacilityType::CameraSyncFacility => Self::CameraSync,
            FacilityType::RawEventDecoderFacility => Self::RawEventDecoder,
            FacilityType::DigitalCropFacility => Self::DigitalCrop,
            FacilityType::DigitalEventMaskFacility => Self::DigitalEventMask,
            FacilityType::ERCModuleFacility => Self::ERCModule,
            FacilityType::CDEventDecoderFacility => Self::CDEventDecoder,
            FacilityType::TriggerEventDecoderFaciliy => Self::TriggerEventDecoder,
            FacilityType::ERCCounterEventDecoderFacility => Self::ERCCounterEventDecoder,
            FacilityType::RGBEventFrameDecoderFacility => Self::RGBEventFrameDecoder,
            FacilityType::MonoEventFrameDecoderFacility => Self::MonoEventFrameDecoder,
            FacilityType::EventRateActivityFilterModuleFacility => {
                Self::EventRateActivityFilterModule
            }
            FacilityType::EventTrailFilterModuleFacility => Self::EventTrailFilterModule,
            FacilityType::RawEventStreamFacility => Self::RawEventStream,
            FacilityType::RawEventStreamDecoderFacility => Self::RawEventStreamDecoder,
            FacilityType::EventSubscriptionFacility => Self::EventSubscription,
            FacilityType::GeometryFacility => Self::Geometry,
            FacilityType::HALSoftwareInfoFacility => Self::HALSoftwareInfo,
            FacilityType::HWIdentificationFacility => Self::HardwareIdentification,
            FacilityType::HWRegisterFacility => Self::HWRegister,
            FacilityType::LLBiasesFacility => Self::LLBiases,
            FacilityType::MonitoringFacility => Self::Monitoring,
            FacilityType::PluginSoftwareInfoFacility => Self::PluginSoftwareInfo,
            FacilityType::ROIFacility => Self::Roi,
            FacilityType::ROIPixelMaskFacility => Self::ROIPixelMask,
            FacilityType::TriggerInFacility => Self::TriggerIn,
            FacilityType::TriggerOutFacility => Self::TriggerOut,
        }
    }
}

/// FFI-safe capability descriptor retained for capability inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi)]
#[repr(C)]
pub struct PluginFacility {
    /// Capability represented by this descriptor.
    pub facility_type: PluginFacilityType,
}

impl PluginFacility {
    /// Creates a capability descriptor for `facility_type`.
    pub const fn new(facility_type: PluginFacilityType) -> Self {
        Self { facility_type }
    }
}

/// Callback sink used instead of exposing crossbeam channels across the ABI.
/// Each call represents one decoded batch.
#[sabi_trait]
pub trait EventBatchSink: Send + Sync {
    /// Receives a decoded CD-event batch.
    fn on_cd_events(&self, events: RSlice<'_, EventCD>);
    /// Receives a decoded external-trigger batch.
    fn on_ext_events(&self, events: RSlice<'_, EventExtTrigger>);
}

/// ABI-safe boxed event-batch sink.
pub type EventBatchSinkBox = EventBatchSink_TO<'static, RBox<()>>;

/// ABI counterpart of the native geometry facility.
#[sabi_trait]
pub trait PluginGeometryFacility: Send + Sync {
    /// Returns the sensor width in pixels.
    fn get_width(&self) -> u32;
    /// Returns the sensor height in pixels.
    fn get_height(&self) -> u32;
}

/// ABI-safe boxed geometry facility.
pub type PluginGeometryFacilityBox = PluginGeometryFacility_TO<'static, RBox<()>>;

/// ABI counterpart of the native hardware-identification facility.
#[sabi_trait]
pub trait PluginHardwareIdentificationFacility: Send + Sync {
    /// Returns the device serial number.
    fn get_serial(&self) -> RResult<RString, RString>;
    /// Returns the device-specific system identifier.
    fn get_system_id(&self) -> RResult<i64, RString>;
    /// Returns sensor metadata.
    fn get_sensor_info(&self) -> RResult<PluginSensorInfo, RString>;
    /// Returns system and firmware metadata.
    fn get_system_info(&self) -> RResult<PluginSystemInfo, RString>;
    /// Returns the physical or software connection type.
    fn get_connection_type(&self) -> RResult<ConnectionType, RString>;
    /// Lists supported data encodings.
    fn get_available_data_encoding_formats(&self) -> RResult<RVec<RString>, RString>;
    /// Returns the active data encoding.
    fn get_current_data_encoding_format(&self) -> RResult<RString, RString>;
}

pub type PluginHardwareIdentificationFacilityBox =
    PluginHardwareIdentificationFacility_TO<'static, RBox<()>>;

#[sabi_trait]
pub trait PluginHALSoftwareInfoFacility: Send + Sync {
    fn get_version(&self) -> RString;
}
/// ABI-safe boxed HAL software-information facility.
pub type PluginHALSoftwareInfoFacilityBox = PluginHALSoftwareInfoFacility_TO<'static, RBox<()>>;

#[sabi_trait]
pub trait PluginPluginSoftwareInfoFacility: Send + Sync {
    fn get_plugin_name(&self) -> RString;
    fn get_version(&self) -> RString;
}
/// ABI-safe boxed plugin software-information facility.
pub type PluginPluginSoftwareInfoFacilityBox =
    PluginPluginSoftwareInfoFacility_TO<'static, RBox<()>>;

#[sabi_trait]
pub trait PluginMonitoringFacility: Send + Sync {
    fn get_temperature(&self) -> RResult<i32, RString>;
    fn get_illumination(&self) -> RResult<i32, RString>;
}
/// ABI-safe boxed monitoring facility.
pub type PluginMonitoringFacilityBox = PluginMonitoringFacility_TO<'static, RBox<()>>;

#[sabi_trait]
pub trait PluginROIFacility: Send + Sync {
    fn get_enabled(&self) -> RResult<bool, RString>;
    fn set_enabled(&mut self, value: bool) -> RResult<(), RString>;
    fn set_roi(&mut self, region: PluginRegion) -> RResult<(), RString>;
    fn set_rois(&mut self, regions: RSlice<'_, PluginRegion>) -> RResult<(), RString>;
    fn roi(&self) -> ROption<PluginRegion>;
    fn rois(&self) -> RVec<PluginRegion>;
}
/// ABI-safe boxed region-of-interest facility.
pub type PluginROIFacilityBox = PluginROIFacility_TO<'static, RBox<()>>;

/// ABI counterpart of the native event-stream facility.
#[sabi_trait]
pub trait PluginRawEventStreamFacility: Send + Sync {
    /// Starts the raw stream.
    fn start(&mut self) -> RResult<(), RString>;
    /// Stops the raw stream.
    fn stop(&mut self) -> RResult<(), RString>;
    /// Polls for the next raw buffer.
    ///
    /// `ROk(RNone)` means the stream reached its end; `RErr` is reserved for
    /// real failures.
    fn poll_buffer(&mut self) -> RResult<ROption<PluginStreamBuffer>, RString>;
    /// Waits for the next raw buffer.
    ///
    /// `ROk(RNone)` means the stream reached its end; `RErr` is reserved for
    /// real failures.
    fn wait_next_buffer(&mut self) -> RResult<ROption<PluginStreamBuffer>, RString>;
}

/// ABI-safe boxed raw event-stream facility.
pub type PluginRawEventStreamFacilityBox = PluginRawEventStreamFacility_TO<'static, RBox<()>>;

#[sabi_trait]
pub trait PluginDecoderErrorSink: Send + Sync {
    /// Reports a decoder error message to the host layer.
    fn on_error(&self, message: RString);
}
/// ABI-safe boxed decoder-error sink.
pub type PluginDecoderErrorSinkBox = PluginDecoderErrorSink_TO<'static, RBox<()>>;

/// ABI counterpart of the native event-stream decoder facility.
#[sabi_trait]
pub trait PluginRawEventStreamDecoderFacility: Send + Sync {
    /// Subscribes to protocol violation messages.
    fn subscribe_to_protocol_violation(
        &mut self,
        sink: PluginDecoderErrorSinkBox,
    ) -> RResult<(), RString>;
    /// Returns the size of one raw event word in bytes.
    fn get_raw_event_size_bytes(&self) -> u8;
    /// Decodes one raw buffer.
    fn decode(&mut self, raw_data: RSlice<'_, u8>) -> RResult<(), RString>;
    /// Returns the last decoded timestamp.
    fn get_last_timestamp(&self) -> EventTimestamp;
    /// Returns the timestamp shift baseline, if known.
    fn get_timestamp_shift(&self) -> ROption<EventTimestamp>;
    /// Reports whether time shifting is enabled.
    fn is_time_shifting_enabled(&self) -> bool;
    /// Replaces the last decoded timestamp.
    fn reset_last_timestamp(&mut self, timestamp: EventTimestamp);
    /// Replaces the timestamp shift baseline.
    fn reset_timestamp_shift(&mut self, shift: EventTimestamp);
    /// Reports whether timestamp indexing is supported.
    fn is_decoded_event_stream_indexable(&self) -> bool;
    fn add_marker(&mut self, timestamp: EventTimestamp) -> FfiKey;
    fn remove_marker(&mut self, key: FfiKey) -> ROption<EventTimestamp>;
}

/// ABI-safe boxed raw event-stream decoder.
pub type PluginRawEventStreamDecoderFacilityBox =
    PluginRawEventStreamDecoderFacility_TO<'static, RBox<()>>;

/// ABI counterpart of the native event-decoder facility.
///
/// Native implementations expose channels. Plugins expose the same logical
/// subscription through ABI-safe callbacks instead.
#[sabi_trait]
pub trait PluginEventSubscriptionFacility: Send + Sync {
    /// Subscribes to decoded CD events.
    fn subscribe_to_cd_events(&mut self, sink: EventBatchSinkBox) -> RResult<(), RString>;
    /// Subscribes to decoded external-trigger events.
    fn subscribe_to_ext_events(&mut self, sink: EventBatchSinkBox) -> RResult<(), RString>;
}

/// ABI-safe boxed event-subscription facility.
pub type PluginEventSubscriptionFacilityBox = PluginEventSubscriptionFacility_TO<'static, RBox<()>>;

/// Optional timestamp-index capability.
#[sabi_trait]
pub trait PluginIndexFacility: Send + Sync {
    /// Returns the earliest indexed timestamp, if available.
    fn t_min(&self) -> ROption<EventTimestamp>;
    /// Returns the latest indexed timestamp, if available.
    fn t_max(&self) -> ROption<EventTimestamp>;
}

/// ABI-safe boxed timestamp-index facility.
pub type PluginIndexFacilityBox = PluginIndexFacility_TO<'static, RBox<()>>;

/// Optional timestamp-seeking capability.
#[sabi_trait]
pub trait PluginSeekFacility: Send + Sync {
    /// Seeks to a timestamp.
    fn seek(&mut self, timestamp: EventTimestamp) -> RResult<(), RString>;
}

/// ABI-safe boxed timestamp-seek facility.
pub type PluginSeekFacilityBox = PluginSeekFacility_TO<'static, RBox<()>>;

/// Optional capability for seeking to the next external trigger.
#[sabi_trait]
pub trait PluginExternalTriggerSeekFacility: Send + Sync {
    /// Seeks to the next external trigger.
    fn seek_to_next_ext(&mut self) -> RResult<(), RString>;
}

/// ABI-safe boxed external-trigger seek facility.
pub type PluginExternalTriggerSeekFacilityBox =
    PluginExternalTriggerSeekFacility_TO<'static, RBox<()>>;

// The native HAL has a capability entry for every one of these facilities.
// Keep a distinct ABI trait and handle variant even when a particular
// facility has no operations implemented by a plugin yet. This prevents a
// plugin-only "other" bucket from being mistaken for a native capability and
// gives each capability a stable place to grow its ABI contract.
macro_rules! define_marker_plugin_facility {
    ($trait_name:ident, $interface_name:ident, $box_name:ident) => {
        #[doc = "ABI-safe marker capability implemented by a plugin."]
        #[sabi_trait]
        pub trait $trait_name: Send + Sync {}
        #[doc = "ABI-safe boxed plugin capability."]
        pub type $box_name = $interface_name<'static, RBox<()>>;
    };
}

define_marker_plugin_facility!(
    PluginAntiFlickerFacility,
    PluginAntiFlickerFacility_TO,
    PluginAntiFlickerFacilityBox
);
define_marker_plugin_facility!(
    PluginRawDecoderFacility,
    PluginRawDecoderFacility_TO,
    PluginRawDecoderFacilityBox
);
define_marker_plugin_facility!(
    PluginCameraSyncFacility,
    PluginCameraSyncFacility_TO,
    PluginCameraSyncFacilityBox
);
define_marker_plugin_facility!(
    PluginRawEventDecoderFacility,
    PluginRawEventDecoderFacility_TO,
    PluginRawEventDecoderFacilityBox
);
define_marker_plugin_facility!(
    PluginDigitalCropFacility,
    PluginDigitalCropFacility_TO,
    PluginDigitalCropFacilityBox
);
define_marker_plugin_facility!(
    PluginDigitalEventMaskFacility,
    PluginDigitalEventMaskFacility_TO,
    PluginDigitalEventMaskFacilityBox
);
define_marker_plugin_facility!(
    PluginERCModuleFacility,
    PluginERCModuleFacility_TO,
    PluginERCModuleFacilityBox
);
define_marker_plugin_facility!(
    PluginCDEventDecoderFacility,
    PluginCDEventDecoderFacility_TO,
    PluginCDEventDecoderFacilityBox
);
define_marker_plugin_facility!(
    PluginTriggerEventDecoderFacility,
    PluginTriggerEventDecoderFacility_TO,
    PluginTriggerEventDecoderFacilityBox
);
define_marker_plugin_facility!(
    PluginERCCounterEventDecoderFacility,
    PluginERCCounterEventDecoderFacility_TO,
    PluginERCCounterEventDecoderFacilityBox
);
define_marker_plugin_facility!(
    PluginRGBEventFrameDecoderFacility,
    PluginRGBEventFrameDecoderFacility_TO,
    PluginRGBEventFrameDecoderFacilityBox
);
define_marker_plugin_facility!(
    PluginMonoEventFrameDecoderFacility,
    PluginMonoEventFrameDecoderFacility_TO,
    PluginMonoEventFrameDecoderFacilityBox
);
define_marker_plugin_facility!(
    PluginEventRateActivityFilterModuleFacility,
    PluginEventRateActivityFilterModuleFacility_TO,
    PluginEventRateActivityFilterModuleFacilityBox
);
define_marker_plugin_facility!(
    PluginEventTrailFilterModuleFacility,
    PluginEventTrailFilterModuleFacility_TO,
    PluginEventTrailFilterModuleFacilityBox
);
define_marker_plugin_facility!(
    PluginHWRegisterFacility,
    PluginHWRegisterFacility_TO,
    PluginHWRegisterFacilityBox
);
define_marker_plugin_facility!(
    PluginLLBiasesFacility,
    PluginLLBiasesFacility_TO,
    PluginLLBiasesFacilityBox
);
define_marker_plugin_facility!(
    PluginROIPixelMaskFacility,
    PluginROIPixelMaskFacility_TO,
    PluginROIPixelMaskFacilityBox
);
define_marker_plugin_facility!(
    PluginTriggerInFacility,
    PluginTriggerInFacility_TO,
    PluginTriggerInFacilityBox
);
define_marker_plugin_facility!(
    PluginTriggerOutFacility,
    PluginTriggerOutFacility_TO,
    PluginTriggerOutFacilityBox
);

/// Type-erased facility handle returned by a plugin.
///
/// The enum is ABI-visible, but each concrete implementation is hidden behind
/// an `abi_stable` vtable. The host layer can call the selected interface while all
/// device logic and native implementation types stay in the plugin.
#[derive(StableAbi)]
#[repr(C)]
pub enum PluginFacilityHandle {
    AntiFlicker(PluginAntiFlickerFacilityBox),
    RawDecoder(PluginRawDecoderFacilityBox),
    CameraSync(PluginCameraSyncFacilityBox),
    RawEventDecoder(PluginRawEventDecoderFacilityBox),
    DigitalCrop(PluginDigitalCropFacilityBox),
    DigitalEventMask(PluginDigitalEventMaskFacilityBox),
    ERCModule(PluginERCModuleFacilityBox),
    CDEventDecoder(PluginCDEventDecoderFacilityBox),
    TriggerEventDecoder(PluginTriggerEventDecoderFacilityBox),
    ERCCounterEventDecoder(PluginERCCounterEventDecoderFacilityBox),
    RGBEventFrameDecoder(PluginRGBEventFrameDecoderFacilityBox),
    MonoEventFrameDecoder(PluginMonoEventFrameDecoderFacilityBox),
    EventRateActivityFilterModule(PluginEventRateActivityFilterModuleFacilityBox),
    EventTrailFilterModule(PluginEventTrailFilterModuleFacilityBox),
    HWRegister(PluginHWRegisterFacilityBox),
    LLBiases(PluginLLBiasesFacilityBox),
    HALSoftwareInfo(PluginHALSoftwareInfoFacilityBox),
    PluginSoftwareInfo(PluginPluginSoftwareInfoFacilityBox),
    Monitoring(PluginMonitoringFacilityBox),
    Roi(PluginROIFacilityBox),
    ROIPixelMask(PluginROIPixelMaskFacilityBox),
    TriggerIn(PluginTriggerInFacilityBox),
    TriggerOut(PluginTriggerOutFacilityBox),
    Geometry(PluginGeometryFacilityBox),
    HardwareIdentification(PluginHardwareIdentificationFacilityBox),
    RawEventStream(PluginRawEventStreamFacilityBox),
    RawEventStreamDecoder(PluginRawEventStreamDecoderFacilityBox),
    EventSubscription(PluginEventSubscriptionFacilityBox),
    Index(PluginIndexFacilityBox),
    Seek(PluginSeekFacilityBox),
    ExternalTriggerSeek(PluginExternalTriggerSeekFacilityBox),
}

impl PluginFacilityHandle {
    /// Returns the stable capability key represented by this handle.
    pub const fn facility_type(&self) -> PluginFacilityType {
        match self {
            Self::AntiFlicker(_) => PluginFacilityType::AntiFlicker,
            Self::RawDecoder(_) => PluginFacilityType::RawDecoder,
            Self::CameraSync(_) => PluginFacilityType::CameraSync,
            Self::RawEventDecoder(_) => PluginFacilityType::RawEventDecoder,
            Self::DigitalCrop(_) => PluginFacilityType::DigitalCrop,
            Self::DigitalEventMask(_) => PluginFacilityType::DigitalEventMask,
            Self::ERCModule(_) => PluginFacilityType::ERCModule,
            Self::CDEventDecoder(_) => PluginFacilityType::CDEventDecoder,
            Self::TriggerEventDecoder(_) => PluginFacilityType::TriggerEventDecoder,
            Self::ERCCounterEventDecoder(_) => PluginFacilityType::ERCCounterEventDecoder,
            Self::RGBEventFrameDecoder(_) => PluginFacilityType::RGBEventFrameDecoder,
            Self::MonoEventFrameDecoder(_) => PluginFacilityType::MonoEventFrameDecoder,
            Self::EventRateActivityFilterModule(_) => {
                PluginFacilityType::EventRateActivityFilterModule
            }
            Self::EventTrailFilterModule(_) => PluginFacilityType::EventTrailFilterModule,
            Self::HWRegister(_) => PluginFacilityType::HWRegister,
            Self::LLBiases(_) => PluginFacilityType::LLBiases,
            Self::HALSoftwareInfo(_) => PluginFacilityType::HALSoftwareInfo,
            Self::PluginSoftwareInfo(_) => PluginFacilityType::PluginSoftwareInfo,
            Self::Monitoring(_) => PluginFacilityType::Monitoring,
            Self::Roi(_) => PluginFacilityType::Roi,
            Self::ROIPixelMask(_) => PluginFacilityType::ROIPixelMask,
            Self::TriggerIn(_) => PluginFacilityType::TriggerIn,
            Self::TriggerOut(_) => PluginFacilityType::TriggerOut,
            Self::Geometry(_) => PluginFacilityType::Geometry,
            Self::HardwareIdentification(_) => PluginFacilityType::HardwareIdentification,
            Self::RawEventStream(_) => PluginFacilityType::RawEventStream,
            Self::RawEventStreamDecoder(_) => PluginFacilityType::RawEventStreamDecoder,
            Self::EventSubscription(_) => PluginFacilityType::EventSubscription,
            Self::Index(_) => PluginFacilityType::Index,
            Self::Seek(_) => PluginFacilityType::Seek,
            Self::ExternalTriggerSeek(_) => PluginFacilityType::ExternalTriggerSeek,
        }
    }
}

#[sabi_trait]
pub trait DevicePlugin: Send + Sync {
    /// A device is a registry of facilities. These metadata/lifecycle methods
    /// remain only as deprecated ABI shims for older host layers; new code should
    /// obtain the corresponding facility and call it there.
    #[deprecated(note = "use the hardware-identification facility")]
    /// Returns the legacy serial number.
    fn serial(&self) -> RString;
    #[deprecated(note = "use the hardware-identification facility")]
    /// Returns the legacy connection type.
    fn connection_type(&self) -> ConnectionType;
    #[deprecated(note = "use the geometry facility")]
    /// Returns the legacy geometry description.
    fn geometry(&self) -> PluginGeometry;
    #[deprecated(note = "use PluginIndexFacility")]
    /// Returns the earliest indexed timestamp.
    fn t_min(&self) -> ROption<EventTimestamp> {
        ROption::RNone
    }
    #[deprecated(note = "use PluginIndexFacility")]
    /// Returns the latest indexed timestamp.
    fn t_max(&self) -> ROption<EventTimestamp> {
        ROption::RNone
    }
    #[deprecated(note = "use PluginSeekFacility")]
    /// Performs the legacy timestamp seek operation.
    fn seek(&mut self, _timestamp: EventTimestamp) -> RResult<(), RString> {
        RResult::RErr("seek is not supported by this plugin".into())
    }
    #[deprecated(note = "use PluginExternalTriggerSeekFacility")]
    /// Performs the legacy external-trigger seek operation.
    fn seek_to_next_ext(&mut self) -> RResult<(), RString> {
        RResult::RErr("external-trigger seeking is not supported by this plugin".into())
    }
    /// Lists all capabilities advertised by the plugin.
    fn get_facilities(&self) -> RVec<PluginFacilityType>;
    #[deprecated(note = "use get_facility_handle")]
    /// Returns a legacy capability descriptor.
    fn get_facility(&self, facility_type: PluginFacilityType) -> ROption<PluginFacility>;
    /// Returns an ABI-safe handle for a capability.
    fn get_facility_handle(
        &self,
        _facility_type: PluginFacilityType,
    ) -> ROption<PluginFacilityHandle> {
        ROption::RNone
    }
    #[deprecated(note = "use the event-stream and event-decoder facilities")]
    /// Starts legacy event delivery.
    fn start_events(&mut self, sink: EventBatchSinkBox) -> RResult<(), RString>;
    #[deprecated(note = "use the event-decoder facility")]
    /// Starts legacy external-trigger delivery.
    fn start_external_triggers(&mut self, sink: EventBatchSinkBox) -> RResult<(), RString>;
    #[deprecated(note = "use the event-stream facility")]
    /// Loads one legacy event batch.
    ///
    /// `ROk(RNone)` means the stream reached its end; `RErr` is reserved for
    /// real failures.
    fn load_batch(&mut self) -> RResult<ROption<()>, RString>;
}

pub type DevicePluginBox = DevicePlugin_TO<'static, RBox<()>>;

#[sabi_trait]
pub trait DeviceDiscoveryPlugin: Send + Sync {
    /// Discovers devices available through this plugin.
    fn discover(&self) -> RVec<PluginCameraDescriptionAbi>;
    /// Returns the TOML schema describing the values accepted at creation.
    ///
    /// An empty string means that the plugin has no configuration schema and
    /// only supports the legacy serial-based opening method.
    fn configuration_schema(&self) -> RString {
        "".into()
    }
    /// Opens a device by serial number.
    #[deprecated(note = "use open_device_with_configuration")]
    fn open_device(&self, serial: RStr<'_>) -> RResult<DevicePluginBox, RString>;
    /// Opens a device using the host-layer-created configuration object.
    ///
    /// The default delegates to the legacy method so existing plugins can be
    /// migrated incrementally. New plugins should validate every required
    /// value before acquiring resources.
    fn open_device_with_configuration(
        &self,
        configuration: PluginConfiguration,
    ) -> RResult<DevicePluginBox, RString> {
        self.open_device(configuration.serial.as_str().into())
    }
}

/// ABI-safe boxed device-discovery plugin.
pub type DeviceDiscoveryPluginBox = DeviceDiscoveryPlugin_TO<'static, RBox<()>>;

/// One named discovery interface exported by a plugin library.
///
/// A single shared library can only export one `abi_stable` root module, so a
/// library that ships several devices (see the `openevt-plugins` crate) returns
/// one of these per device rather than one discovery for the whole module. The
/// name travels with the discovery so each device keeps its own identity in
/// [`super::discovery::CameraDescription::plugin_name`].
#[repr(C)]
#[derive(StableAbi)]
pub struct NamedDiscoveryPlugin {
    /// Display name of the device this discovery exposes.
    pub name: RString,
    /// The discovery interface itself.
    pub discovery: DeviceDiscoveryPluginBox,
}

#[derive(StableAbi)]
#[repr(C)]
#[sabi(kind(Prefix(prefix_ref = DevicePluginModuleRef)))]
pub struct DevicePluginModuleVtable {
    /// Returns the plugin's display name.
    pub name: extern "C" fn() -> RString,
    /// Creates the plugin's discovery interface.
    ///
    /// Single-device libraries only need this. Libraries exporting several
    /// devices should populate [`Self::create_discoveries`] as well and leave
    /// this pointing at whichever device is the most sensible default, so that
    /// hosts built against the older vtable keep working.
    pub create_discovery: extern "C" fn() -> DeviceDiscoveryPluginBox,
    /// Creates every discovery interface exported by this library.
    ///
    /// Appended after `create_discovery`, so its accessor yields `None` for
    /// plugins compiled against the earlier vtable; the host then falls back to
    /// `create_discovery`.
    pub create_discoveries: extern "C" fn() -> RVec<NamedDiscoveryPlugin>,
}

impl RootModule for DevicePluginModuleRef {
    abi_stable::declare_root_module_statics! {DevicePluginModuleRef}
    const BASE_NAME: &'static str = "openevt_device_plugin";
    const NAME: &'static str = "openevt_device_plugin";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use abi_stable::type_level::downcasting::TD_Opaque;

    #[test]
    fn abi_covers_every_native_facility() {
        assert_eq!(PluginFacilityType::ALL.len(), 28);
        assert!(PluginFacilityType::ALL.iter().all(|kind| kind.is_native()));

        let native = [
            FacilityType::AntiFlickerFacility,
            FacilityType::RawDecoderFacility,
            FacilityType::CameraSyncFacility,
            FacilityType::RawEventDecoderFacility,
            FacilityType::DigitalCropFacility,
            FacilityType::DigitalEventMaskFacility,
            FacilityType::ERCModuleFacility,
            FacilityType::CDEventDecoderFacility,
            FacilityType::TriggerEventDecoderFaciliy,
            FacilityType::ERCCounterEventDecoderFacility,
            FacilityType::RGBEventFrameDecoderFacility,
            FacilityType::MonoEventFrameDecoderFacility,
            FacilityType::EventRateActivityFilterModuleFacility,
            FacilityType::EventTrailFilterModuleFacility,
            FacilityType::RawEventStreamFacility,
            FacilityType::RawEventStreamDecoderFacility,
            FacilityType::EventSubscriptionFacility,
            FacilityType::GeometryFacility,
            FacilityType::HALSoftwareInfoFacility,
            FacilityType::HWIdentificationFacility,
            FacilityType::HWRegisterFacility,
            FacilityType::LLBiasesFacility,
            FacilityType::MonitoringFacility,
            FacilityType::PluginSoftwareInfoFacility,
            FacilityType::ROIFacility,
            FacilityType::ROIPixelMaskFacility,
            FacilityType::TriggerInFacility,
            FacilityType::TriggerOutFacility,
        ];

        assert!(
            native
                .into_iter()
                .map(PluginFacilityType::from)
                .all(|kind| PluginFacilityType::ALL.contains(&kind))
        );
    }

    #[test]
    fn opaque_facility_descriptor_preserves_kind() {
        let descriptor = PluginFacility::new(PluginFacilityType::Roi);
        assert_eq!(descriptor.facility_type, PluginFacilityType::Roi);
    }

    struct TestGeometry;

    impl PluginGeometryFacility for TestGeometry {
        fn get_width(&self) -> u32 {
            640
        }

        fn get_height(&self) -> u32 {
            480
        }
    }

    #[test]
    fn type_erased_facility_handle_calls_plugin_implementation() {
        let handle = PluginFacilityHandle::Geometry(PluginGeometryFacility_TO::from_value(
            TestGeometry,
            TD_Opaque,
        ));

        let PluginFacilityHandle::Geometry(geometry) = handle else {
            panic!("wrong facility handle variant");
        };

        assert_eq!(geometry.get_width(), 640);
        assert_eq!(geometry.get_height(), 480);
    }
}
