//! `abi_stable` device plugin exposing [`RawFileReader`] as a replay device.
//!
//! The adapter deliberately reuses the native implementation: file parsing,
//! EVT3 decoding, timestamp normalization, ROI state, and channel creation are
//! not duplicated here. Crossbeam receivers remain private to the plugin. A
//! decoded channel batch is delivered through [`EventBatchSink`] as an
//! ABI-safe borrowed slice, preserving the native one-batch-per-notification
//! behavior without placing crossbeam types in the shared-library ABI.
//!
//! To advertise files for discovery, set `OPENEVT_RAW_FILES` to a platform
//! separated list of EVT3 paths. The discovery serial is the path itself, and
//! the creation schema asks the application to provide the selected file as a
//! semantic `file` value through the host layer API.
//!
//! Event flow is pull-driven and matches the native synchronous reader:
//! `start_events` subscribes to the CD receiver, `load_batch` decodes one raw
//! buffer, and every batch produced by that decode is sent to the callback.

use abi_stable::{
    std_types::{ROption, RResult, RString, RVec},
    type_level::downcasting::TD_Opaque,
};
use crossbeam::channel::Receiver;
use std::sync::{Arc, Mutex};

use openevt_abi::configuration::PluginConfigurationSchema;
use openevt_core::hal::device::discovery::ConnectionType;
use openevt_abi::plugin::{
    DeviceDiscoveryPlugin, DeviceDiscoveryPlugin_TO, DeviceDiscoveryPluginBox, DevicePlugin_TO,
    DevicePluginBox, PluginCameraDescriptionAbi, PluginConfiguration,
};
use openevt_abi::plugin::{
    DevicePlugin, EventBatchSinkBox, PluginFacility, PluginFacilityHandle, PluginFacilityType,
    PluginGeometry, PluginGeometryFacility, PluginGeometryFacility_TO,
};
use openevt_core::types::{EventCD, EventExtTrigger, EventTimestamp};
use openevt::RawFileReader;

// Keep the example's stack footprint aligned with the normal reader tests;
// callers can introduce a larger buffer as a versioned plugin policy.
const BUFFER_SIZE: usize = 131_072;

pub(crate) const RAW_FILE_CONFIGURATION_SCHEMA: &str = r#"
version = 1

[[parameters]]
name = "input_file"
label = "Input event file"
kind = "file"
required = true
description = "An EVT3 event file to replay."
extensions = ["raw", "evt3"]
"#;

/// Plugin adapter exposing a raw event file as a device.
pub struct RawFilePlugin {
    serial: RString,
    reader: Arc<Mutex<RawFileReader<BUFFER_SIZE>>>,
    cd_receiver: Option<Receiver<Vec<EventCD>>>,
    ext_receiver: Option<Receiver<Vec<EventExtTrigger>>>,
    sink: Option<EventBatchSinkBox>,
}

impl RawFilePlugin {
    /// Opens and parses a raw event file for use through the plugin ABI.
    pub fn open(path: &str) -> Result<Self, String> {
        Ok(Self {
            serial: path.into(),
            reader: Arc::new(Mutex::new(
                RawFileReader::try_from_file(path, false).map_err(|e| e.to_string())?,
            )),
            cd_receiver: None,
            ext_receiver: None,
            sink: None,
        })
    }

    fn facility(facility_type: PluginFacilityType) -> PluginFacility {
        PluginFacility { facility_type }
    }

    fn drain_events(&mut self) {
        if let Some(receiver) = &self.cd_receiver {
            while let Ok(batch) = receiver.try_recv() {
                if let Some(sink) = &self.sink {
                    sink.on_cd_events(batch.as_slice().into());
                }
            }
        }
        if let Some(receiver) = &self.ext_receiver {
            while let Ok(batch) = receiver.try_recv() {
                if let Some(sink) = &self.sink {
                    sink.on_ext_events(batch.as_slice().into());
                }
            }
        }
    }

    fn result(result: Result<(), String>) -> RResult<(), RString> {
        match result {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(error.into()),
        }
    }
}

struct RawGeometryFacility {
    width: u32,
    height: u32,
}

struct RawIndexFacility<const N: usize> {
    reader: Arc<Mutex<RawFileReader<N>>>,
}
impl<const N: usize> openevt_abi::plugin::PluginIndexFacility for RawIndexFacility<N> {
    fn t_min(&self) -> ROption<EventTimestamp> {
        self.reader.lock().unwrap().t_min().into()
    }
    fn t_max(&self) -> ROption<EventTimestamp> {
        self.reader.lock().unwrap().t_max().into()
    }
}

