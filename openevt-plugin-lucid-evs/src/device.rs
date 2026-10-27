use abi_stable::std_types::{ROption, RResult, RSlice, RString, RVec};
use abi_stable::type_level::downcasting::TD_Opaque;
use bytes::Bytes;
use crossbeam::channel::{Receiver, RecvTimeoutError, Sender, bounded};
use openevt_core::hal::decoders::evt3::Evt3Decoder;
use openevt_core::hal::device::discovery::ConnectionType;
use openevt_abi::plugin::{
    self as plugin, DevicePlugin, EventBatchSinkBox, PluginEventSubscriptionFacility,
    PluginEventSubscriptionFacility_TO, PluginFacilityHandle, PluginFacilityType, PluginGeometry,
    PluginGeometryFacility, PluginGeometryFacility_TO, PluginIndexFacility, PluginIndexFacility_TO,
    PluginRawEventStreamDecoderFacility, PluginRawEventStreamDecoderFacility_TO,
    PluginRawEventStreamFacility, PluginRawEventStreamFacility_TO, PluginStreamBuffer,
};
use openevt_core::hal::facilities::RawEventStreamDecoderFacility;
use openevt_core::types::{EventCD, EventTimestamp, FfiKey};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use viva_genicam::gige::Iface;
use viva_genicam::{EvsFormat, FrameStream, StreamBlock, StreamBuilder};

pub(crate) const DEFAULT_WIDTH: u16 = 640;
pub(crate) const DEFAULT_HEIGHT: u16 = 480;
// Worker → host handoff. The host polls at its own cadence and decodes on
// whatever thread calls `poll_buffer`, so the worker must be free to keep
// draining the socket between polls. The camera can emit ~12.5k small event
// blocks per second; a 16-slot channel forced the worker into lockstep with
// the host (1 block per poll), and every other block was lost to the kernel
// once the socket receive buffer filled. A deep queue rides out slow polls;
// the bursty-native decode path reassembles EVT3 timestamp state from
// whatever arrives, so throughput tracks the consumer only when it actually
// keeps up.
const RAW_BUFFER_CAPACITY: usize = 2048;

/// How long `poll_buffer` waits for the first buffer before giving up and
/// reporting an empty poll. Short enough not to stall a render loop that
/// treats "no data yet" as one of its normal states.
const FALLBACK_RECV_TIMEOUT: Duration = Duration::from_millis(5);

