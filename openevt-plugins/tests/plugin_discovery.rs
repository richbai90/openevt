use openevt::hal::device::discovery::PluginRegistry;
use std::env;
use std::path::PathBuf;

fn plugin_directories() -> Vec<PathBuf> {
    match env::var_os("OPENEVT_PLUGIN_PATH") {
        Some(configured_path) => env::split_paths(&configured_path).collect(),
        None => vec![
            env::current_exe()
                .expect("test executable path should be available")
                .parent()
                .expect("test executable should have a parent directory")
                .to_path_buf(),
        ],
    }
}

fn loaded_registry() -> PluginRegistry {
    let directories = plugin_directories();

    assert!(
        directories.iter().any(|path| path.is_dir()),
        "OPENEVT_PLUGIN_PATH does not contain an existing directory: {:?}",
        directories
    );

    let mut registry = PluginRegistry::new();
    let loaded = directories
        .iter()
        .map(|path| registry.load_directory(path))
        .sum::<u64>();
    assert!(
        loaded > 0,
        "no plugin libraries could be loaded from {:?}",
        directories
    );

    registry
}

/// Loads only this crate's own plugin library.
///
/// Scanning the whole directory would also load any sibling vendor plugin and
/// run its `discover()`, which for a real camera means touching hardware. These
/// tests are about this crate, so they stay off that path.
fn this_library_only() -> PluginRegistry {
    let file_name = format!(
        "{}openevt_plugins{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );

    let candidate = plugin_directories()
        .into_iter()
        .map(|directory| directory.join(&file_name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("{file_name} was not found in {:?}", plugin_directories()));

    let mut registry = PluginRegistry::new();
    registry
        .load_file(&candidate)
        .unwrap_or_else(|error| panic!("{} failed to load: {error}", candidate.display()));
    registry
}

#[test]
fn configured_plugin_path_discovers_the_simulator() {
    let simulator = loaded_registry()
        .list_devices()
        .into_iter()
        .find(|device| device.plugin_name == "openevt_simulator")
        .expect("the simulator plugin was loaded but did not advertise its device");

    assert_eq!(simulator.plugin_name, "openevt_simulator");
    assert_eq!(simulator.plugin_info.serial, "EventSimulator");
}

/// The simulator and the raw-file reader ship in one shared library, which can
/// only export a single `abi_stable` root module. Both must still reach the
/// registry as independently named devices.
#[test]
fn one_library_registers_both_standard_devices() {
    let registry = this_library_only();

    // SAFETY: single-threaded test; no other thread reads the environment.
    unsafe {
        env::set_var("OPENEVT_RAW_FILES", "/tmp/openevt-discovery-probe.raw");
    }
    let devices = registry.list_devices();
    unsafe {
        env::remove_var("OPENEVT_RAW_FILES");
    }

    let names: Vec<&str> = devices
        .iter()
        .map(|device| device.plugin_name.as_str())
        .collect();

    assert!(
        names.contains(&"openevt_simulator"),
        "simulator missing from {names:?}"
    );
    assert!(
        names.contains(&"openevt_raw_file"),
        "raw-file reader missing from {names:?}"
    );
}

/// The raw reader accepts any path as its serial, so a recording opens without
/// having been advertised through `OPENEVT_RAW_FILES` first. A path that is not
/// a readable event file must therefore fail with the reader's own complaint,
/// not with a bare "nothing claimed this serial".
#[test]
fn raw_reader_reports_why_a_bad_path_failed() {
    let error = match this_library_only().open_device("/nonexistent/openevt-not-a-recording.raw") {
        Ok(_) => panic!("a missing file must not open"),
        Err(error) => error,
    };

    assert!(
        error.contains("openevt_raw_file"),
        "the raw reader should have been tried and named in the error: {error}"
    );
}
