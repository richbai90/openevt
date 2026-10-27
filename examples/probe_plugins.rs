//! Probe: load device plugins from OPENEVT_PLUGIN_PATH and report why
//! opening a raw EVT3 file fails, surfacing the error the Python layer swallows.

use openevt::hal::device::discovery::PluginRegistry;
use openevt::RawFilePlugin;

fn main() {
    let path = std::env::args().nth(1).expect("usage: probe_plugins <file.raw>");

    println!("OPENEVT_PLUGIN_PATH = {:?}", std::env::var("OPENEVT_PLUGIN_PATH"));

    let mut registry = PluginRegistry::new();
    let loaded = registry.load_default_paths();
    println!("load_default_paths loaded {loaded} plugin(s)");

    for entry in std::fs::read_dir(std::env::var("OPENEVT_PLUGIN_PATH").unwrap())
        .expect("plugin dir")
    {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("so") {
            match registry.load_file(&path) {
                Ok(()) => println!("load_file OK: {}", path.display()),
                Err(error) => println!("load_file FAILED {}: {error}", path.display()),
            }
        }
    }

    for device in registry.list_devices() {
        println!("discovered: {device:?}");
    }

    match RawFilePlugin::open(&path) {
        Ok(_) => println!("RawFilePlugin::open OK"),
        Err(error) => println!("RawFilePlugin::open FAILED: {error}"),
    }

    match registry.open_device(&path) {
        Ok(_) => println!("registry.open_device OK"),
        Err(error) => println!("registry.open_device FAILED: {error}"),
    }
}
