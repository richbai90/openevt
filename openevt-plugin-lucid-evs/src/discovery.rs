use crate::device::{DEFAULT_HEIGHT, DEFAULT_WIDTH, LucidEvsDevice};
use abi_stable::std_types::{RResult, RString, RVec};
use abi_stable::type_level::downcasting::TD_Opaque;
use openevt_core::hal::device::discovery::ConnectionType;
use openevt_abi::plugin::{
    DeviceDiscoveryPlugin, DevicePlugin_TO, DevicePluginBox, PluginCameraDescriptionAbi,
    PluginConfiguration,
};
use std::net::Ipv4Addr;
use std::time::Duration;
use viva_genicam::gige::DeviceInfo as VivaDeviceInfo;

pub const CONFIGURATION_SCHEMA: &str = r#"
version = 1

[[parameters]]
name = "frame_timeout_ms"
label = "Frame timeout (ms)"
kind = "integer"
required = false
default = "100"
description = "Maximum time to wait while assembling one GigE Vision block."

[[parameters]]
name = "destination_port"
label = "GVSP destination port"
kind = "integer"
required = false
default = "10040"
description = "UDP port the camera sends GVSP packets to. Triton EVS cameras default to 10040."

[[parameters]]
name = "packet_size"
label = "GVSP packet size (bytes)"
kind = "integer"
required = false
default = "0"
description = "GVSP packet size ceiling in bytes. 0 (default) probes the largest packet size the network path supports on connection, starting from the host NIC MTU. Set a specific value (e.g. 8192 or 1500) to use it as the negotiated ceiling instead."

[[parameters]]
name = "sensor_width"
label = "Sensor width (px)"
kind = "integer"
required = false
default = "640"
description = "Actual sensor width in pixels. Events outside these coordinate bounds are discarded by the EVT3 decoder, so this must match the camera's real resolution (e.g. 1280 for the TRT009S-E)."

[[parameters]]
name = "sensor_height"
label = "Sensor height (px)"
kind = "integer"
required = false
default = "480"
description = "Actual sensor height in pixels. Events outside these coordinate bounds are discarded by the EVT3 decoder, so this must match the camera's real resolution (e.g. 720 for the TRT009S-E)."
"#;

/// Default GVCP discovery timeout used when the plugin auto-detects cameras.
const AUTO_DISCOVERY_TIMEOUT: Duration = Duration::from_millis(250);

/// Sentinel serial values that mean "pick the first auto-discovered LUCID camera".
const AUTO_DISCOVERY_SERIALS: [&str; 3] = ["", "auto", "*"];

pub struct LucidEvsDiscovery;

impl LucidEvsDiscovery {
    /// Run a single GVCP discovery cycle and filter to LUCID devices.
    ///
    /// Used by both the ABI `discover()` method (which only needs the IP and
    /// connection type) and the auto-open path (which needs the full
    /// `DeviceInfo` to call `connect_gige`).
    fn discover_lucid_devices() -> Vec<VivaDeviceInfo> {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => return Vec::new(),
        };

        let devices = runtime
            .block_on(viva_genicam::gige::discover(AUTO_DISCOVERY_TIMEOUT))
            .unwrap_or_default();

        devices
            .into_iter()
            .filter(|device| is_lucid(device))
            .collect()
    }

    /// Resolve `serial` to a concrete `VivaDeviceInfo`.
    ///
    /// Accepts either a direct IPv4 address (current behaviour) or one of the
    /// auto-discovery sentinels (`""`, `"auto"`, `"*"`), in which case the
    /// first auto-discovered LUCID camera is selected.
    fn resolve_camera(serial: &str) -> Result<VivaDeviceInfo, String> {
        if AUTO_DISCOVERY_SERIALS.contains(&serial) {
            let mut cameras = Self::discover_lucid_devices();
            return cameras.pop().ok_or_else(|| {
                "auto-discovery requested but no LUCID GigE Vision cameras responded".to_owned()
            });
        }

        let ip: Ipv4Addr = serial
            .parse()
            .map_err(|error| format!("invalid camera IP `{serial}`: {error}"))?;
        Ok(VivaDeviceInfo::from_ip(ip))
    }
}

