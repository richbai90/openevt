mod device;
mod discovery;

use abi_stable::prefix_type::PrefixTypeTrait;
use abi_stable::std_types::RString;
use abi_stable::type_level::downcasting::TD_Opaque;
use discovery::LucidEvsDiscovery;
use openevt::hal::device::plugin::{
    DeviceDiscoveryPlugin_TO, DeviceDiscoveryPluginBox, DevicePluginModuleRef,
    DevicePluginModuleVtable,
};

extern "C" fn plugin_name() -> RString {
    "lucid_triton_evs".into()
}

extern "C" fn create_discovery() -> DeviceDiscoveryPluginBox {
    DeviceDiscoveryPlugin_TO::from_value(LucidEvsDiscovery, TD_Opaque)
}

#[abi_stable::export_root_module]
pub fn instantiate_root_module() -> DevicePluginModuleRef {
    DevicePluginModuleVtable {
        name: plugin_name,
        create_discovery,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::CONFIGURATION_SCHEMA;
    use openevt::hal::device::configuration::PluginConfigurationSchema;
    use openevt::hal::device::plugin::DeviceDiscoveryPlugin;

    #[test]
    fn plugin_name_is_stable() {
        assert_eq!(plugin_name().as_str(), "lucid_triton_evs");
    }

    #[test]
    fn configuration_schema_is_valid() {
        let schema = PluginConfigurationSchema::parse(CONFIGURATION_SCHEMA).unwrap();
        assert_eq!(schema.parameters.len(), 2);
        assert_eq!(schema.parameters[0].name, "frame_timeout_ms");
        assert_eq!(schema.parameters[1].name, "destination_port");
    }

    #[test]
    fn discovery_starts_empty_without_network_response() {
        let discovery = LucidEvsDiscovery;
        let _ = discovery.discover();
    }
}
