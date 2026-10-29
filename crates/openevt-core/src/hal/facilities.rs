//! Facility abstractions for the HAL.
//!
//! A "facility" is a capability exposed by a device: geometry, hardware
//! identification, streams, decoders, ROI control, trigger control, and so on.
//! A device is represented as a registry rather than a single trait object. It is a
//! registry of individually typed capability objects.
//!
//! The pieces fit together like this:
//!
//! - `FacilityType` names the capability the caller wants.
//! - `FacilityHandle` stores the concrete trait object behind that capability.
//! - `Device::get_facility` returns the handle.
//! - `TryFrom<FacilityHandle>` converts the handle into the exact trait-object
//!   alias expected by the caller.
//! - `BaseFacility` provides `Any`-based downcasting when a mutable handle must
//!   be recovered as a concrete type.
//!
//! This design models heterogeneous hardware while keeping call sites explicit
//! about ownership, mutability, and thread-safety.

use crate::hal::errors::{
    DecoderError, DecoderProtocolViolation, HardwareError, ProcessingError, SharedError,
    StreamError,
};
use crate::types::{CallbackId, Cb, CbRo, EventTimestamp, PixelMask, Region, StreamLength};
use crate::types::{EventCD, EventExtTrigger};
pub use crate::pack_facility;
pub use crate::property;
use slotmap::DefaultKey;
use std::sync::{Arc, RwLock};
use thiserror::Error;

/// Type alias for the immutable geometry facility handle.
/// Shared handle for an immutable geometry facility.
pub type GeometryFacilityHandle = Arc<dyn GeometryFacility + Send + Sync>;
/// Shared handle for immutable HAL software metadata.
pub type HALSoftwareInfoFacilityHandle = Arc<dyn HALSoftwareInfoFacility + Send + Sync>;
/// Shared handle for immutable hardware identification.
pub type HWIdentificationFacilityHandle = Arc<dyn HWIdentificationFacility + Send + Sync>;
/// Shared handle for immutable monitoring data.
pub type MonitoringFacilityHandle = Arc<dyn MonitoringFacility + Send + Sync>;
/// Shared handle for immutable plugin metadata.
pub type PluginSoftwareInfoFacilityHandle = Arc<dyn PluginSoftwareInfoFacility + Send + Sync>;

/// Type alias for a mutable facility handle wrapped in `Arc<RwLock<_>>`.
///
/// Mutable facilities are shared between consumers, but the trait object itself
/// must be locked before use because its methods can mutate internal state.
/// Locked handle for anti-flicker controls.
pub type AntiFlickerFacilityHandle = Arc<RwLock<dyn AntiFlickerFacility + Send>>;
/// Locked handle for raw decoder protocol support.
pub type RawDecoderFacilityHandle = Arc<RwLock<dyn RawDecoderFacility + Send>>;
/// Locked handle for camera synchronization controls.
pub type CameraSyncFacilityHandle = Arc<RwLock<dyn CameraSyncFacility + Send>>;
/// Locked handle for digital crop controls.
pub type DigitalCropFacilityHandle = Arc<RwLock<dyn DigitalCropFacility + Send>>;
/// Locked handle for per-pixel event masks.
pub type DigitalEventMaskFacilityHandle = Arc<RwLock<dyn DigitalEventMaskFacility + Send>>;
/// Locked handle for event-rate control.
pub type ERCModuleFacilityHandle = Arc<RwLock<dyn ERCModuleFacility + Send>>;
/// Locked handle for event-batch subscriptions.
pub type EventSubscriptionFacilityHandle = Arc<RwLock<dyn EventSubscriptionFacility + Send>>;

/// Locked handle for raw event-stream decoding.
pub type RawEventStreamDecoderFacilityHandle =
    Arc<RwLock<dyn RawEventStreamDecoderFacility + Send>>;

/// Locked handle for RGB event-frame decoding.
pub type EventFrameDecoderRGBFacilityHandle =
    Arc<RwLock<dyn EventFrameDecoderFacility<FrameType = RGBFrameType> + Send>>;
/// Locked handle for monochrome event-frame decoding.
pub type EventFrameDecoderMonoFacilityHandle =
    Arc<RwLock<dyn EventFrameDecoderFacility<FrameType = MonoFrameType> + Send>>;
