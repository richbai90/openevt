//! Plugin discovery and library loading for the openevt ABI.
//!
//! Host-side registry that loads ABI-compatible plugin libraries and exposes
//! their devices through the native [`Device`](openevt_core::hal::device::device::Device)
//! interface. Native discovery descriptions live in
//! [`openevt_core::hal::device::discovery`].

use openevt_core::hal::device::discovery::CameraDescription;

/// Directories searched for device plugin libraries.
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

fn is_library(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("so" | "dll" | "dylib")
    )
}

/// Loads the plugin root module exported by one specific library.
///
/// [`abi_stable::library::RootModule::load_from_file`] is deliberately not used
/// here. It memoizes both the loaded library and the root module in
/// process-global `LateStaticRef`s keyed only on the root-module *type*, so the
/// first library to be loaded is returned for every later path as well — a host
/// that loads several plugin libraries would silently get the first one back
/// each time, and one failed load would poison every subsequent attempt.
///
/// Going through [`RawLibrary`] directly keeps each library independent. The
/// library is leaked on purpose: the returned module borrows from it for
/// `'static`, and unloading it while a device is still alive would be a
/// use-after-free.
fn load_root_module(
    path: &Path,
) -> Result<DevicePluginModuleRef, abi_stable::library::LibraryError> {
    let library = RawLibrary::load_at(path)?;

    // SAFETY: `header` borrows from the still-live `library`. On either error
    // path below it is discarded before `library` is dropped, and on success
    // `library` is leaked rather than unloaded.
    let header = unsafe { lib_header_from_raw_library(&library)? };
    let module = header.init_root_module::<DevicePluginModuleRef>()?;

    // Only now that this really is a plugin do we keep the library loaded
    // forever. Directory scans walk unrelated shared libraries too, and those
    // must be unloaded again rather than held open for the whole process.
    Box::leak(Box::new(library));
    Ok(module)
}

/// Runtime registry of loaded discovery plugins.
pub struct PluginRegistry {
    /// One entry per device, not per library: a library exporting several
    /// devices contributes one entry each, paired with the name it is
    /// advertised under. The libraries themselves are leaked by
    /// [`load_root_module`], so entries do not have to keep them alive.
    modules: Vec<(String, DeviceDiscoveryPluginBox)>,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

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

    /// Loads one plugin library and registers every discovery interface it
    /// exports.
    ///
    /// Libraries built against the multi-device vtable expose one named
    /// discovery per device; older single-device libraries only expose
    /// `create_discovery`, and are registered under the module name.
    pub fn load_file(&mut self, path: &Path) -> Result<(), abi_stable::library::LibraryError> {
        let module = load_root_module(path)?;
        let module_name = module.name().expect("plugin name symbol missing")().to_string();

        match module.create_discoveries() {
            Some(create_discoveries) => {
                for entry in create_discoveries() {
                    self.modules.push((entry.name.to_string(), entry.discovery));
                }
            }
            None => {
                let discovery = (module
                    .create_discovery()
                    .expect("plugin constructor missing"))();
                self.modules.push((module_name, discovery));
            }
        }
        Ok(())
    }

    /// Discovers cameras exposed by all loaded plugins.
    pub fn list_devices(&self) -> Vec<CameraDescription> {
        self.modules
            .iter()
            .flat_map(|(plugin_name, discovery)| {
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
    ///
    /// On failure the rejection from every plugin that was tried is reported.
    /// A bare "not found" hides the common cases behind an unrelated-looking
    /// message: no plugin library was on the search path at all, or the right
    /// plugin did claim the serial and failed for its own reason (a missing
    /// recording, a busy camera).
    pub fn open_device(&self, serial: &str) -> Result<super::plugin::DevicePluginBox, String> {
        let mut rejections: Vec<String> = Vec::new();
        for (plugin_name, discovery) in &self.modules {
            match discovery.open_device(serial.into()) {
                abi_stable::std_types::RResult::ROk(device) => return Ok(device),
                abi_stable::std_types::RResult::RErr(error) => {
                    rejections.push(format!("{plugin_name}: {error}"));
                }
            }
        }

        if rejections.is_empty() {
            return Err(format!(
                "no device plugins are loaded, so nothing could open serial \
                 {serial}; configure OPENEVT_PLUGIN_PATH"
            ));
        }
        Err(format!(
            "no plugin device found for serial {serial}; tried {} plugin(s): {}",
            rejections.len(),
            rejections.join("; ")
        ))
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
    /// [`Device`](openevt_core::hal::device::device::Device) API.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::PluginCameraDescriptionAbi;
    use openevt_core::hal::device::discovery::{ConnectionType, PluginCameraDescription};

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

use std::path::{Path, PathBuf};
use std::{env, fs};

use super::plugin::{DeviceDiscoveryPluginBox, DevicePluginModuleRef};
use abi_stable::library::{RawLibrary, lib_header_from_raw_library};
