#[cfg_attr(feature = "plugins", derive(abi_stable::StableAbi))]
#[cfg_attr(feature = "plugins", repr(C))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Transport used to connect a camera plugin to the host layer.
pub enum ConnectionType {
    /// MIPI camera connection.
    Mipi,
    /// USB camera connection.
    Usb,
    /// Network camera connection.
    Network,
    /// Vendor-specific connection.
    Proprietary,
    /// Software only device
    Software,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Description of a camera plugin and its connection.
pub struct PluginCameraDescription {
    /// Camera serial number.
    pub serial: String,
    /// Connection transport.
    pub connection: ConnectionType,
}

impl PluginCameraDescription {
    /// Creates a plugin description from a serial number and transport.
    pub fn new(serial: &str, connection: ConnectionType) -> Self {
        Self {
            serial: serial.to_owned(),
            connection,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Full camera description, including its integrator and plugin identity.
pub struct CameraDescription {
    /// Name of the camera integrator.
    pub integrator_name: String,
    /// Name of the camera plugin.
    pub plugin_name: String,
    /// Plugin-specific camera details.
    pub plugin_info: PluginCameraDescription,
}

impl CameraDescription {
    /// Creates a complete camera description.
    pub fn new(
        integrator_name: String,
        plugin_name: String,
        plugin_info: PluginCameraDescription,
    ) -> Self {
        Self {
            integrator_name,
            plugin_name,
            plugin_info,
        }
    }
}

impl From<PluginCameraDescription> for CameraDescription {
    fn from(plugin_info: PluginCameraDescription) -> Self {
        Self {
            integrator_name: String::new(),
            plugin_name: String::new(),
            plugin_info,
        }
    }
}
