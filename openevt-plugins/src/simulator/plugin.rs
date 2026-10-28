//! Video-backed event-camera simulator plugin.

use crate::simulator::error::SimError;
use crate::simulator::solver::{EvsParameters, EvsSimulator};
use abi_stable::std_types::{ROption, RResult, RString, RVec};
use abi_stable::type_level::downcasting::TD_Opaque;
use ffmpeg_next::codec;
use ffmpeg_next::format::{self, Pixel, input as ffmpeg_input};
use ffmpeg_next::software::scaling::{context::Context, flag::Flags};
use ffmpeg_next::util::error::{EAGAIN, Error as FfmpegError};
use ffmpeg_next::util::frame::video::Video;
use hotpath::wrap::std::sync::mpsc::{Receiver, SyncSender};
use openevt_abi::configuration::PluginConfigurationSchema;
use openevt_core::hal::device::discovery::ConnectionType;
use openevt_abi::plugin::{
    self, DeviceDiscoveryPlugin, DeviceDiscoveryPlugin_TO, DeviceDiscoveryPluginBox,
    DevicePlugin_TO, DevicePluginBox,
    EventBatchSinkBox, PluginCameraDescriptionAbi, PluginConfiguration,
    PluginEventSubscriptionFacility, PluginEventSubscriptionFacility_TO, PluginFacility,
    PluginFacilityHandle, PluginFacilityType, PluginGeometry, PluginGeometryFacility,
    PluginGeometryFacility_TO, PluginIndexFacility, PluginIndexFacility_TO,
    PluginRawEventStreamFacility, PluginRawEventStreamFacility_TO, PluginSeekFacility,
    PluginSeekFacility_TO, PluginStreamBuffer,
};
use openevt_core::types::{EventCD, EventTimestamp, Polarity};
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const SIMULATOR_SERIAL: &str = "EventSimulator";
// One host request corresponds to one video frame. This lets an empty event
// callback preserve the simulator's frame cadence instead of being mistaken
// for a request that should keep fetching until an event appears.
const FRAME_BATCH_SIZE: usize = 1;
// Keep a small amount of decoded work ahead of the host. The queue is bounded
// so a slow consumer cannot grow memory without limit; two batches provide the
// intended double-buffer behavior while keeping latency predictable.
const PREFETCH_BATCHES: usize = 2;
const RAW_EVENT_SIMULATOR_SCHEMA: &str = r#"
version = 1

[[parameters]]
name = "video_file"
label = "Video file"
kind = "file"
required = true
description = "A video file whose frames will be converted into simulated events."

[[parameters]]
name = "fps"
label = "Frames per second"
kind = "float"
required = false
description = "Optional override for the video's encoded frame rate."

[[parameters]]
name = "width"
label = "Output width"
kind = "integer"
required = false
default = "800"
description = "Output width in pixels. Defaults to 800."

[[parameters]]
name = "height"
label = "Output height"
kind = "integer"
required = false
default = "600"
description = "Output height in pixels. Defaults to 600."

[[parameters]]
name = "preload"
label = "Preload video"
kind = "boolean"
required = false
default = "false"
description = "Decode and simulate the complete video before serving events."

[[parameters]]
name = "config_file"
label = "Simulator configuration"
kind = "file"
required = false
description = "Optional TOML file containing the initial simulator parameters."
"#;

struct SimulatorGeometryFacility {
    width: u32,
    height: u32,
}

pub(crate) struct VideoSimulator {
    input: format::context::Input,
    decoder: codec::decoder::Video,
    scaler: Context,
    video_stream_index: usize,
    width: usize,
    height: usize,
    fps: f64,
    pub(crate) duration_us: EventTimestamp,
    frame_count: u64,
    photocurrent_scale: f32,
    frame_index: u64,
    packet_pending: bool,
    eof_sent: bool,
    rgb: Video,
    photocurrents: Vec<f32>,
    simulator: EvsSimulator,
}

// FFmpeg's Rust wrapper does not mark the scaler context as `Send` because it
// contains an opaque C pointer. The entire simulator is accessed through one
// mutex, so the context is never used concurrently or moved independently of
// the other decoder state.
unsafe impl Send for VideoSimulator {}