/// Locked handle for event-rate activity filtering.
pub type EventRateActivityFilterModuleFacilityHandle =
    Arc<RwLock<dyn EventRateActivityFilterModuleFacility + Send>>;
/// Locked handle for event-trail filtering.
pub type EventTrailFilterModuleFacilityHandle =
    Arc<RwLock<dyn EventTrailFilterModuleFacility + Send>>;
/// Locked handle for raw event-stream input.
pub type RawEventStreamFacilityHandle = Arc<RwLock<dyn RawEventStreamFacility + Send>>;
/// Locked handle for hardware register access.
pub type HWRegisterFacilityHandle = Arc<RwLock<dyn HWRegisterFacility + Send>>;
/// Locked handle for low-level bias controls.
pub type LLBiasesFacilityHandle = Arc<RwLock<dyn LLBiasesFacility + Send>>;
/// Locked handle for region-of-interest controls.
pub type ROIFacilityHandle = Arc<RwLock<dyn ROIFacility + Send>>;
/// Locked handle for ROI pixel-mask controls.
pub type ROIPixelMaskFacilityHandle = Arc<RwLock<dyn ROIPixelMaskFacility + Send>>;
/// Locked handle for trigger-input controls.
pub type TriggerInFacilityHandle = Arc<RwLock<dyn TriggerInFacility + Send>>;
/// Locked handle for trigger-output controls.
pub type TriggerOutFacilityHandle = Arc<RwLock<dyn TriggerOutFacility + Send>>;

/// Typed raw decoder handle for CD events.
pub type CDEventDecoderFacilityHandle = Arc<RwLock<dyn RawEventDecoderFacility<EventCD> + Send>>;
/// Typed raw decoder handle for external-trigger events.
pub type ExtTriggerEventDecoderFacilityHandle =
    Arc<RwLock<dyn RawEventDecoderFacility<EventExtTrigger> + Send>>;
/// Typed raw decoder handle for ERC counter events.
pub type ERCCounterEventDecoderFacilityHandle =
    Arc<RwLock<dyn RawEventDecoderFacility<EventERCCounter> + Send>>;

use std::any::Any;
use std::convert::TryFrom;

/// Error returned when a caller requests a facility handle of the wrong type.
#[derive(Error, Debug)]
#[error("Facility type mismatch: The requested facility type does not match the retrieved handle.")]
pub struct FacilityTypeMismatch;

macro_rules! impl_try_from_facility {
    (
        // Matches: Variant, Type Alias
        $( $variant:ident, $alias:ty ),* $(,)?
    ) => {
        $(
            impl TryFrom<FacilityHandle> for $alias {
                type Error = FacilityTypeMismatch;

                fn try_from(handle: FacilityHandle) -> Result<Self, Self::Error> {
                    if let FacilityHandle::$variant(h) = handle {
                        Ok(h)
                    } else {
                        Err(FacilityTypeMismatch)
                    }
                }
            }
        )*
    };
}

impl_try_from_facility! {
    // --- Immutable Facilities ---
    GeometryFacility, GeometryFacilityHandle,
    HALSoftwareInfoFacility, HALSoftwareInfoFacilityHandle,
    HWIdentificationFacility, HWIdentificationFacilityHandle,
    MonitoringFacility, MonitoringFacilityHandle,
    PluginSoftwareInfoFacility, PluginSoftwareInfoFacilityHandle,

    // --- Mutable Facilities ---
    AntiFlickerFacility, AntiFlickerFacilityHandle,
    RawDecoderFacility, RawDecoderFacilityHandle,
    CameraSyncFacility, CameraSyncFacilityHandle,
    DigitalCropFacility, DigitalCropFacilityHandle,
    DigitalEventMaskFacility, DigitalEventMaskFacilityHandle,
    ERCModuleFacility, ERCModuleFacilityHandle,
    EventSubscriptionFacility, EventSubscriptionFacilityHandle,
    RGBEventFrameDecoderFacility, EventFrameDecoderRGBFacilityHandle,
    MonoEventFrameDecoderFacility, EventFrameDecoderMonoFacilityHandle,
    EventRateActivityFilterModuleFacility, EventRateActivityFilterModuleFacilityHandle,
    EventTrailFilterModuleFacility, EventTrailFilterModuleFacilityHandle,
    RawEventStreamFacility, RawEventStreamFacilityHandle,
    HWRegisterFacility, HWRegisterFacilityHandle,
    LLBiasesFacility, LLBiasesFacilityHandle,
    ROIFacility, ROIFacilityHandle,
    ROIPixelMaskFacility, ROIPixelMaskFacilityHandle,
    TriggerInFacility, TriggerInFacilityHandle,
    TriggerOutFacility, TriggerOutFacilityHandle,
    RawEventStreamDecoderFacility, RawEventStreamDecoderFacilityHandle,

    // --- Monomorphized Generic Mutable Facilities ---
    CDEventDecoderFacility, CDEventDecoderFacilityHandle,
    ExtTriggerEventDecoderFacility, ExtTriggerEventDecoderFacilityHandle,
    ERCCounterEventDecoderFacility, ERCCounterEventDecoderFacilityHandle,
}

