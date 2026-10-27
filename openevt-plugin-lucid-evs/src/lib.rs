mod device;
mod discovery;

use abi_stable::prefix_type::PrefixTypeTrait;
use abi_stable::std_types::{RString, RVec};
use abi_stable::type_level::downcasting::TD_Opaque;
use discovery::LucidEvsDiscovery;
use openevt_abi::plugin::{
    DeviceDiscoveryPlugin_TO, DeviceDiscoveryPluginBox, DevicePluginModuleRef,
    DevicePluginModuleVtable, NamedDiscoveryPlugin,
};

const PLUGIN_NAME: &str = "lucid_triton_evs";

extern "C" fn plugin_name() -> RString {
    PLUGIN_NAME.into()
}

extern "C" fn create_discovery() -> DeviceDiscoveryPluginBox {
    DeviceDiscoveryPlugin_TO::from_value(LucidEvsDiscovery, TD_Opaque)
}

extern "C" fn create_discoveries() -> RVec<NamedDiscoveryPlugin> {
    vec![NamedDiscoveryPlugin {
        name: PLUGIN_NAME.into(),
        discovery: create_discovery(),
    }]
    .into()
}

#[abi_stable::export_root_module]
pub fn instantiate_root_module() -> DevicePluginModuleRef {
    DevicePluginModuleVtable {
        name: plugin_name,
        create_discovery,
        create_discoveries,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::CONFIGURATION_SCHEMA;
    use openevt_abi::configuration::PluginConfigurationSchema;
    use openevt_abi::plugin::DeviceDiscoveryPlugin;

    #[test]
    fn plugin_name_is_stable() {
        assert_eq!(plugin_name().as_str(), "lucid_triton_evs");
    }

    #[test]
    fn configuration_schema_is_valid() {
        let schema = PluginConfigurationSchema::parse(CONFIGURATION_SCHEMA).unwrap();
        let names: Vec<_> = schema.parameters.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "frame_timeout_ms",
                "destination_port",
                "packet_size",
                "sensor_width",
                "sensor_height"
            ]
        );
    }

    #[test]
    fn discovery_starts_empty_without_network_response() {
        let discovery = LucidEvsDiscovery;
        let _ = discovery.discover();
    }
}