#[hotpath::measure_all]
impl VideoSimulator {
    pub(crate) fn open(
        video_path: &str,
        fps_override: Option<f64>,
        params: EvsParameters,
        output_width: usize,
        output_height: usize,
    ) -> Result<Self, String> {
        ffmpeg_next::init().map_err(|error| error.to_string())?;
        params.validate().map_err(|error| error.to_string())?;

        let input = ffmpeg_input(video_path).map_err(|error| error.to_string())?;
        let stream = input
            .streams()
            .best(ffmpeg_next::media::Type::Video)
            .ok_or_else(|| "video file does not contain a video stream".to_owned())?;
        let video_stream_index = stream.index();
        let encoded_fps = f64::from(stream.avg_frame_rate());
        let fps = fps_override.unwrap_or(encoded_fps);
        if !fps.is_finite() || fps <= 0.0 {
            return Err("video frame rate must be finite and positive".to_owned());
        }
        let stream_time_base = stream.time_base();
        let duration_us = if stream.duration() > 0 && stream_time_base.denominator() != 0 {
            (stream.duration() as f64 * f64::from(stream_time_base.numerator())
                / f64::from(stream_time_base.denominator())
                * 1_000_000.0) as EventTimestamp
        } else {
            input.duration().max(0) as EventTimestamp
        };
        let frame_count = if stream.frames() > 0 {
            stream.frames() as u64
        } else {
            (duration_us as f64 * fps / 1_000_000.0).ceil() as u64
        };

        let context_decoder = codec::context::Context::from_parameters(stream.parameters())
            .map_err(|error| error.to_string())?;
        let decoder = context_decoder
            .decoder()
            .video()
            .map_err(|error| error.to_string())?;
        let width = decoder.width() as usize;
        let height = decoder.height() as usize;
        if width == 0 || height == 0 {
            return Err("video stream has zero dimensions".to_owned());
        }
        let scaler = Context::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            Pixel::RGB24,
            output_width as u32,
            output_height as u32,
            Flags::BILINEAR,
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            input,
            decoder,
            scaler,
            video_stream_index,
            width: output_width,
            height: output_height,
            fps,
            duration_us,
            frame_count,
            photocurrent_scale: params.photocurrent_scale,
            frame_index: 0,
            packet_pending: false,
            eof_sent: false,
            rgb: Video::empty(),
            photocurrents: Vec::with_capacity(output_width * output_height),
            simulator: EvsSimulator::new((output_width as u64) * (output_height as u64), params)
                .map_err(|error| error.to_string())?,
        })
    }

    pub(crate) fn seek(&mut self, timestamp_us: EventTimestamp) -> Result<(), String> {
        let target = timestamp_us.min(self.duration_us);
        let target_frame =
            ((target as f64 * self.fps / 1_000_000.0).floor() as u64).min(self.frame_count);

        // Decode forward from the beginning so the frame index and simulator
        // state exactly match the requested video frame. FFmpeg keyframe seeks
        // alone can land before the requested frame and would misalign event
        // timestamps with the decoded content.
        self.input.seek(0, ..).map_err(|error| error.to_string())?;
        self.decoder.flush();
        self.packet_pending = false;
        self.eof_sent = false;
        self.frame_index = 0;
        self.simulator.reset();

        while self.frame_index < target_frame {
            if self.next_frame_events()?.is_none() {
                break;
            }
        }
        Ok(())
    }

    fn next_frame_events(&mut self) -> Result<Option<Vec<EventCD>>, String> {
        loop {
            if self.packet_pending {
                let mut decoded = Video::empty();
                match self.decoder.receive_frame(&mut decoded) {
                    Ok(()) => return self.process_frame(&decoded).map(Some),
                    Err(FfmpegError::Other { errno }) if errno == EAGAIN => {
                        self.packet_pending = false;
                    }
                    Err(FfmpegError::Eof) => return Ok(None),
                    Err(error) => return Err(error.to_string()),
                }
            }

            if self.eof_sent {
                return Ok(None);
            }

            let next_packet = self.input.packets().next();
            let Some((stream, packet)) = next_packet else {
                self.decoder.send_eof().map_err(|error| error.to_string())?;
                self.eof_sent = true;
                self.packet_pending = true;
                continue;
            };
            if stream.index() != self.video_stream_index {
                continue;
            }
            self.decoder
                .send_packet(&packet)
                .map_err(|error| error.to_string())?;
            self.packet_pending = true;
        }
    }

    pub(crate) fn next_events_batch(&mut self) -> Result<Option<Vec<EventCD>>, String> {
        let mut events = Vec::new();
        let mut decoded_frame = false;

        for _ in 0..FRAME_BATCH_SIZE {
            let Some(frame_events) = self.next_frame_events()? else {
                break;
            };
            decoded_frame = true;
            events.extend(frame_events);
        }

        // An empty event batch is still meaningful when frames were decoded:
        // it advances the stream by FRAME_BATCH_SIZE frames (or to EOF).
        Ok(decoded_frame.then_some(events))
    }

    fn preload_events(mut self) -> Result<Vec<Vec<EventCD>>, String> {
        let mut batches = Vec::new();
        while let Some(events) = self.next_events_batch()? {
            batches.push(events);
        }
        Ok(batches)
    }

    fn process_frame(&mut self, decoded: &Video) -> Result<Vec<EventCD>, String> {
        self.scaler
            .run(decoded, &mut self.rgb)
            .map_err(|error| error.to_string())?;

        let stride = self.rgb.stride(0);
        let data = self.rgb.data(0);
        self.photocurrents.clear();
        self.photocurrents.reserve(self.width * self.height);
        for y in 0..self.height {
            let row = &data[y * stride..];
            for x in 0..self.width {
                let offset = x * 3;
                let red = row[offset] as f32;
                let green = row[offset + 1] as f32;
                let blue = row[offset + 2] as f32;
                let luminance = (0.2126 * red + 0.7152 * green + 0.0722 * blue) / 255.0;
                // Keep a dark pixel above zero while preserving the video's
                // relative brightness as the simulated photocurrent.
                self.photocurrents.push(luminance * self.photocurrent_scale);
            }
        }

        let timestamp = (self.frame_index as f64 * 1_000_000.0 / self.fps) as f32;
        self.frame_index += 1;
        let generated = self
            .simulator
            .process_frame_over_interval(&self.photocurrents, timestamp, 1.0 / self.fps as f32)
            .map_err(|error| error.to_string())?;

        Ok(generated
            .into_iter()
            .map(|event| {
                let pixel_index = usize::try_from(event.pixel_index)
                    .expect("simulator emitted a pixel index that does not fit this platform");
                EventCD {
                    x: (pixel_index % self.width) as u64,
                    y: (pixel_index / self.width) as u64,
                    p: Polarity::from(event.polarity),
                    t: event.timestamp.max(0.0) as EventTimestamp,
                }
            })
            .collect())
    }
}