/// Placeholder event type for ERC counter decoding.
pub struct EventERCCounter {}
/// Marker type for RGB event-frame facilities.
pub struct RGBFrameType {}
/// Marker type for monochrome event-frame facilities.
pub struct MonoFrameType {}

/// Unified error type for failures that occur while working through a facility.
#[derive(Error, Debug)]
pub enum FacilityError {
    #[error("Plugin facility error: {0}")]
    Plugin(String),
    #[error(transparent)]
    Decoder(#[from] DecoderError),
    #[error(transparent)]
    Hardware(#[from] HardwareError),
    #[error(transparent)]
    Stream(#[from] StreamError),
    #[error(transparent)]
    Processing(#[from] ProcessingError),
    #[error(transparent)]
    DecoderProtocol(#[from] DecoderProtocolViolation),
    #[error("Invalid downcast from {0} to {1}")]
    FacilityDowncastError(String, String),
}

/// Result type used by facility methods.
pub type FacilityResult<T> = Result<T, FacilityError>;

/// Owned bytes returned by a stream facility. The storage strategy is private
/// to the implementation; callers only depend on the valid byte range.
/// A raw stream buffer and the number of valid bytes it contains.
pub type StreamBuffer = (Vec<u8>, StreamLength);

/// Native event delivery is callback-based. Channel, queue, or task choices
/// remain implementation details of a concrete facility.
/// Callback receiving a decoded CD-event batch.
pub type EventCDCallback = Box<dyn FnMut(&[EventCD]) + Send + 'static>;
/// Callback receiving a decoded external-trigger batch.
pub type EventExtTriggerCallback = Box<dyn FnMut(&[EventExtTrigger]) + Send + 'static>;
/// Callback receiving a decoder protocol error.
pub type DecoderErrorCallback = Box<dyn FnMut(SharedError) + Send + 'static>;

/// Identifies a capability a device may expose.
///
/// This enum is the lookup key used with `Device::get_facility`. It is broader
/// than any single device implementation so code can ask for a capability
/// without knowing the concrete type up front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FacilityType {
    /// Anti-flicker control facility.
    AntiFlickerFacility,
    /// Raw decoder protocol facility.
    RawDecoderFacility,
    /// Camera synchronization facility.
    CameraSyncFacility,
    /// Generic raw-event decoder facility.
    RawEventDecoderFacility,
    DigitalCropFacility,
    DigitalEventMaskFacility,
    ERCModuleFacility,
    CDEventDecoderFacility,
    TriggerEventDecoderFaciliy,
    ERCCounterEventDecoderFacility,
    RGBEventFrameDecoderFacility,
    MonoEventFrameDecoderFacility,
    EventRateActivityFilterModuleFacility,
    EventTrailFilterModuleFacility,
    /// Raw event-stream input facility.
    RawEventStreamFacility,
    /// Raw event-stream decoder facility.
    RawEventStreamDecoderFacility,
    /// Decoded-event subscription facility.
    EventSubscriptionFacility,
    GeometryFacility,
    HALSoftwareInfoFacility,
    HWIdentificationFacility,
    HWRegisterFacility,
    LLBiasesFacility,
    MonitoringFacility,
    PluginSoftwareInfoFacility,
    ROIFacility,
    ROIPixelMaskFacility,
    TriggerInFacility,
    TriggerOutFacility,
}

/// Stores the concrete trait object for a facility.
///
/// Immutable facilities are stored as plain `Arc<T>`. Mutable facilities are
/// stored as `Arc<RwLock<T>>` so callers can share them while still taking
/// exclusive write access when they need to mutate device state.
#[derive(Clone)]
pub enum FacilityHandle {
    // --- Immutable Facilities (Read-Only across threads) ---
    /// Geometry facility handle.
    GeometryFacility(GeometryFacilityHandle),
    HALSoftwareInfoFacility(HALSoftwareInfoFacilityHandle),
    HWIdentificationFacility(HWIdentificationFacilityHandle),
    MonitoringFacility(MonitoringFacilityHandle),
    PluginSoftwareInfoFacility(PluginSoftwareInfoFacilityHandle),

    // --- Mutable Facilities (Exclusive access required) ---
    /// Anti-flicker facility handle.
    AntiFlickerFacility(AntiFlickerFacilityHandle),
    /// Raw decoder facility handle.
    RawDecoderFacility(RawDecoderFacilityHandle),
    CameraSyncFacility(CameraSyncFacilityHandle),
    DigitalCropFacility(DigitalCropFacilityHandle),
    DigitalEventMaskFacility(DigitalEventMaskFacilityHandle),
    ERCModuleFacility(ERCModuleFacilityHandle),
    /// Event-subscription facility handle.
    EventSubscriptionFacility(EventSubscriptionFacilityHandle),
    RGBEventFrameDecoderFacility(EventFrameDecoderRGBFacilityHandle),
    MonoEventFrameDecoderFacility(EventFrameDecoderMonoFacilityHandle),
    EventRateActivityFilterModuleFacility(EventRateActivityFilterModuleFacilityHandle),
    EventTrailFilterModuleFacility(EventTrailFilterModuleFacilityHandle),
    /// Raw event-stream facility handle.
    RawEventStreamFacility(RawEventStreamFacilityHandle),
    /// Raw event-stream decoder facility handle.
    RawEventStreamDecoderFacility(RawEventStreamDecoderFacilityHandle),
    HWRegisterFacility(HWRegisterFacilityHandle),
    LLBiasesFacility(LLBiasesFacilityHandle),
    ROIFacility(ROIFacilityHandle),
    ROIPixelMaskFacility(ROIPixelMaskFacilityHandle),
    TriggerInFacility(TriggerInFacilityHandle),
    TriggerOutFacility(TriggerOutFacilityHandle),

    // --- Monomorphized Generic Mutable Facilities ---
    CDEventDecoderFacility(CDEventDecoderFacilityHandle),
    ExtTriggerEventDecoderFacility(ExtTriggerEventDecoderFacilityHandle),
    ERCCounterEventDecoderFacility(ERCCounterEventDecoderFacilityHandle),
}
// --- Supporting Types ---

/// Anti-flicker filter mode.
#[derive(Debug)]
pub enum AntiFlickerMode {
    /// Pass the configured frequency band.
    BandPass,
    /// Reject the configured frequency band.
    BandStop,
}

/// Camera synchronization role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraSyncMode {
    /// Operate independently.
    Standalone,
    /// Provide synchronization to other devices.
    Master,
    /// Follow an external synchronization source.
    Slave,
}

/// Transport used to connect a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionType {
    /// USB transport.
    Usb,
    /// MIPI transport.
    Mipi,
    /// Vendor-specific transport.
    Proprietary,
    /// Unknown transport.
    Unknown,
}

/// Event-trail filtering algorithms supported by a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailFilterTypes {
    /// Trail filter.
    TRAIL,
    /// STC mode that cuts trails.
    STCCutTrail,
    /// STC mode that keeps trails.
    STCKeepTrail,
}

/// Sensor identity metadata.
#[derive(Debug, Clone)]
pub struct SensorInfo {
    /// Sensor name.
    pub name: String,
    /// Integrator or manufacturer name.
    pub integrator: String,
    /// Sensor generation or version.
    pub version: String,
}

/// Device system and firmware metadata.
#[derive(Debug, Clone)]
pub struct SystemInfo {
    /// Device serial number.
    pub serial_number: String,
    /// Firmware version.
    pub firmware_version: String,
}

/// Base trait shared by all facilities.
///
/// The `Any` hooks support dynamic downcasting when a device stores multiple
/// different facility trait objects in one registry.
pub trait BaseFacility: Any + Send + Sync {
    /// Returns this facility as an `Any` reference for immutable downcasting.
    fn as_any(&self) -> &dyn Any;
    /// Returns this facility as an `Any` reference for mutable downcasting.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any + Sized + Send + Sync> BaseFacility for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// --- Facilities ---
/// Controls anti-flicker behavior for the device.
///
/// The exact meaning of the thresholds and frequency fields is device-specific
pub trait AntiFlickerFacility: BaseFacility {
    property! {
        frequency: u32;
        frequency_band: (u32, u32);
        low_frequency: u32;
        min_supported_frequency: u32;
        max_supported_frequency: u32;
        filtering_mode: u32;
        duty_cycle: f32;
        min_supported_duty_cycle: f32;
        max_supported_duty_cycle: f32;
        start_threshold: u32;
        stop_threshold: u32;
        start_stop_threshold: (u32, u32);
        min_supported_start_threshold: u32;
        max_supported_start_threshold: u32;
        min_supported_stop_threshold: u32;
        max_supported_stop_threshold: u32;
        enabled: bool;
    }
}

/// Base interface for decoders that emit protocol violations and expose the raw
/// event word size.
pub trait RawDecoderFacility: BaseFacility {
    /// Registers a callback for protocol violations.
    fn subscribe_to_protocol_violation(
        &mut self,
        callback: DecoderErrorCallback,
    ) -> FacilityResult<()>;

