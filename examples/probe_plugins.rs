//! Probe: load device plugins from OPENEVT_PLUGIN_PATH and report why
//! opening a raw EVT3 file fails, surfacing the error the Python layer swallows.

use openevt::PluginRegistry;

const LIBRARY_EXTENSIONS: [&str; 3] = ["so", "dll", "dylib"];

fn main() {
    let path = std::env::args().nth(1).expect("usage: probe_plugins <file.raw>");

    println!("OPENEVT_PLUGIN_PATH = {:?}", std::env::var("OPENEVT_PLUGIN_PATH"));

    let mut registry = PluginRegistry::new();

    for entry in std::fs::read_dir(std::env::var("OPENEVT_PLUGIN_PATH").unwrap())
        .expect("plugin dir")
    {
        let path = entry.expect("entry").path();
        let is_library = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|extension| LIBRARY_EXTENSIONS.contains(&extension))
            .unwrap_or(false);
        if is_library {
            match registry.load_file(&path) {
                Ok(()) => println!("load_file OK: {}", path.display()),
                Err(error) => println!("load_file FAILED {}: {error}", path.display()),
            }
        }
    }

    for device in registry.list_devices() {
        println!("discovered: {device:?}");
    }

    match registry.open_device(&path) {
        Ok(_) => println!("registry.open_device OK"),
        Err(error) => println!("registry.open_device FAILED: {error}"),
    }
}