enum WorkerMessage {
    Events(Vec<EventCD>),
    End,
    Error(String),
}

#[derive(Clone)]
struct SimulatorConfig {
    video_path: String,
    fps: Option<f64>,
    params: EvsParameters,
    width: usize,
    height: usize,
}

fn spawn_worker(
    config: &SimulatorConfig,
    seek_timestamp: Option<EventTimestamp>,
) -> Result<
    (
        Receiver<WorkerMessage>,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
        JoinHandle<()>,
    ),
    String,
> {
    let mut simulator = VideoSimulator::open(
        &config.video_path,
        config.fps,
        config.params.clone(),
        config.width,
        config.height,
    )?;
    if let Some(timestamp) = seek_timestamp {
        simulator.seek(timestamp)?;
    }

    let (sender, receiver) = hotpath::channel!(
        sync_channel(PREFETCH_BATCHES),
        label = "prefetch-batches",
        capacity = PREFETCH_BATCHES
    );
    let active = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let worker_active = Arc::clone(&active);
    let worker_stop = Arc::clone(&stop);
    let worker = thread::Builder::new()
        .name("openevt-simulator-prefetch".into())
        .spawn(move || run_prefetch_worker(simulator, sender, worker_active, worker_stop))
        .map_err(|error| error.to_string())?;
    Ok((receiver, active, stop, worker))
}

#[hotpath::measure]
fn run_prefetch_worker(
    mut simulator: VideoSimulator,
    sender: SyncSender<WorkerMessage>,
    active: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        if !active.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(1));
            continue;
        }
        let message = match simulator.next_events_batch() {
            Ok(Some(events)) => WorkerMessage::Events(events),
            Ok(None) => WorkerMessage::End,
            Err(error) => WorkerMessage::Error(error),
        };
        if sender.send(message).is_err() || stop.load(Ordering::Acquire) {
            break;
        }
    }
}

#[hotpath::measure]
fn run_preloaded_worker(
    batches: Vec<Vec<EventCD>>,
    sender: SyncSender<WorkerMessage>,
    active: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    while !active.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(1));
    }
    if stop.load(Ordering::Acquire) {
        return;
    }
    for events in batches {
        if sender.send(WorkerMessage::Events(events)).is_err() || stop.load(Ordering::Acquire) {
            return;
        }
    }
    let _ = sender.send(WorkerMessage::End);
}