    property! {
        ro raw_event_size_bytes: u8;
    }
}

/// Decodes raw byte streams into typed events and decode callbacks.
///
/// Implementations are expected to consume the input buffer in order and invoke
/// registered callbacks as events are decoded.
pub trait RawEventDecoderFacility<T>: RawDecoderFacility {
    /// Decodes raw bytes and emits typed events through registered callbacks.
    fn decode(&mut self, raw_data: &[u8]) -> FacilityResult<()>;
    /// Adds a callback and returns its registration identifier.
    fn add_decode_callback(&mut self, cb: Cb<&[T]>) -> FacilityResult<CallbackId>;

    /// Removes a previously registered callback.
    fn remove_decode_callback(&mut self, cb_id: CallbackId) -> FacilityResult<()>;
}

/// Decodes raw data directly into a caller-provided buffer.
///
/// Decodes raw data directly into a caller-provided event buffer.
///
/// This is useful when callers want ownership of the decoded batch instead of
/// callback dispatch. Implementations may use a different buffering strategy
/// from [`RawEventDecoderFacility`].
pub trait BufferedEventDecoderFacility<T>: RawDecoderFacility {
    /// Decodes `raw_data` and appends events to `output`.
    fn decode_to_buffer(&mut self, raw_data: &[u8], output: &mut Vec<T>) -> FacilityResult<()>;
}

/// Controls the camera synchronization mode.
pub trait CameraSyncFacility: BaseFacility {
    property! {
        mode: CameraSyncMode;
    }
}

/// Enables or configures a sensor crop window.
pub trait DigitalCropFacility: BaseFacility {
    property! {
        enabled: bool;
        window_region: Region;
    }
}

/// Controls per-pixel event masking.
pub trait DigitalEventMaskFacility: BaseFacility {
    property! {
        masks: Vec<PixelMask>;
    }
}

/// Configures event-rate control logic.
pub trait ERCModuleFacility: BaseFacility {
    property! {
        enabled: bool;
        cd_event_rate: u32;
        max_supported_cd_event_rate: u32;
        min_supported_cd_event_rate: u32;
        count_period: u32;
        cd_event_count: u32;
        min_supported_cd_event_count: u32;
        max_supported_cd_event_count: u32;
    }