struct RawSeekFacility<const N: usize> {
    reader: Arc<Mutex<RawFileReader<N>>>,
}
impl<const N: usize> openevt_abi::plugin::PluginSeekFacility for RawSeekFacility<N> {
    fn seek(&mut self, timestamp: EventTimestamp) -> RResult<(), RString> {
        match self.reader.lock().unwrap().seek(timestamp) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(error.to_string().into()),
        }
    }
}

struct RawExternalTriggerSeekFacility<const N: usize> {
    reader: Arc<Mutex<RawFileReader<N>>>,
}
impl<const N: usize> openevt_abi::plugin::PluginExternalTriggerSeekFacility
    for RawExternalTriggerSeekFacility<N>
{
    fn seek_to_next_ext(&mut self) -> RResult<(), RString> {
        match self.reader.lock().unwrap().seek_to_next_ext() {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(error.to_string().into()),
        }
    }
}

impl PluginGeometryFacility for RawGeometryFacility {
    fn get_width(&self) -> u32 {
        self.width
    }
    fn get_height(&self) -> u32 {
        self.height
    }
}

impl DevicePlugin for RawFilePlugin {
    fn serial(&self) -> RString {
        self.serial.clone()
    }

    fn connection_type(&self) -> ConnectionType {
        ConnectionType::Proprietary
    }

    fn geometry(&self) -> PluginGeometry {
        let (height, width) = self.reader.lock().unwrap().shape();
        PluginGeometry { width, height }
    }

    fn t_min(&self) -> ROption<EventTimestamp> {
        self.reader.lock().unwrap().t_min().into()
    }

    fn t_max(&self) -> ROption<EventTimestamp> {
        self.reader.lock().unwrap().t_max().into()
    }

    fn seek(&mut self, timestamp: EventTimestamp) -> RResult<(), RString> {
        Self::result(
            self.reader
                .lock()
                .unwrap()
                .seek(timestamp)
                .map_err(|error| error.to_string()),
        )
    }

    fn seek_to_next_ext(&mut self) -> RResult<(), RString> {
        Self::result(
            self.reader
                .lock()
                .unwrap()
                .seek_to_next_ext()
                .map_err(|error| error.to_string()),
        )
    }

    fn get_facilities(&self) -> RVec<PluginFacilityType> {
        vec![
            PluginFacilityType::Geometry,
            PluginFacilityType::Index,
            PluginFacilityType::Seek,
            PluginFacilityType::ExternalTriggerSeek,
        ]
        .into()
    }

    fn get_facility(&self, facility_type: PluginFacilityType) -> ROption<PluginFacility> {
        if self.get_facilities().contains(&facility_type) {
            Some(Self::facility(facility_type)).into()
        } else {
            ROption::RNone
        }
    }

    fn get_facility_handle(
        &self,
        facility_type: PluginFacilityType,
    ) -> ROption<PluginFacilityHandle> {
        match facility_type {
            PluginFacilityType::Geometry => {
                let (height, width) = self.reader.lock().unwrap().shape();
                Some(PluginFacilityHandle::Geometry(
                    PluginGeometryFacility_TO::from_value(
                        RawGeometryFacility { width, height },
                        TD_Opaque,
                    ),
                ))
                .into()
            }
            PluginFacilityType::Index => Some(PluginFacilityHandle::Index(
                openevt_abi::plugin::PluginIndexFacility_TO::from_value(
                    RawIndexFacility {
                        reader: Arc::clone(&self.reader),
                    },
                    TD_Opaque,
                ),
            ))
            .into(),
            PluginFacilityType::Seek => Some(PluginFacilityHandle::Seek(
                openevt_abi::plugin::PluginSeekFacility_TO::from_value(
                    RawSeekFacility {
                        reader: Arc::clone(&self.reader),
                    },
                    TD_Opaque,
                ),
            ))
            .into(),
            PluginFacilityType::ExternalTriggerSeek => {
                Some(PluginFacilityHandle::ExternalTriggerSeek(
                    openevt_abi::plugin::PluginExternalTriggerSeekFacility_TO::from_value(
                        RawExternalTriggerSeekFacility {
                            reader: Arc::clone(&self.reader),
                        },
                        TD_Opaque,
                    ),
                ))
                .into()
            }
            _ => ROption::RNone,
        }
    }

    fn start_events(&mut self, sink: EventBatchSinkBox) -> RResult<(), RString> {
        let result = self.reader.lock().unwrap().cd_receiver();
        match result {
            Ok(receiver) => {
                self.cd_receiver = Some(receiver);
                self.sink = Some(sink);
                RResult::ROk(())
            }
            Err(error) => RResult::RErr(error.to_string().into()),
        }
    }

