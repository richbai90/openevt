# Raw file device plugin

`openevt_plugins::raw::RawFilePlugin` adapts the native [`RawFileReader`] to the
`abi_stable` plugin boundary, so you can replay a recording anywhere an
application would otherwise open a camera. It is intentionally an adapter, not a
second decoder.

It ships in the `openevt-plugins` crate alongside the event simulator, as one of
the standard devices most applications want on hand.

## Build and load

```sh
cargo build --release -p openevt-plugins
```

The resulting `libopenevt_plugins.so` exports the `openevt_device_plugin` root
module and advertises this device under the name `openevt_raw_file`. Place it in
one of the standard plugin directories, or configure the host plugin search path
with `OPENEVT_PLUGIN_PATH`.

The plugin library is separate from the `openevt` cdylib: the core
crate exports no root module of its own, so you can load the two into the same
process without their `abi_stable` root-module symbols colliding.

To make raw files discoverable, set `OPENEVT_RAW_FILES` to a platform-separated
list of EVT3 paths. The discovery serial is the path. An application can query
the host layer for the schema, collect a file path through any UI it chooses,
and pass the resulting configuration to the host layer’s
`PluginRegistry::open_device_with_configuration` method.

```rust,no_run
use openevt::PluginRegistry;

let mut registry = PluginRegistry::new();
registry.load_default_paths();
for camera in registry.list_devices() {
    println!("{}", camera.plugin_info.serial);
}

let serial = "path/to/events.raw";
let schema = registry.configuration_schema(serial)?;
let mut configuration = schema.new_configuration(serial);
configuration.values[0].value = Some(serial.into()).into();
let _device = registry.open_device_with_configuration(configuration)?;
```

## Event channel behavior

The native reader has crossbeam receivers for CD and external-trigger batches.
Those receivers never cross the ABI. Instead:

1. `start_events` calls `RawFileReader::cd_receiver` and retains the receiver.
2. `start_external_triggers` does the same for external-trigger events.
3. `load_batch` delegates to `RawFileReader::load_batch` and maps the native
   `Ok(None)` end-of-file result to `ROk(RNone)`.
4. The plugin drains all batches produced by that decode and calls the shared
   `EventBatchSink` once per batch.

The callback receives an `RSlice`, which is valid only for the duration of the
callback. Consumers that need to queue events must copy them. This is the same
ownership rule as a borrowed native facility buffer, and avoids leaking a Rust
allocator or channel implementation into third-party plugins.

Both event callbacks use the same sink object. The application may implement one
method and ignore the other when it needs only CD events. The plugin opens the reader
when it opens the device, so calling `load_batch` before `start_events` decodes
data and delivers nothing: the drain step finds no stored sink.

## Facilities

`PluginFacilityType` contains a stable descriptor key for every native HAL
facility: anti-flicker, decoders, camera sync, digital crop and event masks,
ERC, frame decoders, filters, event streams, geometry, software and hardware
information, registers, biases, monitoring, ROI, and trigger I/O. `Other` is
reserved for plugin-specific extensions.

The raw-file adapter reports the four capabilities it implements: `Geometry`,
`Index`, `Seek`, and `ExternalTriggerSeek`, each backed by a callable
`PluginFacilityHandle`. `PluginFacility` intentionally identifies a capability
without exposing native trait objects. Add operations as separately versioned
ABI traits as plugins need them; never add native `Any`, crossbeam, `String`,
or borrowed standard slices to the shared interface.

## Template for future devices

Future adapters should follow the same shape:

- keep the existing implementation as the source of truth;
- retain native channels, locks, and buffers inside the plugin;
- convert errors to `RString` at the boundary;
- use `RVec`, `RSlice`, `ROption`, and `RResult` in ABI methods;
- expose a small root-module constructor and a discovery implementation;
- document callback lifetime and backpressure behavior explicitly.