    fn erc_from_file(&mut self, path: &str) -> FacilityResult<()>;
}

/// Receives decoded CD and external-trigger event batches from a decoder.
pub trait EventSubscriptionFacility: BaseFacility {
    /// Registers a callback for decoded CD-event batches.
    fn subscribe_to_cd_events(&mut self, callback: EventCDCallback) -> FacilityResult<()>;
    /// Registers a callback for decoded external-trigger batches.
    fn subscribe_to_ext_events(&mut self, callback: EventExtTriggerCallback) -> FacilityResult<()>;
}

/// Decodes events into a framed output type.
///
/// Decodes event data into a frame type such as RGB or monochrome pixels.
pub trait EventFrameDecoderFacility: BaseFacility {
    /// Frame representation produced by this facility.
    type FrameType;
    property! {
        width: u32;
        height: u32;
    }

    /// Registers a read-only callback for generated frames.
    fn add_event_frame_cb(
        &mut self,
        callback: CbRo<&Self::FrameType>,
    ) -> FacilityResult<CallbackId>;
}

/// Controls the event trail filter module.
pub trait EventTrailFilterModuleFacility: BaseFacility {
    property! {
        enabled: bool;
        filter_type: TrailFilterTypes;
        threshold: u32;
    }
    fn get_available_types(&self) -> Vec<TrailFilterTypes>;
    fn get_max_supported_threshold(&self) -> u32;
    fn get_min_supported_threshold(&self) -> u32;
}

/// Controls the event-rate activity filter module.
pub trait EventRateActivityFilterModuleFacility: BaseFacility {
    property! {
        enabled: bool;
        thresholds: (u32, u32);
    }
}

/// Abstracts the raw byte stream coming from a file or device.
pub trait RawEventStreamFacility: BaseFacility {
    /// Starts delivery from the underlying raw source.
    fn start(&mut self) -> FacilityResult<()>;
    /// Stops delivery from the underlying raw source.
    fn stop(&mut self) -> FacilityResult<()>;
    /// Polls for the next buffer without requiring a blocking wait.
    ///
    /// `Ok(None)` means the stream reached its end. `Err` is reserved for
    /// real failures, never for end of stream.
    fn poll_buffer(&mut self) -> FacilityResult<Option<StreamBuffer>>;
    /// Waits for and returns the next raw buffer.
    ///
    /// `Ok(None)` means the stream reached its end. `Err` is reserved for
    /// real failures, never for end of stream.
    fn wait_next_buffer(&mut self) -> FacilityResult<Option<StreamBuffer>>;
}

/// Decodes raw stream buffers into typed events.
///
/// This is the main decoder contract used by file-backed readers.
/// It maintains internal timestamp state and dispatches decoded batches
/// through the event facilities.
pub trait RawEventStreamDecoderFacility: RawDecoderFacility + BaseFacility {
    /// Decodes raw data. Identifies the events in the buffer and dispatches them
    /// to the corresponding event decoders.
    ///
    /// Warning: It is mandatory to pass strictly consecutive buffers from the same source.
    fn decode(&mut self, raw_data: &[u8]) -> FacilityResult<()>;

