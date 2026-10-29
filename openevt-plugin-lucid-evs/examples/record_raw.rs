//! Records raw EVT3.0 buffers from a LUCID EVS camera into a `.raw` file
//! that the openevt raw-file device can play back.
//!
//! The recorder bypasses the decoder entirely: it drives the plugin's
//! `RawEventStream` facility and appends every buffer verbatim after a
//! Metavision-style `%` header. While recording, it prints one line per
//! second with buffer / byte counts and the longest gap between buffers, so
//! bursty delivery can be attributed to the plugin or to the viewer.
//!
//! Usage:
//!     cargo build -p openevt-plugin-lucid-evs --release
//!     cargo run -p openevt-plugin-lucid-evs --release --example record_raw -- \
//!         [--serial 192.168.1.192] [--seconds 10] [--out capture.raw]
//!
//! Without `--serial` the first discovered camera is used.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use abi_stable::std_types::{ROption, RResult, RVec};
use openevt_abi::discovery::PluginRegistry;
use openevt_abi::plugin::{
    PluginConfiguration, PluginFacilityHandle, PluginFacilityType, PluginStreamBuffer,
};

struct Args {
    serial: Option<String>,
    seconds: u64,
    out: PathBuf,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        serial: None,
        seconds: 10,
        out: PathBuf::from("capture.raw"),
    };
    let mut iter = std::env::args().skip(1);
    while let Some(flag) = iter.next() {
        let mut value = || {
            iter.next()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        match flag.as_str() {
            "--serial" => args.serial = Some(value()?),
            "--seconds" => {
                args.seconds = value()?
                    .parse()
                    .map_err(|error| format!("invalid --seconds: {error}"))?
            }
            "--out" => args.out = PathBuf::from(value()?),
            "-h" | "--help" => {
                println!("usage: record_raw [--serial SERIAL] [--seconds N] [--out FILE]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(args)
}

fn locate_plugin_artifact() -> Result<PathBuf, String> {
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

fn main() -> Result<(), String> {
    let args = parse_args()?;

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

    let device = registry.open_device_with_configuration(PluginConfiguration {
        serial: serial.clone().into(),
        values: RVec::new(),
    })?;

    let (width, height) = match device
        .get_facility_handle(PluginFacilityType::Geometry)
        .into_option()
    {
        Some(PluginFacilityHandle::Geometry(geometry)) => {
            (geometry.get_width(), geometry.get_height())
        }
        _ => return Err("device does not expose a Geometry facility".into()),
    };
    let mut stream = match device
        .get_facility_handle(PluginFacilityType::RawEventStream)
        .into_option()
    {
        Some(PluginFacilityHandle::RawEventStream(stream)) => stream,
        _ => return Err("device does not expose a RawEventStream facility".into()),
    };
    // Mirror the viewer: the device box is dropped once the handles exist.
    drop(device);

    let file = File::create(&args.out).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    write!(
        writer,
        "% camera_integrator_name LUCID\n\
         % format EVT3;width={width};height={height}\n\
         % geometry {width}x{height}\n\
         % serial_number {serial}\n\
         % plugin_name openevt-plugin-lucid-evs\n\
         % end\n"
    )
    .map_err(|error| error.to_string())?;

    if let RResult::RErr(error) = stream.start() {
        return Err(format!("stream start failed: {error}"));
    }

    let start = Instant::now();
    let deadline = start + Duration::from_secs(args.seconds);
    let mut total_buffers = 0u64;
    let mut total_bytes = 0u64;
    let mut second_buffers = 0u64;
    let mut second_bytes = 0u64;
    let mut max_gap = Duration::ZERO;
    let mut last_buffer_at = start;
    let mut next_report = start + Duration::from_secs(1);

    eprintln!("recording {} s to {}", args.seconds, args.out.display());
    while Instant::now() < deadline {
        let PluginStreamBuffer { data, valid_len } = match stream.wait_next_buffer() {
            RResult::ROk(ROption::RSome(buffer)) => buffer,
            RResult::ROk(ROption::RNone) => {
                eprintln!("stream ended");
                break;
            }
            RResult::RErr(error) => {
                eprintln!("stream ended: {error}");
                break;
            }
        };
        let now = Instant::now();
        max_gap = max_gap.max(now - last_buffer_at);
        last_buffer_at = now;

        let payload = &data[..valid_len as usize];
        writer
            .write_all(payload)
            .map_err(|error| error.to_string())?;
        total_buffers += 1;
        total_bytes += valid_len;
        second_buffers += 1;
        second_bytes += valid_len;

        if now >= next_report {
            eprintln!(
                "t={:>5.1}s  buffers/s={:<5} KiB/s={:<8.1} max gap={:>6.1} ms",
                (now - start).as_secs_f64(),
                second_buffers,
                second_bytes as f64 / 1024.0,
                max_gap.as_secs_f64() * 1000.0
            );
            second_buffers = 0;
            second_bytes = 0;
            max_gap = Duration::ZERO;
            next_report += Duration::from_secs(1);
        }
    }

    let _ = stream.stop();
    writer.flush().map_err(|error| error.to_string())?;
    eprintln!(
        "wrote {total_buffers} buffers / {total_bytes} bytes ({:.1} MiB) to {}",
        total_bytes as f64 / (1024.0 * 1024.0),
        args.out.display()
    );
    Ok(())
}
