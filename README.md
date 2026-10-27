# openevt

`openevt` is an independently maintained Rust crate for OpenEB-inspired event
camera data and device abstractions. It is related to Prophesee's OpenEB only
in spirit and is not endorsed, sponsored, or maintained by Prophesee.

The HAL and shared buffer utilities are always available. File-backed raw
device support and the frame-generation plugin ABI are optional:

```toml
[dependencies]
openevt = { version = "1.0.4", features = ["devices"] }
```

The `all` feature enables every optional component and is the default feature.
Use `default-features = false` for a core-only build.

## Documentation

The complete developer documentation is available as a GitBook-style guide in
[`docs/README.md`](docs/README.md), with navigation in
[`docs/SUMMARY.md`](docs/SUMMARY.md). It covers architecture, ABI rules,
plugin lifecycle, configuration schemas, testing, and troubleshooting.

OpenEVT supports two plugin families:

- [Device plugins](docs/plugin-development-guide.md) connect cameras, files,
  network sources, and simulators to the host layer.
- [Frame-generation plugins](docs/framegen-plugin.md) consume decoded CD
  events and render RGBA frames.

The repository includes a [raw-file plugin reference](docs/raw-file-plugin.md)
and a [video simulator plugin](docs/simulator-plugin.md). See the
[development workflow](docs/development-workflow.md) for build, discovery,
testing, and troubleshooting commands.

## Layout

```text
.
├── Cargo.toml
├── docs/        # Plugin development and architecture guides
├── crates/
│   ├── openevt-core/    # types, HAL traits/facilities, decoders, algorithms
│   ├── openevt-devices/ # file-backed raw event device
│   ├── openevt-abi/     # stable plugin ABI, registry, host adapters, framegen
│   └── openevt-py/      # PyO3 bindings (python feature)
└── src/             # facade crate re-exporting the workspace crates
```

Enable the frame-generation API with `features = ["framegen"]`. It is also
included by the default `all` feature.

The primary modules are:

- `openevt-core` contains event types, decoder and device traits, facilities,
  dispatchers, buffers, and algorithms.
- `openevt-devices` contains raw-file header parsing, indexing,
  stream/decoder wiring, and reader/iterator APIs. Its main types are also
  re-exported at the crate root when the `devices` feature is enabled.
- `openevt-abi` hosts the stable plugin ABI, plugin registry/loading, and the
  host-side plugin adapters, including the frame-generation contract.
- `openevt-py` hosts the PyO3 bindings, gated behind the `python` feature.

For a developer-focused walkthrough of dynamic device plugins, see the
[plugin development guide](docs/plugin-development-guide.md). Raw-file access
is available through the [raw-file plugin](docs/raw-file-plugin.md) or directly
through the public `RawFileReader` convenience API; the native handler and
stream implementation remain private details.
The video simulator is documented in the [simulator plugin guide](docs/simulator-plugin.md).

Set `OPENEVT_PLUGIN_PATH` to the directory containing the plugin before using a
host layer or the Python bindings. Raw-file discovery uses `OPENEVT_RAW_FILES`.

EVT3 decoding is currently supported; EVT2, DAT, and HDF5 decoder paths are not
yet implemented.

The `property!` and `pack_facility!` macros, along with the `new` derive
re-export, now live directly in the crate. The previous standalone macro and
procedural-macro packages are no longer needed.

## Runtime flow

1. The host layer loads the raw-file plugin through `PluginRegistry`.
2. The plugin opens the discovered raw-file serial.
3. Its private stream and decoder read and decode EVT3 data.
4. Decoded events cross the ABI through `EventBatchSink` callbacks.
5. Applications use the host layer API to subscribe to batches; optional indexing
   supports `seek`.

## Building

```bash
cargo check                         # default/all features
cargo check --no-default-features    # core only
cargo test --all-features
cargo build --features python       # Python/NumPy bindings; loads raw support as a plugin
```

To build Python wheels locally, install Maturin and run:

```bash
python -m pip install maturin
maturin build --release
```

The resulting wheels are written to `target/wheels/`. Test them in a clean
environment before uploading with `maturin publish` or a PyPI publishing
tool. The published distribution and importable module are named `openevt`:

```bash
python -m pip install openevt
```

## Attribution and independence

This project is a separate rewrite inspired by the architecture and event
camera abstractions in [Prophesee's OpenEB](https://github.com/prophesee-ai/openeb).
We gratefully credit the Prophesee team for that foundational work. `openevt`
is independently maintained and is not affiliated with, endorsed by, or
sponsored by Prophesee.

The implementation is incomplete: EVT3 is the currently supported raw decoder,
while EVT2, DAT, and HDF5 paths remain unfinished, and several facilities are
still scaffolding.
