use abi_stable::std_types::{ROption, RResult, RSlice, RString, RVec};
use abi_stable::type_level::downcasting::TD_Opaque;
use crossbeam::channel::{Receiver, Sender, TryRecvError, bounded};
use openevt::hal::decoders::evt3::Evt3Decoder;
use openevt::hal::device::discovery::ConnectionType;
use openevt::hal::device::plugin::{
    self as plugin, DevicePlugin, EventBatchSinkBox, PluginEventSubscriptionFacility,
    PluginEventSubscriptionFacility_TO, PluginFacilityHandle, PluginFacilityType, PluginGeometry,
    PluginGeometryFacility, PluginGeometryFacility_TO, PluginIndexFacility, PluginIndexFacility_TO,
};
use openevt::hal::facilities::RawEventStreamDecoderFacility;
use openevt::hal::types::{EventCD, EventTimestamp};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use viva_genicam::evs::EvsFormat;
use viva_genicam::gige::nic::Iface;
use viva_genicam::stream::{FrameStream, StreamBlock, StreamBuilder};

const DEFAULT_WIDTH: u32 = 640;
const DEFAULT_HEIGHT: u32 = 480;
const CHANNEL_CAPACITY: usize = 16;

struct BridgeState {
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for BridgeState {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub struct LucidEvsDevice {
    serial: RString,
    width: u32,
    height: u32,
    receiver: Receiver<Vec<EventCD>>,
    bridge: Arc<Mutex<BridgeState>>,
    sink: Arc<Mutex<Option<EventBatchSinkBox>>>,
    timing: Arc<Mutex<DecoderTiming>>,
}

#[derive(Default)]
struct DecoderTiming {
    first: Option<EventTimestamp>,
    last: Option<EventTimestamp>,
}

struct GeometryFacility {
    width: u32,
    height: u32,
}

struct IndexFacility {
    timing: Arc<Mutex<DecoderTiming>>,
}

struct EventSubscriptionFacility {
    sink: Arc<Mutex<Option<EventBatchSinkBox>>>,
}

impl LucidEvsDevice {
    pub fn open(
        ip: Ipv4Addr,
        frame_timeout: Duration,
        destination_port: u16,
    ) -> Result<Self, String> {
        let (sender, receiver) = bounded(CHANNEL_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let timing = Arc::new(Mutex::new(DecoderTiming::default()));
        let worker_stop = Arc::clone(&stop);
        let worker_error = Arc::clone(&error);
        let worker = thread::Builder::new()
            .name("openevt-lucid-evs".into())
            .spawn(move || {
                if let Err(error_message) =
                    run_camera(ip, frame_timeout, destination_port, sender, worker_stop)
                {
                    if let Ok(mut error) = worker_error.lock() {
                        *error = Some(error_message);
                    }
                }
            })
            .map_err(|error| error.to_string())?;

        Ok(Self {
            serial: ip.to_string().into(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            receiver,
            bridge: Arc::new(Mutex::new(BridgeState {
                stop,
                error,
                worker: Some(worker),
            })),
            sink: Arc::new(Mutex::new(None)),
            timing,
        })
    }

    fn bridge_error(&self) -> Option<String> {
        self.bridge
            .lock()
            .ok()
            .and_then(|bridge| bridge.error.lock().ok()?.clone())
    }

    fn deliver(&mut self, events: Vec<EventCD>) {
        if let Ok(mut timing) = self.timing.lock() {
            if let Some(first) = events.first() {
                timing.first.get_or_insert(first.t);
            }
            if let Some(last) = events.last() {
                timing.last = Some(last.t);
            }
        }
        if let Ok(sink) = self.sink.lock()
            && let Some(sink) = sink.as_ref()
        {
            sink.on_cd_events(RSlice::from_slice(&events));
        }
    }
}

/// Long-lived Tokio runtime shared by every camera worker spawned by the
/// plugin. `viva_genicam::GigeRegisterIo` requires a multi-threaded runtime
/// because it uses `tokio::task::block_in_place` for register reads, and a
/// runtime dropped mid-`next_block` would panic the time driver. Holding one
/// runtime per process sidesteps both issues.
fn shared_runtime() -> Result<Arc<tokio::runtime::Runtime>, String> {
    static RUNTIME: std::sync::OnceLock<Result<Arc<tokio::runtime::Runtime>, String>> =
        std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_io()
                .enable_time()
                .build()
                .map(Arc::new)
                .map_err(|error| error.to_string())
        })
        .clone()
}

fn run_camera(
    ip: Ipv4Addr,
    frame_timeout: Duration,
    destination_port: u16,
    sender: Sender<Vec<EventCD>>,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let runtime = shared_runtime()?;
    runtime.block_on(async move {
        let device = viva_genicam::gige::DeviceInfo::from_ip(ip);
        let mut camera = viva_genicam::connect_gige(&device)
            .await
            .map_err(|error| error.to_string())?;
        configure_camera_for_evt3(&mut camera)?;
        bump_payload_size(&mut camera);
        let width = camera
            .get("Width")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(DEFAULT_WIDTH);
        let height = camera
            .get("Height")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(DEFAULT_HEIGHT);
        let iface = Iface::from_remote_ipv4(ip).map_err(|error| error.to_string())?;
        let stream = {
            let mut raw_device = camera
                .transport()
                .lock_device()
                .map_err(|error| error.to_string())?;
            StreamBuilder::new(&mut raw_device)
                .iface(iface)
                .destination_port(destination_port)
                .auto_packet_size()
                .build()
                .await
                .map_err(|error| error.to_string())?
        };
        let mut frame_stream = FrameStream::new(stream, None);
        if let Err(error) = camera.set("TLParamsLocked", "1") {
            tracing::warn!(error = %error, "failed to lock transport-layer parameters");
        }
        camera
            .acquisition_start()
            .map_err(|error| error.to_string())?;
        let result = run_stream(
            &mut frame_stream,
            &mut camera,
            frame_timeout,
            sender,
            stop.clone(),
            width,
            height,
        )
        .await;
        let _ = camera.acquisition_stop();
        result
    })
}

fn configure_camera_for_evt3(
    camera: &mut viva_genicam::Camera<viva_genicam::GigeRegisterIo>,
) -> Result<(), String> {
    // LUCID's EVT cameras expose a read-only `EventFormat` node and the EVT
    // version is selected on the GVSP leader's pixel format code, not via
    // `PixelFormat`. The only writable control here is the acquisition mode.
    let _ = camera.acquisition_stop();

    if let Err(error) = camera.set("AcquisitionMode", "Continuous") {
        tracing::warn!(error = %error, "failed to set AcquisitionMode=Continuous");
    }

    match camera.get("EventFormat") {
        Ok(value) => eprintln!("[openevt-lucid-evs] EventFormat = {value}"),
        Err(error) => eprintln!("[openevt-lucid-evs] EventFormat not readable: {error}"),
    }

    Ok(())
}

fn bump_payload_size(camera: &mut viva_genicam::Camera<viva_genicam::GigeRegisterIo>) {
    // The Triton EVT firmware rejects writes to `PayloadSize` and to
    // `GevSCPSPacketSize > 16000` (the latter is a 16-bit field with the
    // SDK's documented max). Set the packet size to the largest value the
    // camera will accept so each GVSP block carries the most events
    // possible. Anything above this is the camera's firmware choice.
    let candidates: [(&str, u32); 3] = [
        ("GevSCPSPacketSize", 16000),
        ("GevSCPSPacketSize", 8192),
        ("GevSCPSPacketSize", 1500),
    ];
    for (node, desired) in candidates {
        match camera.set(node, &desired.to_string()) {
            Ok(()) => {
                eprintln!("[openevt-lucid-evs] {node} = {desired}");
                return;
            }
            Err(error) => {
                eprintln!("[openevt-lucid-evs] {node}={desired} rejected: {error}");
            }
        }
    }
}

async fn run_stream(
    frame_stream: &mut FrameStream,
    _camera: &mut viva_genicam::Camera<viva_genicam::GigeRegisterIo>,
    frame_timeout: Duration,
    sender: Sender<Vec<EventCD>>,
    stop: Arc<AtomicBool>,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let mut decoder = Evt3Decoder::new(width as u16, height as u16, true);
    let sender_for_decoder = sender.clone();
    decoder.evt_dispatcher.subscribe_cd(Box::new(move |events| {
        let _ = sender_for_decoder.try_send(events.to_vec());
    }));
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        // The `frame_timeout` is a soft deadline: it bounds how long we wait
        // for the *first* block, but ongoing blocks should arrive as fast as
        // the camera can deliver them. Wrapping every `next_block` call in a
        // timeout caps throughput at one block per `frame_timeout` and starves
        // the decoder, which is why 17 events arrived in 5 s.
        let next = frame_stream.next_block();
        tokio::pin!(next);
        let block = tokio::select! {
            biased;
            _ = wait_for_stop(stop.clone()) => return Ok(()),
            result = &mut next => match result {
                Ok(Some(block)) => block,
                Ok(None) => return Ok(()),
                Err(error) => return Err(error.to_string()),
            },
        };
        let block = block;
        match StreamBlock::from(block) {
            StreamBlock::Evs(evs) if evs.format == EvsFormat::Evt30 => {
                decoder
                    .decode(&evs.payload)
                    .map_err(|error| error.to_string())?;
            }
            StreamBlock::Evs(_) => tracing::warn!("received unsupported EVT2.1 payload"),
            StreamBlock::Image(_) => tracing::warn!("skipping image payload from EVS camera"),
            _ => tracing::warn!("skipping unsupported GVSP payload"),
        }
    }
}

async fn wait_for_stop(stop: Arc<AtomicBool>) {
    // Poll the stop flag from inside the runtime so `tokio::select!` can
    // observe a teardown request without needing to abort the in-flight
    // `next_block` future (which would panic on a runtime that's shutting
    // down).
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

impl PluginGeometryFacility for GeometryFacility {
    fn get_width(&self) -> u32 {
        self.width
    }

    fn get_height(&self) -> u32 {
        self.height
    }
}

impl PluginIndexFacility for IndexFacility {
    fn t_min(&self) -> ROption<EventTimestamp> {
        self.timing
            .lock()
            .ok()
            .and_then(|timing| timing.first)
            .into()
    }

    fn t_max(&self) -> ROption<EventTimestamp> {
        self.timing
            .lock()
            .ok()
            .and_then(|timing| timing.last)
            .into()
    }
}

impl PluginEventSubscriptionFacility for EventSubscriptionFacility {
    fn subscribe_to_cd_events(&mut self, sink: EventBatchSinkBox) -> RResult<(), RString> {
        self.sink
            .lock()
            .map_err(|_| "event sink lock poisoned".to_owned())
            .map(|mut stored| {
                *stored = Some(sink);
            })
            .map_or_else(|error| RResult::RErr(error.into()), |_| RResult::ROk(()))
    }

    fn subscribe_to_ext_events(&mut self, _sink: EventBatchSinkBox) -> RResult<(), RString> {
        RResult::RErr("external triggers are not supported by this plugin".into())
    }
}

impl DevicePlugin for LucidEvsDevice {
    fn serial(&self) -> RString {
        self.serial.clone()
    }

    fn connection_type(&self) -> ConnectionType {
        ConnectionType::Network
    }

    fn geometry(&self) -> PluginGeometry {
        PluginGeometry {
            width: self.width,
            height: self.height,
        }
    }

    fn get_facilities(&self) -> RVec<PluginFacilityType> {
        vec![
            PluginFacilityType::Geometry,
            PluginFacilityType::Index,
            PluginFacilityType::EventSubscription,
        ]
        .into()
    }

    fn get_facility(&self, facility_type: PluginFacilityType) -> ROption<plugin::PluginFacility> {
        if self.get_facilities().contains(&facility_type) {
            Some(plugin::PluginFacility::new(facility_type)).into()
        } else {
            ROption::RNone
        }
    }

    fn get_facility_handle(
        &self,
        facility_type: PluginFacilityType,
    ) -> ROption<PluginFacilityHandle> {
        match facility_type {
            PluginFacilityType::Geometry => Some(PluginFacilityHandle::Geometry(
                PluginGeometryFacility_TO::from_value(
                    GeometryFacility {
                        width: self.width,
                        height: self.height,
                    },
                    TD_Opaque,
                ),
            ))
            .into(),
            PluginFacilityType::Index => Some(PluginFacilityHandle::Index(
                PluginIndexFacility_TO::from_value(
                    IndexFacility {
                        timing: Arc::clone(&self.timing),
                    },
                    TD_Opaque,
                ),
            ))
            .into(),
            PluginFacilityType::EventSubscription => Some(PluginFacilityHandle::EventSubscription(
                PluginEventSubscriptionFacility_TO::from_value(
                    EventSubscriptionFacility {
                        sink: Arc::clone(&self.sink),
                    },
                    TD_Opaque,
                ),
            ))
            .into(),
            _ => ROption::RNone,
        }
    }

    fn start_events(&mut self, sink: EventBatchSinkBox) -> RResult<(), RString> {
        match self.sink.lock() {
            Ok(mut stored) => {
                *stored = Some(sink);
                RResult::ROk(())
            }
            Err(_) => RResult::RErr("event sink lock poisoned".into()),
        }
    }

    fn start_external_triggers(&mut self, _sink: EventBatchSinkBox) -> RResult<(), RString> {
        RResult::RErr("external triggers are not supported by this plugin".into())
    }

    fn load_batch(&mut self) -> RResult<(), RString> {
        loop {
            match self.receiver.try_recv() {
                Ok(events) => self.deliver(events),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        if let Some(error) = self.bridge_error() {
            return RResult::RErr(error.into());
        }
        RResult::ROk(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_metadata_matches_network_camera() {
        let device = LucidEvsDevice::open(
            "192.0.2.10".parse().unwrap(),
            Duration::from_millis(1),
            10040,
        )
        .unwrap();
        assert_eq!(device.serial.as_str(), "192.0.2.10");
        assert_eq!(device.connection_type(), ConnectionType::Network);
    }
}