    /// Gets the timestamp of the last event.
    fn get_last_timestamp(&self) -> EventTimestamp;

    /// Retrieves the timestamp shift (timestamp of the first event in the stream).
    /// Returns `Some(shift)` if known, otherwise `None`.
    fn get_timestamp_shift(&self) -> Option<EventTimestamp>;

    /// Returns true if time shifting is enabled.
    fn is_time_shifting_enabled(&self) -> bool;

    /// Resets the decoder last timestamp.
    ///
    /// If time shifting is enabled, `timestamp` must be in the shifted time reference.
    fn reset_last_timestamp(&mut self, timestamp: EventTimestamp);

    /// Resets the decoder timestamp shift.
    ///
    /// If time shifting is disabled, this function should do nothing.
    fn reset_timestamp_shift(&mut self, shift: EventTimestamp);

    /// Returns true if the decoded events stream can be indexed.
    fn is_decoded_event_stream_indexable(&self) -> bool;

    /// Add a marker to the event stream
    fn add_marker(&mut self, timestamp: EventTimestamp) -> DefaultKey;

    /// Remove a marker from the event stream
    /// Returns the marker ID if removal was successful
    fn remove_marker(&mut self, key: DefaultKey) -> Option<EventTimestamp>;
}

/// Exposes the sensor geometry.
pub trait GeometryFacility: BaseFacility {
    /// Returns the sensor width in pixels.
    fn get_width(&self) -> i32;
    /// Returns the sensor height in pixels.
    fn get_height(&self) -> i32;
}

/// Exposes software version information for the host-side HAL implementation.
///
/// Camera firmware information belongs to [`HWIdentificationFacility::get_system_info`].
pub trait HALSoftwareInfoFacility: BaseFacility {
    /// Returns the software version exposed by the HAL.
    fn get_version(&self) -> String;
}

/// Exposes hardware identity and file/sensor metadata.
pub trait HWIdentificationFacility: BaseFacility {
    /// Returns the device serial number.
    fn get_serial(&self) -> FacilityResult<String>;
    /// Returns the device-specific system identifier.
    fn get_system_id(&self) -> FacilityResult<i64>;
    /// Returns sensor identity and generation metadata.
    fn get_sensor_info(&self) -> FacilityResult<SensorInfo>;
    /// Returns system and firmware metadata.
    fn get_system_info(&self) -> FacilityResult<SystemInfo>;
    /// Returns the physical or software connection type.
    fn get_connection_type(&self) -> FacilityResult<ConnectionType>;
    /// Lists data encodings supported by the device.
    fn get_available_data_encoding_formats(&self) -> FacilityResult<Vec<String>>;
    /// Returns the currently selected data encoding.
    fn get_current_data_encoding_format(&self) -> FacilityResult<String>;
}

/// Reads and writes device registers.
pub trait HWRegisterFacility: BaseFacility {
    /// Reads a 32-bit register at `address`.
    fn read_register(&self, address: u32) -> FacilityResult<u32>;
    /// Writes a 32-bit value to `address`.
    fn write_register(&mut self, address: u32, value: u32) -> FacilityResult<()>;
}

/// Reads and writes low-level bias settings.
pub trait LLBiasesFacility: BaseFacility {
    /// Sets a named low-level bias.
    fn set(&mut self, bias_name: &str, bias_value: i32) -> FacilityResult<()>;
    /// Reads a named low-level bias.
    fn get(&self, bias_name: &str) -> FacilityResult<i32>;
    /// Returns all available bias names and values.
    fn get_all_biases(&self) -> FacilityResult<Vec<(String, i32)>>;
}

/// Exposes monitoring information such as temperature and illumination.
pub trait MonitoringFacility: BaseFacility {
    /// Returns the current device temperature.
    fn get_temperature(&self) -> FacilityResult<i32>;
    /// Returns the current illumination estimate.
    fn get_illumination(&self) -> FacilityResult<i32>;
}

/// Exposes plugin metadata.
pub trait PluginSoftwareInfoFacility: BaseFacility {
    /// Returns the plugin's display name.
    fn get_plugin_name(&self) -> String;
    /// Returns the plugin version.
    fn get_version(&self) -> String;
}

/// Controls region-of-interest settings.
pub trait ROIFacility: BaseFacility {
    property! {
        enabled: bool;
    }