struct SimulatorState {
    started: bool,
    config: SimulatorConfig,
    duration_us: EventTimestamp,
    batches: Option<Receiver<WorkerMessage>>,
    active: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    cd_sinks: Vec<EventBatchSinkBox>,
    ext_sinks: Vec<EventBatchSinkBox>,
}

impl SimulatorState {
    fn seek(&mut self, timestamp: EventTimestamp) -> Result<(), String> {
        let was_started = self.started;
        self.active.store(false, Ordering::Release);
        self.stop.store(true, Ordering::Release);
        self.batches.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }

        let (batches, active, stop, worker) = spawn_worker(&self.config, Some(timestamp))?;
        self.batches = Some(batches);
        self.active = active;
        self.stop = stop;
        self.worker = Some(worker);
        self.active.store(was_started, Ordering::Release);
        Ok(())
    }

    #[hotpath::measure]
    fn advance(state: &Arc<hotpath::wrap::std::sync::Mutex<Self>>) -> Result<(), String> {
        let message = {
            let lock = state
                .lock()
                .map_err(|_| "simulator state lock was poisoned".to_owned())?;
            if !lock.started {
                return Err("simulator stream has not been started".to_owned());
            }
            let receiver = lock
                .batches
                .as_ref()
                .ok_or_else(|| "simulator worker is not available".to_owned())?;
            loop {
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(message) => break message,
                    Err(RecvTimeoutError::Timeout) => {
                        if lock.stop.load(Ordering::Acquire) {
                            return Err("simulator worker stopped".to_owned());
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err("simulator worker disconnected".to_owned());
                    }
                }
            }
        };

        let events = match message {
            WorkerMessage::Events(events) => events,
            WorkerMessage::End => return Err("end of simulator video".to_owned()),
            WorkerMessage::Error(error) => return Err(error),
        };

        let (mut cd_sinks, mut ext_sinks) = {
            let mut lock = state
                .lock()
                .map_err(|_| "simulator state lock was poisoned".to_owned())?;
            (
                std::mem::take(&mut lock.cd_sinks),
                std::mem::take(&mut lock.ext_sinks),
            )
        };

        for sink in &cd_sinks {
            sink.on_cd_events(events.as_slice().into());
        }
        for sink in &ext_sinks {
            sink.on_ext_events([].as_slice().into());
        }

        let mut lock = state
            .lock()
            .map_err(|_| "simulator state lock was poisoned".to_owned())?;
        lock.cd_sinks.append(&mut cd_sinks);
        lock.ext_sinks.append(&mut ext_sinks);
        Ok(())
    }
}

impl Drop for SimulatorState {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        self.stop.store(true, Ordering::Release);
        // Drop the receiver before joining. This wakes a worker blocked on a
        // full bounded queue instead of waiting for the consumer forever.
        self.batches.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct SimulatorDevice {
    state: Arc<hotpath::wrap::std::sync::Mutex<SimulatorState>>,
    width: u32,
    height: u32,
}

struct SimulatorStreamFacility {
    state: Arc<hotpath::wrap::std::sync::Mutex<SimulatorState>>,
}

struct SimulatorIndexFacility {
    state: Arc<hotpath::wrap::std::sync::Mutex<SimulatorState>>,
}

impl PluginIndexFacility for SimulatorIndexFacility {
    fn t_min(&self) -> ROption<EventTimestamp> {
        Some(0).into()
    }

    fn t_max(&self) -> ROption<EventTimestamp> {
        self.state.lock().ok().map(|state| state.duration_us).into()
    }
}

struct SimulatorSeekFacility {
    state: Arc<hotpath::wrap::std::sync::Mutex<SimulatorState>>,
}

impl PluginSeekFacility for SimulatorSeekFacility {
    fn seek(&mut self, timestamp: EventTimestamp) -> RResult<(), RString> {
        match self.state.lock() {
            Ok(mut state) => match state.seek(timestamp) {
                Ok(()) => RResult::ROk(()),
                Err(error) => RResult::RErr(error.into()),
            },
            Err(_) => RResult::RErr("simulator state lock was poisoned".into()),
        }
    }
}

impl SimulatorStreamFacility {
    fn request(&mut self) -> RResult<PluginStreamBuffer, RString> {
        match SimulatorState::advance(&self.state) {
            Ok(()) => RResult::ROk(PluginStreamBuffer {
                data: Vec::new().into(),
                valid_len: 0,
            }),
            Err(error) => RResult::RErr(error.into()),
        }
    }
}

impl PluginRawEventStreamFacility for SimulatorStreamFacility {
    fn start(&mut self) -> RResult<(), RString> {
        match self.state.lock() {
            Ok(mut state) => {
                state.started = true;
                state.active.store(true, Ordering::Release);
                RResult::ROk(())
            }
            Err(_) => RResult::RErr("simulator state lock was poisoned".into()),
        }
    }

