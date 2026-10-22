use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use abi_stable::rvec;
use abi_stable::std_types::{ROption, RResult, RSlice, RVec};
use abi_stable::type_level::downcasting::TD_Opaque;
use openevt_abi::discovery::PluginRegistry;
use openevt_abi::plugin::{
    EventBatchSink, EventBatchSink_TO, PluginConfiguration, PluginConfigurationValue,
    PluginFacilityHandle, PluginFacilityType, PluginRawEventStreamDecoderFacility,
    PluginRawEventStreamDecoderFacility_TO, PluginRawEventStreamFacility,
    PluginRawEventStreamFacility_TO, PluginStreamBuffer,
};
use openevt_core::hal::facilities::RawEventStreamDecoderFacility;
use openevt_core::types::{EventCD, EventExtTrigger};

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

        // Step 2: open the device with that serial, passing the real sensor
        // geometry. Without it the decoder's coordinate bounds default to
        // 640x480 and silently discard events from the 1280x720 sensor.
        let config_value = |name: &str, value: u16| PluginConfigurationValue {
            name: name.into(),
            value: ROption::RSome(value.to_string().into()),
        };
        let device = registry
            .open_device_with_configuration(PluginConfiguration {
                serial: serial.into(),
                values: rvec![
                    config_value("sensor_width", 1280),
                    config_value("sensor_height", 720),
                ],
            })
            .expect("open device via documented path");

        // Step 3: pull the three relevant facility handles. The host
        // subscribes a sink via EventSubscription, then drives the stream
        // + decoder to convert raw EVT3.0 buffers into typed events.
        let event_subscription = device
            .get_facility_handle(PluginFacilityType::EventSubscription)
            .into_option()
            .expect("no EventSubscription handle");
        let stream_handle = device
            .get_facility_handle(PluginFacilityType::RawEventStream)
            .into_option()
            .expect("no RawEventStream handle");
        let decoder_handle = device
            .get_facility_handle(PluginFacilityType::RawEventStreamDecoder)
            .into_option()
            .expect("no RawEventStreamDecoder handle");

        let PluginFacilityHandle::EventSubscription(mut event_subscription) = event_subscription
        else {
            panic!("EventSubscription facility returned wrong variant");
        };
        let PluginFacilityHandle::RawEventStream(mut stream) = stream_handle else {
            panic!("RawEventStream facility returned wrong variant");
        };
        let PluginFacilityHandle::RawEventStreamDecoder(mut decoder) = decoder_handle else {
            panic!("RawEventStreamDecoder facility returned wrong variant");
        };

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
        match event_subscription.subscribe_to_cd_events(sink) {
            RResult::ROk(()) => {}
            RResult::RErr(error) => panic!("subscribe_to_cd_events failed: {error}"),
        }

        assert!(matches!(stream.start(), RResult::ROk(())));

        let buffers = Arc::new(AtomicUsize::new(0));
        let bytes = Arc::new(AtomicUsize::new(0));

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let PluginStreamBuffer { data, valid_len } = match stream.wait_next_buffer() {
                RResult::ROk(buffer) => buffer,
                RResult::RErr(error) => panic!("raw stream failed: {error}"),
            };
            buffers.fetch_add(1, Ordering::Relaxed);
            bytes.fetch_add(valid_len as usize, Ordering::Relaxed);
            if valid_len == 0 {
                continue;
            }
            let payload = &data[..valid_len as usize];
            match decoder.decode(RSlice::from_slice(payload)) {
                RResult::ROk(()) => {}
                RResult::RErr(error) => panic!("decoder failed: {error}"),
            }
            if events.load(Ordering::Relaxed) > 0 {
                break;
            }
        }

        let total_buffers = buffers.load(Ordering::Relaxed);
        let total_bytes = bytes.load(Ordering::Relaxed);
        let total_events = events.load(Ordering::Relaxed);
        let total_batches = batches.load(Ordering::Relaxed);
        let last_size = last_batch_size.load(Ordering::Relaxed);
        let last_timestamp = decoder.get_last_timestamp();
        println!(
            "Collected {total_buffers} raw buffers / {total_bytes} bytes / {total_events} events across {total_batches} batches (last batch: {last_size} events, decoder last timestamp = {last_timestamp})"
        );

        assert!(
            total_events > 0,
            "no CD events received in 5s; check Wireshark for GVSP packets on UDP/10040"
        );
    }
}
