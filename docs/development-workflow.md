# Build, test, and troubleshoot

## Build the workspace

From the repository root:

```sh
cargo check --workspace
cargo test --workspace
cargo build --release --workspace
```

`cargo test --workspace` covers the default `all` feature set. Avoid
`--all-features`: it enables the `python` feature, and the `extension-module`
PyO3 build cannot link test binaries. Test the bindings separately with
`cargo build --features python` followed by `pytest tests/python`; those tests
load the debug extension and the plugin library directly, without maturin.

The device plugin crate is `openevt-plugins`. Its library builds as both
`rlib` and `cdylib`; the shared library under `target/release` is the
artifact to place on the plugin search path. The simulator additionally needs
FFmpeg development libraries available to `ffmpeg-next`.

## Load a device plugin explicitly

```sh
export OPENEVT_PLUGIN_PATH="$PWD/target/release"
cargo test -p openevt-plugins --test plugin_discovery -- --nocapture
```

The discovery integration test expects a built plugin directory and verifies
that `openevt_simulator` advertises the `EventSimulator` device, that
`openevt_raw_file` registers from the same library, and that opening a missing
file reports the plugin's error.

To inspect a built plugin by hand, run the probe example against any raw file:

```sh
OPENEVT_PLUGIN_PATH="$PWD/target/release" cargo run --example probe_plugins -- some.raw
```

It prints load results per library, discovered devices, and the reason
`open_device` fails when it does.

## Configure a device from the host

The normal sequence is:

1. Create `PluginRegistry`.
2. Call `load_default_paths()` or `load_directory()`.
3. Call `list_devices()`.
4. Request `configuration_schema(serial)`.
5. Create `new_configuration(serial)` and fill values by parameter name.
6. Call `open_device_with_configuration()`.
7. Obtain facilities, subscribe sinks, start the stream, and load batches.

Do not assume parameter ordering; use each schema parameter's stable `name`.
The schema lists defaults as suggestions; the host does not insert them
automatically into the configuration object.

## Common failures

**No libraries load.** Confirm the directory exists, contains the platform
library extension, and is in `OPENEVT_PLUGIN_PATH`. Check that you built the host
and plugin from compatible sources.

**Schema validation fails.** Check required values, spelling, duplicate
parameters, and textual representations (`true`, integers, finite floats, and
enum choices). The host validates before opening, and the plugin validates
again by design.

**Events never arrive.** Subscribe before starting/loading. For a device
plugin, call the stream's `start` or the legacy `start_events` path before
requesting batches. Empty CD batches are meaningful, and the plugin should still
deliver them when a source advanced.

**Seeking produces wrong timestamps.** Reset all state that affects output,
discard prefetched data, and seek the decoder and model together. The simulator
replays from the beginning because a codec keyframe seek alone does not restore
stateful event-model alignment.

**A frame generator shows stale pixels.** Clear or fully overwrite the RGBA
buffer in `render`; do not update only the coordinates that received events
unless the contract explicitly defines a persistent framebuffer.

**FFmpeg cannot open a video.** Verify the input has a video stream, FFmpeg is
installed, dimensions are nonzero, and the selected output dimensions are
positive. The simulator uses the encoded average FPS unless `fps` overrides it.

## Performance checklist

- Reuse decoder, scaler, event, and output buffers.
- Bound producer queues and define stop/drop behavior.
- Avoid holding a global lock while invoking user callbacks.
- Keep deterministic state transitions if reproducibility matters.
- Benchmark with representative resolution, FPS, event rate, and seek patterns.
- Use the repository's `hotpath` instrumentation only when profiling; do not
  make profiling output part of the plugin ABI.