    fn stop(&mut self) -> RResult<(), RString> {
        match self.state.lock() {
            Ok(mut state) => {
                state.started = false;
                state.active.store(false, Ordering::Release);
                RResult::ROk(())
            }
            Err(_) => RResult::RErr("simulator state lock was poisoned".into()),
        }
    }

    fn poll_buffer(&mut self) -> RResult<PluginStreamBuffer, RString> {
        self.request()
    }

    fn wait_next_buffer(&mut self) -> RResult<PluginStreamBuffer, RString> {
        self.request()
    }
}

struct SimulatorEventSubscription {
    state: Arc<hotpath::wrap::std::sync::Mutex<SimulatorState>>,
}

impl PluginEventSubscriptionFacility for SimulatorEventSubscription {
    fn subscribe_to_cd_events(&mut self, sink: EventBatchSinkBox) -> RResult<(), RString> {
        match self.state.lock() {
            Ok(mut state) => {
                state.cd_sinks.push(sink);
                RResult::ROk(())
            }
            Err(_) => RResult::RErr("simulator state lock was poisoned".into()),
        }
    }

    fn subscribe_to_ext_events(&mut self, sink: EventBatchSinkBox) -> RResult<(), RString> {
        match self.state.lock() {
            Ok(mut state) => {
                state.ext_sinks.push(sink);
                RResult::ROk(())
            }
            Err(_) => RResult::RErr("simulator state lock was poisoned".into()),
        }
    }
}

fn configuration_value<'a>(configuration: &'a PluginConfiguration, name: &str) -> Option<&'a str> {
    configuration
        .values
        .iter()
        .find(|value| value.name.as_str() == name)
        .and_then(|value| value.value.as_ref().into_option())
        .map(|value| value.as_str())
}

fn simulator_parameters(configuration: &PluginConfiguration) -> Result<EvsParameters, SimError> {
    let Some(path) = configuration_value(configuration, "config_file") else {
        let parameters = EvsParameters::default();
        parameters
            .validate()
            .map_err(SimError::InvalidConfiguration)?;
        return Ok(parameters);
    };
    let source = fs::read_to_string(path)?;
    let parameters = toml::from_str::<EvsParameters>(&source)?;
    parameters
        .validate()
        .map_err(SimError::InvalidConfiguration)?;
    Ok(parameters)
}

fn positive_configuration_dimension(
    configuration: &PluginConfiguration,
    name: &str,
    default: usize,
) -> Result<usize, SimError> {
    let Some(value) = configuration_value(configuration, name) else {
        return Ok(default);
    };
    let value = value.parse::<u64>().map_err(|_| {
        SimError::InvalidConfiguration(format!("`{name}` must be a positive integer"))
    })?;
    if value == 0 {
        return Err(SimError::InvalidConfiguration(format!(
            "`{name}` must be a positive integer"
        )));
    }
    value.try_into().map_err(|_| {
        SimError::InvalidConfiguration(format!("`{name}` is too large for this platform"))
    })
}

