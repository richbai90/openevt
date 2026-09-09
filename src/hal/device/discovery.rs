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

/// Directories searched for device plugin libraries.
#[cfg(feature = "plugins")]
pub fn default_plugin_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = env::var_os("OPENEVT_PLUGIN_PATH")
        .map(|value| env::split_paths(&value).collect())
        .unwrap_or_default();
    paths.extend([
        PathBuf::from("/usr/lib/openevt/plugins"),
        PathBuf::from("/usr/local/lib/openevt/plugins"),
    ]);
    paths
}

#[cfg(feature = "plugins")]
fn is_library(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("so" | "dll" | "dylib")
    )
}

/// Runtime registry of loaded discovery plugins.
#[cfg(feature = "plugins")]
pub struct PluginRegistry {
    modules: Vec<(DevicePluginModuleRef, DeviceDiscoveryPluginBox)>,
}

#[cfg(feature = "plugins")]
impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "plugins")]
impl PluginRegistry {
    /// Creates an empty plugin registry.
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
    }

    /// Loads every ABI-compatible library found in the configured directories.
    /// Invalid or unrelated shared libraries are skipped, allowing a directory
    /// to contain other vendor libraries.
    pub fn load_default_paths(&mut self) -> u64 {
        let mut loaded = 0;
        for directory in default_plugin_paths() {
            loaded += self.load_directory(&directory);
        }
        loaded
    }

    /// Loads compatible plugin libraries directly from `directory`.
    ///
    /// Returns the number of libraries successfully loaded. Missing
    /// directories and individual load failures are ignored.
    pub fn load_directory(&mut self, directory: &Path) -> u64 {
        let Ok(entries) = fs::read_dir(directory) else {
            return 0;
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_library(path))
            .filter_map(|path| self.load_file(&path).ok())
            .count() as u64
    }

    /// Loads one plugin library and registers its discovery interface.
    pub fn load_file(&mut self, path: &Path) -> Result<(), abi_stable::library::LibraryError> {
        let module = DevicePluginModuleRef::load_from_file(path)?;
        let discovery = (module
            .create_discovery()
            .expect("plugin constructor missing"))();
        self.modules.push((module, discovery));
        Ok(())
    }

    /// Discovers cameras exposed by all loaded plugins.
    pub fn list_devices(&self) -> Vec<CameraDescription> {
        self.modules
            .iter()
            .flat_map(|(module, discovery)| {
                let plugin_name = module.name().expect("plugin name symbol missing")().to_string();
                discovery
                    .discover()
                    .into_iter()
                    .map(move |info| CameraDescription {
                        integrator_name: plugin_name.clone(),
                        plugin_name: plugin_name.clone(),
                        plugin_info: info.into(),
                    })
            })
            .collect()
    }

    /// Opens the plugin device matching `serial`.
    pub fn open_device(&self, serial: &str) -> Result<super::plugin::DevicePluginBox, String> {
        for (_, discovery) in &self.modules {
            match discovery.open_device(serial.into()) {
                abi_stable::std_types::RResult::ROk(device) => return Ok(device),
                abi_stable::std_types::RResult::RErr(_) => continue,
            }
        }
        Err(format!("no plugin device found for serial {serial}"))
    }

    /// Retrieves and parses the creation schema for a discovered device.
    pub fn configuration_schema(
        &self,
        serial: &str,
    ) -> Result<super::configuration::PluginConfigurationSchema, String> {
        for (_, discovery) in &self.modules {
            if discovery
                .discover()
                .iter()
                .any(|description| description.serial.as_str() == serial)
            {
                let source = discovery.configuration_schema();
                return if source.is_empty() {
                    Ok(super::configuration::PluginConfigurationSchema::empty())
                } else {
                    super::configuration::PluginConfigurationSchema::parse(source.as_str())
                        .map_err(|error| error.to_string())
                };
            }
        }
        Err(format!("no plugin device found for serial {serial}"))
    }

    /// Creates an all-optional configuration form for a discovered device.
    pub fn new_configuration(
        &self,
        serial: &str,
    ) -> Result<super::plugin::PluginConfiguration, String> {
        Ok(self.configuration_schema(serial)?.new_configuration(serial))
    }

    /// Opens a discovered device with a host-layer-populated configuration.
    pub fn open_device_with_configuration(
        &self,
        configuration: super::plugin::PluginConfiguration,
    ) -> Result<super::plugin::DevicePluginBox, String> {
        // The schema lookup is best-effort: a plugin may accept serials that
        // don't appear in its own `discover()` output (for example, an
        // auto-discovery sentinel that resolves at open time). When the
        // host has a schema for the serial we still validate the supplied
        // configuration values; otherwise we let the plugin decide.
        if let Ok(schema) = self.configuration_schema(configuration.serial.as_str()) {
            schema
                .validate(&configuration)
                .map_err(|error| error.to_string())?;
        }

        let mut last_error: Option<String> = None;
        for (_, discovery) in &self.modules {
            match discovery.open_device_with_configuration(configuration.clone()) {
                abi_stable::std_types::RResult::ROk(device) => return Ok(device),
                abi_stable::std_types::RResult::RErr(error) => {
                    last_error = Some(error.to_string());
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            format!("no plugin device found for serial {}", configuration.serial)
        }))
    }

    /// Opens a configured plugin through the native host-layer device adapter.
    pub fn open_native_device_with_configuration(
        &self,
        configuration: super::plugin::PluginConfiguration,
    ) -> Result<super::device::HostPluginDevice, String> {
        self.open_device_with_configuration(configuration)
            .map(Into::into)
    }

    /// Opens a plugin device through the ordinary host-layer
    /// [`super::device::Device`] API.
    ///
    /// The conversion is explicit and ABI-safe: the adapter only exposes
    /// facility handles for which the host layer has a matching native adapter.
    pub fn open_native_device(
        &self,
        serial: &str,
    ) -> Result<super::device::HostPluginDevice, String> {
        self.open_device(serial).map(Into::into)
    }
}

#[cfg(all(test, feature = "plugins"))]
mod tests {
    use super::*;
    use crate::hal::device::plugin::PluginCameraDescriptionAbi;

    #[test]
    fn empty_registry_has_no_devices() {
        assert!(PluginRegistry::new().list_devices().is_empty());
    }

    #[test]
    fn abi_description_converts_to_native_description() {
        let abi = PluginCameraDescriptionAbi {
            serial: "CAM-42".into(),
            connection: ConnectionType::Usb,
        };
        let native: PluginCameraDescription = abi.into();
        assert_eq!(native.serial, "CAM-42");
        assert_eq!(native.connection, ConnectionType::Usb);
    }
}

#[cfg(feature = "plugins")]
use std::path::{Path, PathBuf};
#[cfg(feature = "plugins")]
use std::{env, fs};

#[cfg(feature = "plugins")]
use super::plugin::{DeviceDiscoveryPluginBox, DevicePluginModuleRef};
#[cfg(feature = "plugins")]
use abi_stable::library::RootModule;
