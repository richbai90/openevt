//! Probes the exact event path that EVT3-Viewer uses in live mode:
//!
//!   RawEventStreamFacility::poll_buffer()          (non-blocking)
//!     -> RawEventStreamDecoderFacility::decode(raw)
//!        -> decoder harvest -> pending_events -> registered sink
//!           -> host EventBatchSink::on_cd_events
//!
//! Phase A drives poll_buffer/decode in a tight loop with no pacing,
//! phase B paces the loop like the viewer player thread (one accumulation
//! window per frame, default 33.33 ms), phase C removes pacing again to
//! measure how quickly the pipeline recovers. Every phase prints one line
//! per second with buffers, bytes, received events, and poll hit/empty
//! counts, so bursty delivery can be attributed to the poll path itself.
//!
//! Usage:
//!     cargo build -p openevt-plugin-lucid-evs --release --example poll_probe
//!     cargo run -p openevt-plugin-lucid-evs --release --example poll_probe -- \
//!         [--serial 192.168.1.121] [--seconds 20] [--frame-ms 33.33]
//!
//! Without `--serial` the first discovered camera is used.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use abi_stable::std_types::{RResult, RVec};
use openevt::hal::device::discovery::PluginRegistry;
use openevt::hal::device::plugin::{
    EventBatchSink, PluginConfiguration, EventBatchSink_TO, PluginFacilityHandle,
    PluginFacilityType,
};
use openevt::types::EventCD;