impl SimulatorDevice {
    fn open(configuration: &PluginConfiguration) -> Result<Self, SimError> {
        let video_path = configuration_value(configuration, "video_file")
            .ok_or_else(|| SimError::InvalidConfiguration("`video_file` is required".into()))?;
        let fps = configuration_value(configuration, "fps")
            .map(|value| {
                value
                    .parse::<f64>()
                    .map_err(|_| SimError::InvalidConfiguration("`fps` must be a number".into()))
            })
            .transpose()?;
        let output_width = positive_configuration_dimension(configuration, "width", 800)?;
        let output_height = positive_configuration_dimension(configuration, "height", 600)?;
        let preload = configuration_value(configuration, "preload")
            .map(|value| {
                value.parse::<bool>().map_err(|_| {
                    SimError::InvalidConfiguration("`preload` must be true or false".into())
                })
            })
            .transpose()?
            .unwrap_or(false);
        let parameters = simulator_parameters(configuration)?;
        let config = SimulatorConfig {
            video_path: video_path.to_owned(),
            fps,
            params: parameters.clone(),
            width: output_width,
            height: output_height,
        };
        let simulator =
            VideoSimulator::open(video_path, fps, parameters, output_width, output_height)
                .map_err(SimError::InvalidConfiguration)?;
        let duration_us = simulator.duration_us;
        let output_width = output_width as u32;
        let output_height = output_height as u32;
        let (simulator, preloaded_batches) = if preload {
            let batches = simulator
                .preload_events()
                .map_err(SimError::InvalidConfiguration)?;
            (None, Some(batches))
        } else {
            (Some(simulator), None)
        };
        let width = output_width;
        let height = output_height;
        let (sender, receiver) = hotpath::channel!(
            sync_channel(PREFETCH_BATCHES),
            label = "prefetch-batches",
            capacity = PREFETCH_BATCHES
        );
        let active = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_active = Arc::clone(&active);
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("openevt-simulator-prefetch".into())
            .spawn(move || {
                if let Some(batches) = preloaded_batches {
                    run_preloaded_worker(batches, sender, worker_active, worker_stop);
                } else if let Some(simulator) = simulator {
                    run_prefetch_worker(simulator, sender, worker_active, worker_stop);
                } else {
                    let _ = sender.send(WorkerMessage::Error(
                        "simulator worker was not initialized".into(),
                    ));
                }
            })
            .map_err(SimError::Io)?;
        Ok(Self {
            width,
            height,
            state: Arc::new(hotpath::mutex!(
                Mutex::new(SimulatorState {
                    started: false,
                    config,
                    duration_us,
                    batches: Some(receiver),
                    active,
                    stop,
                    worker: Some(worker),
                    cd_sinks: Vec::new(),
                    ext_sinks: Vec::new(),
                }),
                label = "simulator-state"
            )),
        })
    }
}

impl PluginGeometryFacility for SimulatorGeometryFacility {
    fn get_width(&self) -> u32 {
        self.width
    }
    fn get_height(&self) -> u32 {
        self.height
    }
}

impl plugin::DevicePlugin for SimulatorDevice {
    fn serial(&self) -> RString {
        SIMULATOR_SERIAL.into()
    }

    fn connection_type(&self) -> ConnectionType {
        ConnectionType::Software
    }

    fn geometry(&self) -> PluginGeometry {
        PluginGeometry {
            width: self.width,
            height: self.height,
        }
    }

    fn t_min(&self) -> ROption<EventTimestamp> {
        Some(0).into()
    }

    fn t_max(&self) -> ROption<EventTimestamp> {
        self.state.lock().ok().map(|state| state.duration_us).into()
    }

    fn seek(&mut self, timestamp: EventTimestamp) -> RResult<(), RString> {
        match self.state.lock() {
            Ok(mut state) => match state.seek(timestamp) {
                Ok(()) => RResult::ROk(()),
                Err(error) => RResult::RErr(error.into()),
            },
            Err(_) => RResult::RErr("simulator state lock was poisoned".into()),
        }
    }

    fn get_facilities(&self) -> RVec<PluginFacilityType> {
        vec![
            PluginFacilityType::Geometry,
            PluginFacilityType::Index,
            PluginFacilityType::Seek,
            PluginFacilityType::RawEventStream,
            PluginFacilityType::EventSubscription,
        ]
        .into()
    }

    fn get_facility(&self, facility_type: PluginFacilityType) -> ROption<PluginFacility> {
        if self.get_facilities().contains(&facility_type) {
            Some(PluginFacility { facility_type }).into()
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
                    SimulatorGeometryFacility {
                        width: self.width,
                        height: self.height,
                    },
                    TD_Opaque,
                ),
            ))
            .into(),
            PluginFacilityType::Index => Some(PluginFacilityHandle::Index(
                PluginIndexFacility_TO::from_value(
                    SimulatorIndexFacility {
                        state: Arc::clone(&self.state),
                    },
                    TD_Opaque,
                ),
            ))
            .into(),
            PluginFacilityType::Seek => Some(PluginFacilityHandle::Seek(
                PluginSeekFacility_TO::from_value(
                    SimulatorSeekFacility {
                        state: Arc::clone(&self.state),
                    },
                    TD_Opaque,
                ),
            ))
            .into(),
            PluginFacilityType::RawEventStream => Some(PluginFacilityHandle::RawEventStream(
                PluginRawEventStreamFacility_TO::from_value(
                    SimulatorStreamFacility {
                        state: Arc::clone(&self.state),
                    },
                    TD_Opaque,
                ),
            ))
            .into(),
            PluginFacilityType::EventSubscription => Some(PluginFacilityHandle::EventSubscription(
                PluginEventSubscriptionFacility_TO::from_value(
                    SimulatorEventSubscription {
                        state: Arc::clone(&self.state),
                    },
                    TD_Opaque,
                ),
            ))
            .into(),
            _ => ROption::RNone,
        }
    }

    fn start_events(&mut self, sink: EventBatchSinkBox) -> RResult<(), RString> {
        match self.state.lock() {
            Ok(mut state) => {
                state.started = true;
                state.cd_sinks.push(sink);
                state.active.store(true, Ordering::Release);
                RResult::ROk(())
            }
            Err(_) => RResult::RErr("simulator state lock was poisoned".into()),
        }
    }

    fn start_external_triggers(&mut self, sink: EventBatchSinkBox) -> RResult<(), RString> {
        match self.state.lock() {
            Ok(mut state) => {
                state.started = true;
                state.ext_sinks.push(sink);
                state.active.store(true, Ordering::Release);
                RResult::ROk(())
            }
            Err(_) => RResult::RErr("simulator state lock was poisoned".into()),
        }
    }

    fn load_batch(&mut self) -> RResult<(), RString> {
        match SimulatorState::advance(&self.state) {
            Ok(()) => RResult::ROk(()),
            Err(error) => RResult::RErr(error.into()),
        }
    }
}