    fn start_external_triggers(&mut self, sink: EventBatchSinkBox) -> RResult<(), RString> {
        let result = self.reader.lock().unwrap().ext_receiver();
        match result {
            Ok(receiver) => {
                self.ext_receiver = Some(receiver);
                self.sink = Some(sink);
                RResult::ROk(())
            }
            Err(error) => RResult::RErr(error.to_string().into()),
        }
    }

    fn load_batch(&mut self) -> RResult<(), RString> {
        let result = self.reader.lock().unwrap().load_batch();
        match result {
            Ok(Some(())) => {
                self.drain_events();
                RResult::ROk(())
            }
            Ok(None) => RResult::RErr("End of file reached".into()),
            Err(error) => RResult::RErr(error.to_string().into()),
        }
    }
}

/// Discovery interface advertising EVT3 files as replay devices.
pub struct RawFileDiscovery;

impl DeviceDiscoveryPlugin for RawFileDiscovery {
    fn discover(&self) -> RVec<PluginCameraDescriptionAbi> {
        std::env::var_os("OPENEVT_RAW_FILES")
            .map(|value| {
                std::env::split_paths(&value)
                    .map(|path| PluginCameraDescriptionAbi {
                        serial: path.to_string_lossy().into(),
                        connection: ConnectionType::Proprietary,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn configuration_schema(&self) -> RString {
        RAW_FILE_CONFIGURATION_SCHEMA.into()
    }

    fn open_device(
        &self,
        serial: abi_stable::std_types::RStr<'_>,
    ) -> RResult<DevicePluginBox, RString> {
        match RawFilePlugin::open(serial.as_str()) {
            Ok(plugin) => RResult::ROk(DevicePlugin_TO::from_value(plugin, TD_Opaque)),
            Err(error) => RResult::RErr(error.into()),
        }
    }

    fn open_device_with_configuration(
        &self,
        configuration: PluginConfiguration,
    ) -> RResult<DevicePluginBox, RString> {
        let schema = match PluginConfigurationSchema::parse(RAW_FILE_CONFIGURATION_SCHEMA) {
            Ok(schema) => schema,
            Err(error) => return RResult::RErr(error.to_string().into()),
        };
        if let Err(error) = schema.validate(&configuration) {
            return RResult::RErr(error.to_string().into());
        }
        let input_file = match configuration
            .values
            .iter()
            .find(|value| value.name.as_str() == "input_file")
            .and_then(|value| value.value.as_ref().into_option())
        {
            Some(path) => path.as_str(),
            None => return RResult::RErr("required parameter `input_file` is missing".into()),
        };
        match RawFilePlugin::open(input_file) {
            Ok(plugin) => RResult::ROk(DevicePlugin_TO::from_value(plugin, TD_Opaque)),
            Err(error) => RResult::RErr(error.into()),
        }
    }
}

/// Name this device is advertised under.
pub const RAW_FILE_PLUGIN_NAME: &str = "openevt_raw_file";

/// Builds the raw-file discovery interface for the crate root module.
pub extern "C" fn create_raw_discovery() -> DeviceDiscoveryPluginBox {
    DeviceDiscoveryPlugin_TO::from_value(RawFileDiscovery, TD_Opaque)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openevt_abi::plugin::{EventBatchSink, EventBatchSink_TO};
    use abi_stable::std_types::RSlice;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CD_BATCHES: AtomicUsize = AtomicUsize::new(0);
    static CD_EVENTS: AtomicUsize = AtomicUsize::new(0);

    struct CountingSink;

    impl EventBatchSink for CountingSink {
        fn on_cd_events(&self, events: RSlice<'_, EventCD>) {
            CD_BATCHES.fetch_add(1, Ordering::Relaxed);
            CD_EVENTS.fetch_add(events.len(), Ordering::Relaxed);
        }

        fn on_ext_events(&self, _events: RSlice<'_, EventExtTrigger>) {}
    }

    #[test]
    fn raw_plugin_preserves_decoded_cd_batch_callbacks() {
        // Shares the openevt-devices crate's EVT3 fixture rather than duplicating it.
        let path = format!(
            "{}/../crates/openevt-devices/tests/sample.raw",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut plugin = RawFilePlugin::open(&path).expect("sample EVT3 file should open");
        CD_BATCHES.store(0, Ordering::Relaxed);
        CD_EVENTS.store(0, Ordering::Relaxed);
        let sink_box = EventBatchSink_TO::from_value(CountingSink, TD_Opaque);

        assert!(matches!(plugin.start_events(sink_box), RResult::ROk(())));
        assert!(matches!(plugin.load_batch(), RResult::ROk(())));
        assert!(CD_BATCHES.load(Ordering::Relaxed) > 0);
        assert!(CD_EVENTS.load(Ordering::Relaxed) > 0);
    }
}