/// Keeps the camera worker alive and tears it down deterministically.
///
/// Shared (via `Arc`) between `LucidEvsDevice` and every `RawEventStream`
/// facility handle it hands out. Hosts such as EVT3-Viewer drop the
/// `DevicePluginBox` as soon as they have extracted the facility handles, so
/// the worker must outlive the device as long as a stream handle exists. The
/// last holder to go away runs `Drop`, which raises the worker's `shutdown`
/// flag and joins the thread. The worker keeps its own clone of `shutdown`
/// and only releases it when it exits.
struct BridgeState {
    worker: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl Drop for BridgeState {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub struct LucidEvsDevice {
    serial: RString,
    width: u32,
    height: u32,
    raw_receiver: Receiver<Bytes>,
    /// Shared with `RawEventStreamFacility` handles; see `BridgeState`.
    bridge: Arc<BridgeState>,
    timing: Arc<Mutex<DecoderTiming>>,
    decoder: Arc<Mutex<Evt3Decoder>>,
    pending_events: Arc<Mutex<Vec<EventCD>>>,
    sink: Arc<Mutex<Option<EventBatchSinkBox>>>,
    /// Set by the host via `RawEventStreamFacility::start()`. The worker
    /// blocks on this flag before doing any GVCP / GVSP work, so the host
    /// can wire up the raw_receiver and any subscribers before packets
    /// start arriving.
    should_run: Arc<AtomicBool>,
    bridge_error: Arc<Mutex<Option<String>>>,
    bridge_started: Arc<AtomicBool>,
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

struct RawEventStreamFacility {
    raw_receiver: Receiver<Bytes>,
    /// Keeps the worker alive while the host holds this handle, even after
    /// it has dropped the `DevicePluginBox`.
    _bridge: Arc<BridgeState>,
    started: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    should_run: Arc<AtomicBool>,
}

struct RawEvsDecoderFacility {
    decoder: Arc<Mutex<Evt3Decoder>>,
    timing: Arc<Mutex<DecoderTiming>>,
    pending_events: Arc<Mutex<Vec<EventCD>>>,
    sink: Arc<Mutex<Option<EventBatchSinkBox>>>,
}

impl LucidEvsDevice {
    /// Open a camera session.
    ///
    /// `sensor_width` / `sensor_height` are the actual sensor geometry. They
    /// feed both the reported `PluginGeometry` and the EVT3 decoder's
    /// coordinate bounds: the decoder drops every event whose x/y falls
    /// outside those bounds (and resets on out-of-bounds x), so defaults
    /// smaller than the real sensor silently discard most of the events.
    ///
    /// `packet_size` is the GVSP packet size ceiling in bytes. `None` probes
    /// the largest size the network path supports on connection (from the
    /// host NIC MTU); `Some(size)` uses that ceiling instead. Both paths
    /// still verify the size against the path before streaming.
    pub fn open(
        ip: Ipv4Addr,
        frame_timeout: Duration,
        destination_port: u16,
        packet_size: Option<u32>,
        sensor_width: u16,
        sensor_height: u16,
    ) -> Result<Self, String> {
        let (raw_sender, raw_receiver) = bounded(RAW_BUFFER_CAPACITY);
        let shutdown = Arc::new(AtomicBool::new(false));
        let should_run = Arc::new(AtomicBool::new(false));
        let bridge_started = Arc::new(AtomicBool::new(false));
        let bridge_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let timing = Arc::new(Mutex::new(DecoderTiming::default()));
        let pending_events: Arc<Mutex<Vec<EventCD>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: Arc<Mutex<Option<EventBatchSinkBox>>> = Arc::new(Mutex::new(None));
        let decoder = Arc::new(Mutex::new(Evt3Decoder::new(
            sensor_width,
            sensor_height,
            true,
        )));

        // Install a long-lived subscription that harvests decoded EventCD
        // batches into a shared buffer. The decoder facility drains this
        // buffer inside `decode()` and forwards the events to the registered
        // sink, if any.
        let pending_for_subscription = Arc::clone(&pending_events);
        decoder
            .lock()
            .map_err(|error| error.to_string())?
            .evt_dispatcher
            .subscribe_cd(Box::new(move |events| {
                if let Ok(mut pending) = pending_for_subscription.lock() {
                    pending.extend_from_slice(events);
                }
            }));

        let worker_shutdown = Arc::clone(&shutdown);
        let worker_should_run = Arc::clone(&should_run);
        let worker_started = Arc::clone(&bridge_started);
        let worker_error = Arc::clone(&bridge_error);
        let worker = thread::Builder::new()
            .name("openevt-lucid-evs".into())
            .spawn(move || {
                if let Err(error_message) = run_camera(
                    ip,
                    frame_timeout,
                    destination_port,
                    packet_size,
                    raw_sender,
                    worker_should_run,
                    worker_started,
                    worker_shutdown,
                ) {
                    if let Ok(mut error) = worker_error.lock() {
                        *error = Some(error_message);
                    }
                }
            })
            .map_err(|error| error.to_string())?;

        Ok(Self {
            serial: ip.to_string().into(),
            width: u32::from(sensor_width),
            height: u32::from(sensor_height),
            raw_receiver,
            bridge: Arc::new(BridgeState {
                worker: Some(worker),
                shutdown,
            }),
            timing,
            decoder,
            pending_events,
            sink,
            should_run,
            bridge_error,
            bridge_started,
        })
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

/// Top-level camera lifecycle, gated by the `should_run` flag.
///
/// The worker waits for `should_run` to flip true (driven by
/// `RawEventStreamFacility::start`), then connects, negotiates the stream,
/// starts acquisition, and pumps GVSP blocks into `raw_sender` until either:
///   * the host clears `should_run` (calling `RawEventStreamFacility::stop`),
///     in which case the worker loops and waits for the next `start`,
///   * the GVSP stream completes naturally (`next_block` returns `Ok(None)`),
///   * `next_block` returns an error, or
///   * `shutdown` is raised by `BridgeState::drop` (last device / stream
///     handle gone), in which case the worker exits and is joined.
///
/// The worker owns `shutdown`: the flag is only released when this function
/// returns and the `Arc` clone goes out of scope.
fn run_camera(
    ip: Ipv4Addr,
    _frame_timeout: Duration,
    destination_port: u16,
    packet_size: Option<u32>,
    raw_sender: Sender<Bytes>,
    should_run: Arc<AtomicBool>,
    started: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), String> {
    let runtime = shared_runtime()?;
    runtime.block_on(async move {
        loop {
            if shutdown.load(Ordering::Acquire) {
                eprintln!("[openevt-lucid-evs] worker: shutdown flag set, exiting");
                return Ok(());
            }
            if !should_run.load(Ordering::Acquire) {
                if wait_for_flag(should_run.clone(), shutdown.clone()).await {
                    return Ok(());
                }
                continue;
            }

            eprintln!("[openevt-lucid-evs] worker: should_run=true, connecting");
            let result = run_one_session(
                ip,
                destination_port,
                packet_size,
                raw_sender.clone(),
                started.clone(),
                should_run.clone(),
                shutdown.clone(),
            )
            .await;
            match result {
                Ok(SessionOutcome::HostStopped) => {
                    eprintln!("[openevt-lucid-evs] worker: host stopped stream, looping");
                }
                Ok(SessionOutcome::StreamEnded) => {
                    eprintln!("[openevt-lucid-evs] worker: stream ended naturally, looping");
                }
                Ok(SessionOutcome::Shutdown) => {
                    eprintln!("[openevt-lucid-evs] worker: shutdown flag set, exiting");
                    return Ok(());
                }
                Err(error) => {
                    eprintln!("[openevt-lucid-evs] worker: session error: {error}, exiting");
                    started.store(false, Ordering::Release);
                    return Err(error);
                }
            }
        }
    })
}

#[derive(Debug)]
enum SessionOutcome {
    /// `should_run` was cleared by the host; loop and wait for a new start.
    HostStopped,
    /// `next_block` returned `Ok(None)`; loop and wait for the host to
    /// decide whether to reconnect.
    StreamEnded,
    /// `shutdown` was set; exit the worker entirely.
    Shutdown,
}

async fn run_one_session(
    ip: Ipv4Addr,
    destination_port: u16,
    packet_size: Option<u32>,
    raw_sender: Sender<Bytes>,
    started: Arc<AtomicBool>,
    should_run: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
) -> Result<SessionOutcome, String> {
    let device = viva_genicam::gige::DeviceInfo::from_ip(ip);
    let mut camera = viva_genicam::connect_gige(&device)
        .await
        .map_err(|error| error.to_string())?;
    configure_camera_for_evt3(&mut camera)?;
    let width = camera
        .get("Width")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(DEFAULT_WIDTH as u32);
    let height = camera
        .get("Height")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(DEFAULT_HEIGHT as u32);
    let iface = Iface::from_remote_ipv4(ip).map_err(|error| error.to_string())?;
    let stream = {
        let mut raw_device = camera
            .transport()
            .lock_device()
            .map_err(|error| error.to_string())?;
        // Packet sizing is handled entirely by the stream builder: `None`
        // probes the largest workable size from the NIC MTU down, `Some(size)`
        // negotiates that explicit ceiling instead. Both modes path-probe, so
        // an unreachable size is detected before acquisition starts.
        let mut builder = StreamBuilder::new(&mut raw_device)
            .iface(iface)
            .destination_port(destination_port);
        builder = match packet_size {
            Some(size) => builder.packet_size(size),
            None => builder.auto_packet_size(),
        };
        builder
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
    started.store(true, Ordering::Release);
    eprintln!("[openevt-lucid-evs] worker: acquisition started, entering run_stream");

    let outcome = run_stream(
        &mut frame_stream,
        raw_sender,
        should_run.clone(),
        shutdown.clone(),
        width,
        height,
    )
    .await;

    started.store(false, Ordering::Release);
    let _ = camera.acquisition_stop();
    eprintln!("[openevt-lucid-evs] worker: run_stream finished, outcome={outcome:?}");
    outcome
}

fn configure_camera_for_evt3(
    camera: &mut viva_genicam::Camera<viva_genicam::GigeRegisterIo>,
) -> Result<(), String> {
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

async fn run_stream(
    frame_stream: &mut FrameStream,
    raw_sender: Sender<Bytes>,
    should_run: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    width: u32,
    height: u32,
) -> Result<SessionOutcome, String> {
    tracing::info!(
        width,
        height,
        "camera reported sensor geometry (decoder bounds must match)"
    );
    // No decoding on the worker: the host pulls raw EVT3.0 buffers through
    // the `RawEventStreamFacility` and decodes them through the shared
    // `Evt3Decoder` on its own. We just move ownership of the payload into
    // the channel (no clone).
    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(SessionOutcome::Shutdown);
        }
        if !should_run.load(Ordering::Acquire) {
            return Ok(SessionOutcome::HostStopped);
        }
        let next = frame_stream.next_block();
        tokio::pin!(next);
        let block = tokio::select! {
            biased;
            _ = wait_for_set(shutdown.clone()) => return Ok(SessionOutcome::Shutdown),
            _ = wait_for_clear(should_run.clone()) => return Ok(SessionOutcome::HostStopped),
            result = &mut next => match result {
                Ok(Some(block)) => block,
                Ok(None) => return Ok(SessionOutcome::StreamEnded),
                Err(error) => return Err(error.to_string()),
            },
        };
        match StreamBlock::from(block) {
            StreamBlock::Evs(evs) if evs.format == EvsFormat::Evt30 => {
                // The channel is deep enough that a full slot here means the
                // host has stalled for seconds, not a slow poll. `send` blocks
                // (applying backpressure up to the socket, where the kernel
                // drops datagrams) rather than throwing data away silently;
                // the flag checks above still win because they run before the
                // send is attempted.
                if let Err(error) = raw_sender.send(evs.payload) {
                    tracing::warn!("raw stream receiver went away, stopping: {error}");
                    return Ok(SessionOutcome::HostStopped);
                }
            }
            StreamBlock::Evs(_) => tracing::warn!("received unsupported EVT2.1 payload"),
            StreamBlock::Image(_) => tracing::warn!("skipping image payload from EVS camera"),
            _ => tracing::warn!("skipping unsupported GVSP payload"),
        }
    }
}

/// Polls a flag every 50 ms. Returns true if `shutdown` was set (caller
/// should exit), false if the watched flag was observed true.
async fn wait_for_flag(flag: Arc<AtomicBool>, shutdown: Arc<AtomicBool>) -> bool {
    loop {
        if shutdown.load(Ordering::Acquire) {
            return true;
        }
        if flag.load(Ordering::Acquire) {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_set(flag: Arc<AtomicBool>) {
    while !flag.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_clear(flag: Arc<AtomicBool>) {
    while flag.load(Ordering::Acquire) {
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
        match self.sink.lock() {
            Ok(mut stored) => {
                *stored = Some(sink);
                RResult::ROk(())
            }
            Err(_) => RResult::RErr("event sink lock poisoned".into()),
        }
    }

    fn subscribe_to_ext_events(&mut self, _sink: EventBatchSinkBox) -> RResult<(), RString> {
        RResult::RErr("external triggers are not supported by this plugin".into())
    }
}

impl PluginRawEventStreamFacility for RawEventStreamFacility {
    fn start(&mut self) -> RResult<(), RString> {
        self.should_run.store(true, Ordering::Release);
        RResult::ROk(())
    }

    fn stop(&mut self) -> RResult<(), RString> {
        // "Host wants to stop" is a separate flag from the worker-owned
        // `shutdown`: the worker leaves the current session (acquisition_stop,
        // then dropping the FrameStream closes the UDP socket) and waits for
        // the next `start`. Only dropping the last device / stream handle
        // (`BridgeState::drop`) terminates the worker.
        self.should_run.store(false, Ordering::Release);
        RResult::ROk(())
    }

    fn poll_buffer(&mut self) -> RResult<PluginStreamBuffer, RString> {
        // Block briefly instead of returning empty on the first try: hosts
        // poll at render cadence (~30 Hz), far slower than the camera fills
        // this channel. Draining as many buffers as are queued per call keeps
        // the camera-side queue from backing up behind the poll rate.
        let mut buffers = Vec::new();
        match self.raw_receiver.recv_timeout(FALLBACK_RECV_TIMEOUT) {
            Ok(bytes) => buffers.push(bytes),
            Err(RecvTimeoutError::Timeout) => {
                return RResult::ROk(PluginStreamBuffer {
                    data: RVec::new(),
                    valid_len: 0,
                });
            }
            Err(RecvTimeoutError::Disconnected) => return self.stream_closed_error(),
        }
        while let Ok(bytes) = self.raw_receiver.try_recv() {
            buffers.push(bytes);
        }
        let valid_len = buffers.iter().map(Bytes::len).sum::<usize>() as u64;
        let mut data = Vec::with_capacity(valid_len as usize);
        for bytes in buffers {
            data.extend_from_slice(&bytes);
        }
        RResult::ROk(PluginStreamBuffer {
            data: data.into(),
            valid_len,
        })
    }

    fn wait_next_buffer(&mut self) -> RResult<PluginStreamBuffer, RString> {
        let buffer = match self.raw_receiver.recv() {
            Ok(bytes) => PluginStreamBuffer {
                data: bytes.to_vec().into(),
                valid_len: bytes.len() as u64,
            },
            Err(_) => return self.stream_closed_error(),
        };
        RResult::ROk(buffer)
    }
}

impl RawEventStreamFacility {
    fn stream_closed_error(&self) -> RResult<PluginStreamBuffer, RString> {
        if let Ok(error) = self.error.lock() {
            if let Some(message) = error.clone() {
                return RResult::RErr(message.into());
            }
        }
        RResult::RErr("raw stream disconnected".into())
    }
}

impl PluginRawEventStreamDecoderFacility for RawEvsDecoderFacility {
    fn subscribe_to_protocol_violation(
        &mut self,
        _sink: plugin::PluginDecoderErrorSinkBox,
    ) -> RResult<(), RString> {
        RResult::ROk(())
    }

    fn get_raw_event_size_bytes(&self) -> u8 {
        2
    }

    fn decode(&mut self, raw_data: RSlice<'_, u8>) -> RResult<(), RString> {
        // 1. Run the decoder on the raw bytes.
        let decode_result = self
            .decoder
            .lock()
            .map_err(|_| "decoder lock poisoned".to_owned())
            .and_then(|mut decoder| {
                decoder
                    .decode(raw_data.as_slice())
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = decode_result {
            return RResult::RErr(error.into());
        }

        // 2. Refresh cached t_min/t_max from the decoder.
        if let Ok(mut timing) = self.timing.lock() {
            if let Ok(decoder) = self.decoder.lock() {
                let last = decoder.get_last_timestamp();
                if last != 0 {
                    timing.first.get_or_insert(last);
                    timing.last = Some(last);
                }
            }
        }

        // 3. Drain the events that the long-lived subscription harvested
        //    during step 1 and forward them to the registered sink, if any.
        let pending = {
            let mut pending = match self.pending_events.lock() {
                Ok(pending) => pending,
                Err(_) => return RResult::RErr("pending events lock poisoned".into()),
            };
            std::mem::take(&mut *pending)
        };
        if !pending.is_empty() {
            if let Ok(sink) = self.sink.lock() {
                if let Some(sink) = sink.as_ref() {
                    sink.on_cd_events(RSlice::from_slice(&pending));
                }
            }
        }
        RResult::ROk(())
    }

    fn get_last_timestamp(&self) -> EventTimestamp {
        self.decoder
            .lock()
            .map(|decoder| decoder.get_last_timestamp())
            .unwrap_or(0)
    }

    fn get_timestamp_shift(&self) -> ROption<EventTimestamp> {
        match self.decoder.lock() {
            Ok(decoder) => decoder.get_timestamp_shift().into(),
            Err(_) => ROption::RNone,
        }
    }

    fn is_time_shifting_enabled(&self) -> bool {
        self.decoder
            .lock()
            .map(|decoder| decoder.is_time_shifting_enabled())
            .unwrap_or(false)
    }

    fn reset_last_timestamp(&mut self, timestamp: EventTimestamp) {
        if let Ok(mut decoder) = self.decoder.lock() {
            decoder.reset_last_timestamp(timestamp);
        }
    }

    fn reset_timestamp_shift(&mut self, shift: EventTimestamp) {
        if let Ok(mut decoder) = self.decoder.lock() {
            decoder.reset_timestamp_shift(shift);
        }
    }

    fn is_decoded_event_stream_indexable(&self) -> bool {
        true
    }

    fn add_marker(&mut self, timestamp: EventTimestamp) -> FfiKey {
        match self.decoder.lock() {
            Ok(mut decoder) => decoder.add_marker(timestamp).into(),
            Err(_) => FfiKey { data: 0 },
        }
    }

    fn remove_marker(&mut self, key: FfiKey) -> ROption<EventTimestamp> {
        match self.decoder.lock() {
            Ok(mut decoder) => {
                let key: openevt_core::types::SlotmapKey = key.into();
                decoder.remove_marker(key).into()
            }
            Err(_) => ROption::RNone,
        }
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
            PluginFacilityType::RawEventStream,
            PluginFacilityType::RawEventStreamDecoder,
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
            PluginFacilityType::RawEventStream => Some(PluginFacilityHandle::RawEventStream(
                PluginRawEventStreamFacility_TO::from_value(
                    RawEventStreamFacility {
                        raw_receiver: self.raw_receiver.clone(),
                        _bridge: Arc::clone(&self.bridge),
                        started: Arc::clone(&self.bridge_started),
                        error: Arc::clone(&self.bridge_error),
                        should_run: Arc::clone(&self.should_run),
                    },
                    TD_Opaque,
                ),
            ))
            .into(),
            PluginFacilityType::RawEventStreamDecoder => {
                Some(PluginFacilityHandle::RawEventStreamDecoder(
                    PluginRawEventStreamDecoderFacility_TO::from_value(
                        RawEvsDecoderFacility {
                            decoder: Arc::clone(&self.decoder),
                            timing: Arc::clone(&self.timing),
                            pending_events: Arc::clone(&self.pending_events),
                            sink: Arc::clone(&self.sink),
                        },
                        TD_Opaque,
                    ),
                ))
                .into()
            }
            _ => ROption::RNone,
        }
    }

    fn start_events(&mut self, _sink: EventBatchSinkBox) -> RResult<(), RString> {
        RResult::RErr("use the EventSubscription facility to receive decoded events".into())
    }

    fn start_external_triggers(&mut self, _sink: EventBatchSinkBox) -> RResult<(), RString> {
        RResult::RErr("external triggers are not supported by this plugin".into())
    }

    fn load_batch(&mut self) -> RResult<(), RString> {
        RResult::RErr("use the RawEventStream and RawEventStreamDecoder facilities".into())
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
            None,
            1280,
            720,
        )
        .unwrap();
        assert_eq!(device.serial.as_str(), "192.0.2.10");
        assert_eq!(device.connection_type(), ConnectionType::Network);
        assert_eq!(device.width, 1280);
        assert_eq!(device.height, 720);
    }
}