/// Discovery interface advertising the software event simulator.
pub struct SimulatorDiscovery;

impl DeviceDiscoveryPlugin for SimulatorDiscovery {
    fn discover(&self) -> RVec<PluginCameraDescriptionAbi> {
        vec![PluginCameraDescriptionAbi {
            serial: SIMULATOR_SERIAL.into(),
            connection: ConnectionType::Software,
        }]
        .into()
    }

    fn configuration_schema(&self) -> RString {
        RAW_EVENT_SIMULATOR_SCHEMA.into()
    }

    fn open_device(
        &self,
        _serial: abi_stable::std_types::RStr<'_>,
    ) -> RResult<DevicePluginBox, RString> {
        RResult::RErr("the simulator requires a configuration object".into())
    }

    fn open_device_with_configuration(
        &self,
        configuration: PluginConfiguration,
    ) -> RResult<DevicePluginBox, RString> {
        let schema = match PluginConfigurationSchema::parse(RAW_EVENT_SIMULATOR_SCHEMA) {
            Ok(schema) => schema,
            Err(error) => return RResult::RErr(error.to_string().into()),
        };
        if let Err(error) = schema.validate(&configuration) {
            return RResult::RErr(error.to_string().into());
        }
        match SimulatorDevice::open(&configuration) {
            Ok(device) => RResult::ROk(DevicePlugin_TO::from_value(device, TD_Opaque)),
            Err(error) => RResult::RErr(error.to_string().into()),
        }
    }
}

/// Name this device is advertised under.
pub const SIMULATOR_PLUGIN_NAME: &str = "openevt_simulator";

/// Builds the simulator discovery interface for the crate root module.
pub extern "C" fn create_simulator_discovery() -> DeviceDiscoveryPluginBox {
    DeviceDiscoveryPlugin_TO::from_value(SimulatorDiscovery, TD_Opaque)
}

#[cfg(test)]
mod tests {
    use super::*;
    use abi_stable::std_types::{RResult, RSlice};
    use openevt_abi::plugin::{DevicePlugin, EventBatchSink, EventBatchSink_TO};
    use openevt_core::types::{EventCD, EventExtTrigger};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    #[test]
    fn schema_requires_a_video_and_allows_optional_overrides() {
        let schema = PluginConfigurationSchema::parse(RAW_EVENT_SIMULATOR_SCHEMA).unwrap();
        let configuration = schema.new_configuration(SIMULATOR_SERIAL);
        assert!(schema.validate(&configuration).is_err());
        assert_eq!(schema.parameters.len(), 6);
        assert!(!schema.parameters[1].required);
        assert!(!schema.parameters[2].required);
        assert_eq!(schema.parameters[2].default.as_deref(), Some("800"));
        assert_eq!(schema.parameters[3].default.as_deref(), Some("600"));
        assert_eq!(schema.parameters[4].default.as_deref(), Some("false"));
    }

    #[test]
    fn discovery_exposes_the_simulator_configuration() {
        let discovery = SimulatorDiscovery;
        assert_eq!(discovery.discover().len(), 1);
        assert!(discovery.configuration_schema().contains("video_file"));
    }

