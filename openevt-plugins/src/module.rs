//! The crate's single `abi_stable` root module.
//!
//! A shared library can only export one root module, so every device in this
//! collection is advertised through one vtable: [`instantiate_root_module`]
//! returns each device as a [`NamedDiscoveryPlugin`], letting the host register
//! them individually while keeping their own names and creation schemas.
//!
//! `create_discovery` still resolves to the simulator so that a host built
//! against the single-device vtable keeps loading this library.

use abi_stable::{
    prefix_type::PrefixTypeTrait,
    std_types::{RString, RVec},
};
use openevt_abi::plugin::{
    DeviceDiscoveryPluginBox, DevicePluginModuleRef, DevicePluginModuleVtable, NamedDiscoveryPlugin,
};

use crate::raw::{RAW_FILE_PLUGIN_NAME, create_raw_discovery};
use crate::simulator::plugin::{SIMULATOR_PLUGIN_NAME, create_simulator_discovery};

extern "C" fn standard_plugins_name() -> RString {
    "openevt_standard_plugins".into()
}

extern "C" fn create_default_discovery() -> DeviceDiscoveryPluginBox {
    create_simulator_discovery()
}

extern "C" fn create_standard_discoveries() -> RVec<NamedDiscoveryPlugin> {
    vec![
        NamedDiscoveryPlugin {
            name: SIMULATOR_PLUGIN_NAME.into(),
            discovery: create_simulator_discovery(),
        },
        NamedDiscoveryPlugin {
            name: RAW_FILE_PLUGIN_NAME.into(),
            discovery: create_raw_discovery(),
        },
    ]
    .into()
}

/// Constructs the standard-plugin collection root module.
#[abi_stable::export_root_module]
pub fn instantiate_root_module() -> DevicePluginModuleRef {
    DevicePluginModuleVtable {
        name: standard_plugins_name,
        create_discovery: create_default_discovery,
        create_discoveries: create_standard_discoveries,
    }
    .leak_into_prefix()
}