    /// Sets a single region of interest.
    fn set_roi(&mut self, region: Region) -> FacilityResult<()>;
    /// Sets multiple regions of interest.
    fn set_rois(&mut self, regions: &[Region]) -> FacilityResult<()>;
    /// Returns the configured single ROI, if one exists.
    fn roi(&self) -> Option<Region>;
    /// Returns all configured ROIs, if any exist.
    fn rois(&self) -> Option<Vec<Region>>;
}

/// Controls pixel masks as a batch of individual mask entries.
pub trait ROIPixelMaskFacility: BaseFacility {
    property! {
        pixel_masks: Vec<PixelMask>;
    }
}

/// Controls external trigger input channels.
pub trait TriggerInFacility: BaseFacility {
    /// Enables an external trigger input channel.
    fn enable(&mut self, channel: u32) -> FacilityResult<()>;
    /// Disables an external trigger input channel.
    fn disable(&mut self, channel: u32) -> FacilityResult<()>;
}

/// Controls external trigger output timing.
pub trait TriggerOutFacility: BaseFacility {
    /// Enables trigger output generation.
    fn enable(&mut self) -> FacilityResult<()>;
    /// Disables trigger output generation.
    fn disable(&mut self) -> FacilityResult<()>;
    /// Sets the output period in microseconds.
    fn set_period(&mut self, period_us: u32) -> FacilityResult<()>;
    /// Sets the output duty cycle as a fraction or percentage defined by the implementation.
    fn set_duty_cycle(&mut self, duty_cycle: f64) -> FacilityResult<()>;
}