struct Args {
    serial: Option<String>,
    seconds: u64,
    frame_ms: f64,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        serial: None,
        seconds: 20,
        frame_ms: 33.333_333,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(flag) = iter.next() {
        let mut value = || iter.next().ok_or_else(|| format!("{flag} requires a value"));
        match flag.as_str() {
            "--serial" => args.serial = Some(value()?),
            "--seconds" => {
                args.seconds = value()?
                    .parse()
                    .map_err(|error| format!("invalid --seconds: {error}"))?
            }
            "--frame-ms" => {
                args.frame_ms = value()?
                    .parse()
                    .map_err(|error| format!("invalid --frame-ms: {error}"))?
            }
            "-h" | "--help" => {
                println!("usage: poll_probe [--serial SERIAL] [--seconds N] [--frame-ms MS]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(args)
}

fn locate_plugin_artifact() -> Result<std::path::PathBuf, String> {
    let candidates = [
        "libopenevt_plugin_lucid_evs.so",
        "openevt_plugin_lucid_evs.dll",
        "libopenevt_plugin_lucid_evs.dylib",
    ];
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut dir = exe.parent().map(|path| path.to_path_buf());
    while let Some(current) = dir {
        for candidate in candidates {
            let path = current.join(candidate);
            if path.is_file() {
                return Ok(path);
            }
        }
        dir = current.parent().map(|parent| parent.to_path_buf());
    }
    Err(format!(
        "plugin cdylib not found near {}; run `cargo build -p openevt-plugin-lucid-evs --release` first",
        exe.display()
    ))
}

/// Host-side sink, mirrors the viewer's PluginEventSink (state.rs): pushes
/// every decoded batch into an unbounded channel for the consumer thread.
#[derive(Clone)]
struct ProbeSink {
    events: Arc<AtomicU64>,
}

impl EventBatchSink for ProbeSink {
    fn on_cd_events(&self, events: abi_stable::std_types::RSlice<'_, EventCD>) {
        self.events.fetch_add(events.len() as u64, Ordering::Relaxed);
    }

    fn on_ext_events(
        &self,
        _events: abi_stable::std_types::RSlice<'_, openevt::types::EventExtTrigger>,
    ) {
    }
}

fn main() -> Result<(), String> {
    let args = parse_args()?;
    let _guard = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .try_init();

    let mut registry = PluginRegistry::new();
    registry
        .load_file(&locate_plugin_artifact()?)
        .map_err(|error| error.to_string())?;

    let serial = match args.serial {
        Some(serial) => serial,
        None => {
            let devices = registry.list_devices();
            let first = devices
                .first()
                .ok_or("no LUCID cameras discovered; pass --serial <ip>")?;
            first.plugin_info.serial.to_string()
        }
    };
    eprintln!("opening {serial}");

    let device = registry
        .open_device_with_configuration(PluginConfiguration {
            serial: serial.clone().into(),
            values: RVec::new(),
        })?;

    let mut stream = match device
        .get_facility_handle(PluginFacilityType::RawEventStream)
        .into_option()
    {
        Some(PluginFacilityHandle::RawEventStream(stream)) => stream,
        _ => return Err("device does not expose a RawEventStream facility".into()),
    };
    let mut decoder = match device
        .get_facility_handle(PluginFacilityType::RawEventStreamDecoder)
        .into_option()
    {
        Some(PluginFacilityHandle::RawEventStreamDecoder(decoder)) => decoder,
        _ => return Err("device does not expose a RawEventStreamDecoder facility".into()),
    };
    let mut subscription = match device
        .get_facility_handle(PluginFacilityType::EventSubscription)
        .into_option()
    {
        Some(PluginFacilityHandle::EventSubscription(subscription)) => subscription,
        _ => return Err("device does not expose an EventSubscription facility".into()),
    };
    // Mirror the viewer: the device box is dropped once the handles exist.
    drop(device);

    let received = Arc::new(AtomicU64::new(0));
    subscription
        .subscribe_to_cd_events(EventBatchSink_TO::from_value(
            ProbeSink {
                events: Arc::clone(&received),
            },
            abi_stable::type_level::downcasting::TD_Opaque,
        ))
        .into_result()
        .map_err(|error| error.to_string())?;

    if let RResult::RErr(error) = stream.start() {
        return Err(format!("stream start failed: {error}"));
    }

    let start = Instant::now();
    let frame_interval = Duration::from_secs_f64(args.frame_ms / 1000.0);
    let deadline = start + Duration::from_secs(args.seconds);
    // Phase split: tight (A) -> paced (B) -> tight (C). The 20 s default
    // gives ~5 s / ~10 s / ~5 s.
    let paced_until = start + Duration::from_secs_f64(args.seconds as f64 * 0.25);
    let resume_tight = start + Duration::from_secs_f64(args.seconds as f64 * 0.75);

    let mut total_buffers = 0u64;
    let mut total_bytes = 0u64;
    let mut total_polls = 0u64;
    let mut empty_polls = 0u64;
    let mut sec_buffers = 0u64;
    let mut sec_bytes = 0u64;
    let mut sec_polls = 0u64;
    let mut sec_empty = 0u64;
    let mut next_report = start + Duration::from_secs(1);
    let mut paced = false;

    eprintln!(
        "probing viewer path for {} s: A tight 0-{:>4.1}s, B paced {:>4.1}-{:>4.1}s @ {:.2} ms, C tight {:>4.1}s-{}s",
        args.seconds,
        (paced_until - start).as_secs_f64(),
        (paced_until - start).as_secs_f64(),
        (resume_tight - start).as_secs_f64(),
        args.frame_ms,
        (resume_tight - start).as_secs_f64(),
        args.seconds
    );

    let mut last_events = 0u64;

    while Instant::now() < deadline {
        let now = Instant::now();
        let now_paced = now >= paced_until && now < resume_tight;
        if now_paced != paced {
            paced = now_paced;
            eprintln!(
                "--- phase {} @ t={:.1}s ---",
                if paced { "B (viewer-paced)" } else { "tight" },
                (now - start).as_secs_f64()
            );
        }

        let frame_started = Instant::now();

        // Exactly one poll per frame cycle, like the viewer player thread.
        let RResult::ROk(buffer) = stream.poll_buffer() else {
            return Err("poll_buffer returned an error".into());
        };
        total_polls += 1;
        sec_polls += 1;
        let valid_len = buffer.valid_len as usize;
        if valid_len > 0 {
            let payload = &buffer.data[..valid_len];
            total_buffers += 1;
            total_bytes += valid_len as u64;
            sec_buffers += 1;
            sec_bytes += valid_len as u64;
            if let RResult::RErr(error) = decoder.decode(payload.into()) {
                return Err(format!("decode failed: {error}"));
            }
        } else {
            empty_polls += 1;
            sec_empty += 1;
            // The viewer decodes whatever poll_buffer returns, including
            // empty buffers (dispatch flushes the decoder, no-op on empty).
            if let RResult::RErr(error) = decoder.decode(buffer.data[..0].into()) {
                return Err(format!("decode of empty buffer failed: {error}"));
            }
        }

        // Viewer pacing: the player thread sleeps the remainder of the
        // accumulation window and does not poll during the sleep.
        if paced {
            let elapsed = frame_started.elapsed();
            if let Some(sleep) = frame_interval.checked_sub(elapsed) {
                std::thread::sleep(sleep);
            }
        }

        // Per-second report.
        if frame_started >= next_report {
            let t = (frame_started - start).as_secs_f64();
            let events = received.load(Ordering::Relaxed);
            let phase = if paced { "B" } else { "A" };
            eprintln!(
                "{phase} t={:>5.1}s  bufs/s={:<6} KiB/s={:<8.1} polls/s={:<6} empty={:<6} ev/s={:<9}",
                t,
                sec_buffers,
                sec_bytes as f64 / 1024.0,
                sec_polls,
                sec_empty,
                events - last_events
            );
            last_events = events;
            sec_buffers = 0;
            sec_bytes = 0;
            sec_polls = 0;
            sec_empty = 0;
            next_report += Duration::from_secs(1);
        }
    }

    // Close-out: the consumer may still be receiving; wait one second and
    // print totals with phase-independent counters.
    std::thread::sleep(Duration::from_secs(1));
    let total_events = received.load(Ordering::Relaxed);
    eprintln!();
    eprintln!("totals: buffers={total_buffers} bytes={total_bytes} ({:.1} MiB) polls={total_polls} empty_polls={empty_polls} events={total_events}",
        total_bytes as f64 / (1024.0 * 1024.0));
    eprintln!(
        "decode efficiency: {} events from {} KiB = {} B/event (baseline camctl: ~5.2 B/event)",
        total_events,
        total_bytes / 1024,
        if total_buffers > 0 { total_bytes / total_events.max(1) } else { 0 }
    );
    Ok(())
}