fn is_lucid(device: &VivaDeviceInfo) -> bool {
    device
        .manufacturer
        .as_deref()
        .is_some_and(|manufacturer| manufacturer.to_ascii_lowercase().contains("lucid"))
}

impl DeviceDiscoveryPlugin for LucidEvsDiscovery {
    fn discover(&self) -> RVec<PluginCameraDescriptionAbi> {
        Self::discover_lucid_devices()
            .into_iter()
            .map(|device| PluginCameraDescriptionAbi {
                serial: device.ip.to_string().into(),
                connection: ConnectionType::Network,
            })
            .collect()
    }

    fn configuration_schema(&self) -> RString {
        CONFIGURATION_SCHEMA.into()
    }

    fn open_device(
        &self,
        serial: abi_stable::std_types::RStr<'_>,
    ) -> RResult<DevicePluginBox, RString> {
        self.open_device_with_configuration(PluginConfiguration {
            serial: serial.to_string().into(),
            values: RVec::new(),
        })
    }

    fn open_device_with_configuration(
        &self,
        configuration: PluginConfiguration,
    ) -> RResult<DevicePluginBox, RString> {
        let device = match Self::resolve_camera(configuration.serial.as_str()) {
            Ok(device) => device,
            Err(error) => return RResult::RErr(error.into()),
        };

        let parsed_integer = |name: &str, default: u64| -> Result<u64, String> {
            let raw = configuration
                .values
                .iter()
                .find(|value| value.name.as_str() == name)
                .and_then(|value| value.value.clone().into_option());
            let value = match raw {
                Some(raw) => raw
                    .as_str()
                    .parse::<u64>()
                    .map_err(|error| format!("invalid {name}: {error}"))?,
                None => default,
            };
            if value == 0 {
                Err(format!("{name} must be greater than zero"))
            } else if value > u16::MAX as u64 {
                Err(format!("{name} must fit in a u16"))
            } else {
                Ok(value)
            }
        };

        let frame_timeout_ms = match parsed_integer("frame_timeout_ms", 100) {
            Ok(value) => value,
            Err(error) => return RResult::RErr(error.into()),
        };
        let destination_port = match parsed_integer("destination_port", 10040) {
            Ok(value) => value as u16,
            Err(error) => return RResult::RErr(error.into()),
        };
        let fit_u16 = |name: &str, value: u64| -> Result<u16, RString> {
            u16::try_from(value).map_err(|_| format!("{name} must fit in a u16").into())
        };
        let sensor_width = match parsed_integer("sensor_width", u64::from(DEFAULT_WIDTH)) {
            Ok(value) => match fit_u16("sensor_width", value) {
                Ok(value) => value,
                Err(error) => return RResult::RErr(error),
            },
            Err(error) => return RResult::RErr(error.into()),
        };
        let sensor_height = match parsed_integer("sensor_height", u64::from(DEFAULT_HEIGHT)) {
            Ok(value) => match fit_u16("sensor_height", value) {
                Ok(value) => value,
                Err(error) => return RResult::RErr(error),
            },
            Err(error) => return RResult::RErr(error.into()),
        };
        // 0 or absent means "probe the path for the largest workable packet
        // size"; any other value is an explicit ceiling handed to the stream
        // builder, which still path-probes below it.
        let packet_size = configuration
            .values
            .iter()
            .find(|value| value.name.as_str() == "packet_size")
            .and_then(|value| value.value.clone().into_option());
        let packet_size = match packet_size {
            Some(raw) => match raw.as_str().parse::<u32>() {
                Ok(0) => None,
                Ok(value) => Some(value),
                Err(error) => return RResult::RErr(format!("invalid packet_size: {error}").into()),
            },
            None => None,
        };

        match LucidEvsDevice::open(
            device.ip,
            Duration::from_millis(frame_timeout_ms),
            destination_port,
            packet_size,
            sensor_width,
            sensor_height,
        ) {
            Ok(opened) => RResult::ROk(DevicePlugin_TO::from_value(opened, TD_Opaque)),
            Err(error) => RResult::RErr(error.into()),
        }
    }
}