    #[test]
    fn video_index_reports_duration_and_seek_uses_frame_timeline() {
        let video_path = [
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/tests/test.mp4"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test.mp4"),
        ]
        .into_iter()
        .find(|path| path.is_file())
        .expect("test video is missing");
        let mut simulator = VideoSimulator::open(
            video_path.to_str().unwrap(),
            None,
            EvsParameters::default(),
            160,
            90,
        )
        .unwrap();

        let target_timestamp = 3_000_000_u64;
        let expected_frame = (target_timestamp as f64 * simulator.fps / 1_000_000.0).floor() as u64;
        simulator.seek(target_timestamp).unwrap();
        assert_eq!(simulator.frame_index, expected_frame);
        assert!(simulator.duration_us > 45_000_000);
        assert!(simulator.duration_us < 47_000_000);

        simulator.next_events_batch().unwrap();
        assert_eq!(simulator.frame_index, expected_frame + 1);
    }

    struct CollectingSink {
        batches: Arc<Mutex<Vec<Vec<EventCD>>>>,
    }

    impl EventBatchSink for CollectingSink {
        fn on_cd_events(&self, events: RSlice<'_, EventCD>) {
            self.batches.lock().unwrap().push(events.to_vec());
        }

        fn on_ext_events(&self, _events: RSlice<'_, EventExtTrigger>) {}
    }

    #[test]
    fn default_framerate_produces_monotonic_frame_timestamps() {
        let _hotpath = hotpath::HotpathGuardBuilder::new("test.mp4-e2e").build();
        let video_path = [
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/tests/test.mp4"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test.mp4"),
        ]
        .into_iter()
        .find(|path| path.is_file())
        .expect("test video is missing");

        let schema = PluginConfigurationSchema::parse(RAW_EVENT_SIMULATOR_SCHEMA).unwrap();
        let mut configuration = schema.new_configuration(SIMULATOR_SERIAL);
        configuration
            .values
            .iter_mut()
            .find(|value| value.name.as_str() == "video_file")
            .unwrap()
            .value = Some(video_path.to_string_lossy().into()).into();
        let config_path = std::env::temp_dir().join(format!(
            "openevt-simulator-test-{}.toml",
            std::process::id()
        ));
        fs::write(
            &config_path,
            "threshold_on = 0.001\nthreshold_off = 0.001\n",
        )
        .unwrap();
        configuration
            .values
            .iter_mut()
            .find(|value| value.name.as_str() == "config_file")
            .unwrap()
            .value = Some(config_path.to_string_lossy().into()).into();
        // Deliberately leave `fps` unset: the simulator must use the encoded
        // 30000/1001 fps from test.mp4.
        assert!(
            configuration
                .values
                .iter()
                .find(|value| value.name.as_str() == "fps")
                .unwrap()
                .value
                .is_none()
        );

        let mut device = SimulatorDevice::open(&configuration).unwrap();
        assert_eq!(device.geometry().width, 800);
        assert_eq!(device.geometry().height, 600);
        let _ = fs::remove_file(&config_path);
        let batches = Arc::new(Mutex::new(Vec::new()));
        let sink = EventBatchSink_TO::from_value(
            CollectingSink {
                batches: Arc::clone(&batches),
            },
            TD_Opaque,
        );
        assert!(matches!(device.start_events(sink), RResult::ROk(())));

        const FRAMES_TO_CHECK: usize = 12;
        for _ in 0..FRAMES_TO_CHECK {
            let result = device.load_batch();
            assert!(
                matches!(result, RResult::ROk(())),
                "load batch failed: {result:?}"
            );
        }

        let batches = batches.lock().unwrap();
        assert_eq!(batches.len(), FRAMES_TO_CHECK);

        let frame_period_us = 1_000_000.0 * 1001.0 / 30_000.0;
        let mut previous_timestamp = None;
        let mut event_count = 0;
        for (frame, batch) in batches.iter().enumerate() {
            let frame_start = (frame as f64 * frame_period_us).floor() as EventTimestamp;
            let frame_end = ((frame + 1) as f64 * frame_period_us).ceil() as EventTimestamp;
            for event in batch {
                assert!(
                    (frame_start..=frame_end).contains(&event.t),
                    "event timestamp {} is outside frame {} interval [{}, {}]",
                    event.t,
                    frame,
                    frame_start,
                    frame_end,
                );
                if let Some(previous) = previous_timestamp {
                    assert!(
                        event.t >= previous,
                        "event timestamps moved backwards: {} then {}",
                        previous,
                        event.t,
                    );
                }
                previous_timestamp = Some(event.t);
                event_count += 1;
            }
        }
        assert!(
            event_count > 0,
            "lightning video produced no simulated events"
        );
    }
}
