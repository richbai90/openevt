use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use abi_stable::std_types::{RResult, RSlice, RVec};
use abi_stable::type_level::downcasting::TD_Opaque;
use openevt::hal::device::discovery::PluginRegistry;
use openevt::hal::device::plugin::{EventBatchSink, EventBatchSink_TO, PluginConfiguration};
use openevt::types::{EventCD, EventExtTrigger};

struct CollectingSink {
    events: Arc<AtomicUsize>,
    batches: Arc<AtomicUsize>,
    last_batch_size: Arc<AtomicUsize>,
}

impl EventBatchSink for CollectingSink {
    fn on_cd_events(&self, events: RSlice<'_, EventCD>) {
        self.events.fetch_add(events.len(), Ordering::Relaxed);
        self.batches.fetch_add(1, Ordering::Relaxed);
        self.last_batch_size.store(events.len(), Ordering::Relaxed);
    }
    fn on_ext_events(&self, _events: RSlice<'_, EventExtTrigger>) {}
}

fn locate_plugin_artifact() -> PathBuf {
    let candidates: [String; 4] = [
        "libopenevt_plugin_lucid_evs.so".into(),
        "openevt_plugin_lucid_evs.dll".into(),
        "libopenevt_plugin_lucid_evs.dylib".into(),
        "openevt_plugin_lucid_evs.so".into(),
    ];

    let exe = std::env::current_exe().expect("test executable path should be available");
    let mut dir = exe.parent().map(|p| p.to_path_buf());
    while let Some(current) = dir {
        for candidate in &candidates {
            let path = current.join(candidate);
            if path.is_file() {
                return path;
            }
        }
        match current.parent() {
            Some(parent) if parent != current => dir = Some(parent.to_path_buf()),
            _ => break,
        }
    }

    panic!(
        "plugin cdylib not found near {}; run `cargo build -p openevt-plugin-lucid-evs` first",
        exe.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn live_camera_streams_cd_events() {
        let plugin_path = locate_plugin_artifact();
        let mut registry = PluginRegistry::new();
        registry
            .load_file(&plugin_path)
            .expect("load plugin cdylib");

        // Step 1: list devices, pick a serial from the registry.
        let devices = registry.list_devices();
        assert!(
            !devices.is_empty(),
            "no LUCID cameras discovered by the plugin"
        );
        let serial = devices[0].plugin_info.serial.to_string();

        // Step 2: open the device with that serial. The host now accepts any
        // serial the plugin is willing to handle, so the documented
        // `open_device_with_configuration` path is the right one.
        let mut device = registry
            .open_device_with_configuration(PluginConfiguration {
                serial: serial.into(),
                values: RVec::new(),
            })
            .expect("open device via documented path");

        let events = Arc::new(AtomicUsize::new(0));
        let batches = Arc::new(AtomicUsize::new(0));
        let last_batch_size = Arc::new(AtomicUsize::new(0));
        let sink = EventBatchSink_TO::from_value(
            CollectingSink {
                events: Arc::clone(&events),
                batches: Arc::clone(&batches),
                last_batch_size: Arc::clone(&last_batch_size),
            },
            TD_Opaque,
        );
        assert!(matches!(device.start_events(sink), RResult::ROk(())));

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let _ = device.load_batch();
            if events.load(Ordering::Relaxed) > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        let total_events = events.load(Ordering::Relaxed);
        let total_batches = batches.load(Ordering::Relaxed);
        let last_size = last_batch_size.load(Ordering::Relaxed);
        println!(
            "Collected {total_events} events across {total_batches} batches (last batch: {last_size} events)"
        );

        assert!(
            total_events > 0,
            "no CD events received in 5s; check Wireshark for GVSP packets on UDP/10040"
        );
    }
}
