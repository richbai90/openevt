# Developing an OpenEVT Device Plugin

This guide explains how to create a Rust device plugin that can be discovered
and loaded by the OpenEVT host layer. It is written for developers who know
basic Rust but may not have written a dynamic library or an FFI
(foreign-function interface) before.

The guide uses the plugin API in `openevt::hal::device::plugin` and the loading
API in `openevt::hal::device::discovery::PluginRegistry`.

## What a Plugin Does

An OpenEVT plugin is a dynamic library that connects the OpenEVT host layer to
a camera or another event source. The host layer is the private, library-controlled
side of the ABI boundary. Applications built with OpenEVT interact with the
host layer through its public Rust API; they do not implement the ABI boundary.

The flow is:

1. The host layer loads the plugin library.
2. The library provides a root module with its name and a discovery factory.
3. The discovery object lists available devices and exposes a creation schema.
4. The host layer exposes the devices and schema to the application.
5. The application decides how to collect values, such as through a GUI or
   terminal.
6. The application passes those values to the host layer using the same public API
   regardless of how they were collected.
7. The host layer validates the configuration and sends it to the plugin.
8. The device reports its facilities and optionally sends event batches.

The application does not need to know how the camera is connected. A plugin
can use a USB SDK, a network protocol, a file reader, or a simulator internally.

The repository includes a complete raw-file adapter in
[`openevt-plugins/src/raw/plugin.rs`](../openevt-plugins/src/raw/plugin.rs). It
is the best reference when this guide and an implementation detail appear to
differ.

## A Little FFI Vocabulary

An ordinary Rust function call assumes that both sides were compiled with the
same Rust representation and can freely exchange Rust types. An FFI boundary
cannot make that assumption. The two sides need a stable contract describing
which functions exist and how values are represented.

This project uses [`abi_stable`](https://docs.rs/abi_stable) for that contract.
It supplies ABI-safe equivalents for common types:

| Rust type | Plugin ABI type | Typical use |
| --- | --- | --- |
| `String` | `RString` | Names and error messages |
| `&str` | `RStr<'_>` | Borrowed input strings |
| `Vec<T>` | `RVec<T>` | Device lists and facility lists |
| `Result<T, E>` | `RResult<T, E>` | Fallible plugin calls |
| `Option<T>` | `ROption<T>` | Optional facilities |
| `Box<T>` | `RBox<T>` | Owned values crossing the boundary |
| `&[T]` | `RSlice<'_, T>` | Borrowed event batches |

Use the ABI types in public plugin interfaces. Do not replace them with native
`String`, `Vec`, `Result`, channels, mutexes, or trait objects.

The plugin may use ordinary Rust types internally. For example, it can keep a
`crossbeam::Receiver` in its device struct and convert each received batch to
an `RSlice` only while invoking the host-layer callback.

## Project Setup

There are two useful layouts.

### Layout a: Add the Device to `openevt-plugins`

This is convenient for an adapter that reuses OpenEVT’s existing device code,
and it is where the standard devices live. Add a module next to `simulator` and
`raw`, then register it in the crate's single root module (`src/module.rs`):

```rust,ignore
extern "C" fn create_standard_discoveries() -> RVec<NamedDiscoveryPlugin> {
    vec![
        // ... existing devices ...
        NamedDiscoveryPlugin {
            name: MY_DEVICE_PLUGIN_NAME.into(),
            discovery: create_my_device_discovery(),
        },
    ]
    .into()
}
```

```sh
cargo build --release -p openevt-plugins
```

A shared library can only export one `abi_stable` root module, so devices in
this crate are *not* exported individually. `create_discoveries` returns one
[`NamedDiscoveryPlugin`] per device, and the host registers each separately
while preserving its name and creation schema.

Do not add an `export_root_module` to the core `openevt` crate. It is linked
into every plugin library as an `rlib`, so a root module exported from there
would collide with the plugin's own.

### Layout B: A Separate Plugin Crate

A separate crate is usually better for a vendor integration. Add OpenEVT and
`abi_stable` as dependencies, and configure the plugin crate as a dynamic
library:

```toml
[package]
name = "example-camera-plugin"
version = "0.1.0"
edition = "2024"

[dependencies]
abi_stable = "0.11.3"
openevt = { version = "1.0.4", features = ["plugins"] }

[lib]
crate-type = ["cdylib"]
```

In practice, build the plugin against the same OpenEVT plugin API version as
the host layer. Keep `abi_stable` versions aligned as well. The root module performs
ABI compatibility checks when the host layer loads the library, but avoiding version
drift makes failures easier to diagnose.

## The Three Pieces Every Plugin Needs

Every plugin has these parts:

- A device type implementing `DevicePlugin`;
- A discovery type implementing `DeviceDiscoveryPlugin`;
- An exported root-module constructor returning `DevicePluginModuleRef`.

The following sections build them one at a time.

## Step 1: Define a Device Type

Start with the state needed to communicate with one device. The serial number
is required by the API. Other fields are private implementation details.

```rust
use abi_stable::std_types::{RResult, RString, RVec};
use openevt::hal::device::discovery::ConnectionType;
use openevt::hal::device::plugin::{
    DevicePlugin, EventBatchSinkBox, PluginFacility, PluginFacilityType,
};

struct ExampleCamera {
    serial: RString,
    // Add a camera SDK handle, socket, file reader, or simulator here.
}

impl ExampleCamera {
    fn open(serial: &str) -> Result<Self, String> {
        // Connect to the real device here and return an error if it fails.
        Ok(Self {
            serial: serial.into(),
        })
    }

    fn error(result: Result<(), String>) -> RResult<(), RString> {
        match result {
            Ok(()) => RResult::ROk(()),
            Err(message) => RResult::RErr(message.into()),
        }
    }
}
```

A useful pattern is to keep connection errors as ordinary `Result` values
inside the implementation and convert them at the ABI boundary. This keeps
the internal code idiomatic and makes every exported method return the type the
the host layer expects.

## Step 2: Report Facilities

A facility is a capability, such as geometry, ROI control, or an event stream.
The plugin API uses `PluginFacilityType` as a stable key. The legacy
`PluginFacility` value is only a capability descriptor. For callable
capabilities, implement the corresponding ABI-safe facility trait and return
its type-erased `PluginFacilityHandle` from `get_facility_handle`.

The ABI knows the complete native facility key set, including:

- geometry and hardware identification;
- event streams, base decoders, event decoders, and CD/external-trigger
  decoders;
- camera sync, anti-flicker, ERC, digital crop, and digital event masks;
- frame decoders and event filters;
- software information, monitoring, hardware registers, and low-level biases;
- ROI, ROI pixel masks, trigger input, and trigger output.

Only advertise a facility when the plugin really implements it. A raw-file
plugin, for example, advertises geometry, hardware identification, ROI, event
stream, event-stream decoder, and event decoder. It does not claim to support
hardware registers merely because that enum variant exists.

```rust
impl DevicePlugin for ExampleCamera {
    fn get_facilities(&self) -> RVec<PluginFacilityType> {
        vec![
            PluginFacilityType::Geometry,
            PluginFacilityType::HardwareIdentification,
            PluginFacilityType::RawEventStream,
            PluginFacilityType::EventSubscription,
        ]
        .into()
    }

    fn get_facility(
        &self,
        facility_type: PluginFacilityType,
    ) -> abi_stable::std_types::ROption<PluginFacility> {
        if self.get_facilities().contains(&facility_type) {
            Some(PluginFacility::new(facility_type)).into()
        } else {
            abi_stable::std_types::ROption::RNone
        }
    }

    // Other DevicePlugin methods are shown below.
    # fn serial(&self) -> RString { self.serial.clone() }
    # fn connection_type(&self) -> ConnectionType { ConnectionType::Usb }
    # fn start_events(&mut self, _sink: EventBatchSinkBox) -> RResult<(), RString> { RResult::ROk(()) }
    # fn start_external_triggers(&mut self, _sink: EventBatchSinkBox) -> RResult<(), RString> { RResult::ROk(()) }
    # fn load_batch(&mut self) -> RResult<(), RString> { RResult::ROk(()) }
}
```

The `#` lines are hidden by Rust documentation renderers to keep the example
focused; in a source file, provide all required trait methods normally.

The plugin facility traits mirror the native facility concepts without
transporting native Rust trait objects. For example,
`PluginRawEventStreamDecoderFacility` exposes `decode` and timestamp operations,
and `PluginEventSubscriptionFacility` uses `EventBatchSinkBox` in place of native
channels. Native `Any`, crossbeam channels, locks, and borrowed standard
slices must stay inside the plugin. Use the ABI-safe equivalents
(`RSlice`, `RVec`, `RResult`, and callback traits) at the boundary.

The descriptor accessor remains available while existing plugins migrate:

```rust
fn get_facility_handle(
    &self,
    facility_type: PluginFacilityType,
) -> ROption<PluginFacilityHandle> {
    match facility_type {
        PluginFacilityType::Geometry => Some(PluginFacilityHandle::Geometry(
            PluginGeometryFacility_TO::from_value(
                MyGeometryFacility { width: 640, height: 480 },
                TD_Opaque,
            ),
        )).into(),
        _ => ROption::RNone,
    }
}
```

The host layer calls the returned type-erased handle through its generated ABI
vtable; the concrete implementation and all native state remain in the
plugin. Facilities not yet represented by a plugin trait should not be
treated as callable merely because their descriptor is advertised.

Optional device behavior is also represented as facilities. Plugins can expose
`PluginIndexFacility` for `t_min`/`t_max`, `PluginSeekFacility` for timestamp
seeking, and `PluginExternalTriggerSeekFacility` for seeking to the next
external trigger. A device that does not support one of these capabilities
simply omits its facility handle; it does not need a placeholder method or a
`todo!()` implementation.

## Step 3: Declare and Validate Creation Parameters

Device resources belong to the plugin, so the host layer does not pass an open file
handle, socket, or SDK object across the ABI. Instead, discovery returns a
versioned TOML schema. The host layer exposes that schema to the application, and the
application chooses how to collect the values. A GUI application can use
`kind = "file"` to open a file picker; a terminal application can read a path
from an argument.

For example, a raw-file plugin can expose:

```toml
version = 1

[[parameters]]
name = "input_file"
label = "Input event file"
kind = "file"
required = true
description = "An EVT3 event file to replay."
extensions = ["raw", "evt3"]
```

The ABI configuration object is deliberately less opinionated than the
schema. Every field is represented as `ROption<RString>`, including required
fields, so an in-progress application form is representable. The host layer provides
`PluginRegistry::configuration_schema` to expose the parsed plugin schema,
`PluginConfigurationSchema::new_configuration` to create the configuration
object, and `PluginConfigurationSchema::validate` to validate it before
opening the device.

The plugin must validate again inside `open_device_with_configuration` before
opening its resource. This keeps the plugin authoritative and ensures that
malformed callers cannot cause it to acquire a resource with an incomplete
configuration. The plugin owns the resource and its lifetime.

The legacy serial-only `open_device` method remains available during
migration, but new plugins should implement the configuration-aware method.

## Step 4: Implement the Device Life Cycle
The remaining `DevicePlugin` methods describe the device and drive event
delivery:

```rust
impl DevicePlugin for ExampleCamera {
    fn serial(&self) -> RString {
        self.serial.clone()
    }

    fn connection_type(&self) -> ConnectionType {
        ConnectionType::Usb
    }

    fn get_facilities(&self) -> RVec<PluginFacilityType> {
        vec![PluginFacilityType::EventSubscription].into()
    }

    fn get_facility(
        &self,
        facility_type: PluginFacilityType,
    ) -> abi_stable::std_types::ROption<PluginFacility> {
        if self.get_facilities().contains(&facility_type) {
            Some(PluginFacility::new(facility_type)).into()
        } else {
            abi_stable::std_types::ROption::RNone
        }
    }

    fn start_events(&mut self, _sink: EventBatchSinkBox) -> RResult<(), RString> {
        // Store the sink and start or subscribe to the CD event source.
        RResult::ROk(())
    }

    fn start_external_triggers(
        &mut self,
        _sink: EventBatchSinkBox,
    ) -> RResult<(), RString> {
        // Store the sink and start or subscribe to trigger events.
        RResult::ROk(())
    }

    fn load_batch(&mut self) -> RResult<(), RString> {
        // Read/decode one unit of work, then invoke the stored sink for every
        // batch produced by that work.
        RResult::ROk(())
    }
}
```

### Pull-Driven Event Delivery
The current API is pull-driven. The host layer calls `load_batch`, and the plugin
does the work synchronously. A plugin may use a background thread internally,
but `load_batch` must still provide a clear synchronization point and should
return an error if the device is not initialized.

`start_events` and `start_external_triggers` receive the same kind of callback
sink. Store it in the device state if events will be delivered later. A simple
plugin can instead read and decode directly from `load_batch`.

## Step 4: Send Event Batches Safely

The callback methods use borrowed `RSlice` values:

```rust
#[sabi_trait]
pub trait EventBatchSink: Send + Sync {
    fn on_cd_events(&self, events: RSlice<'_, EventCD>);
    fn on_ext_events(&self, events: RSlice<'_, EventExtTrigger>);
}
```

The slice is valid only during the callback. If the host layer needs to retain the
events, it must copy them. The plugin must also ensure that its backing storage
stays alive until the callback returns.

A typical adapter drains an internal receiver:

```rust
fn drain_cd_batches(&mut self) {
    while let Ok(batch) = self.receiver.try_recv() {
        if let Some(sink) = &self.sink {
            sink.on_cd_events(batch.as_slice().into());
        }
    }
}
```

Do not pass a receiver, mutex, raw pointer, or native `Vec` through the ABI.
Do not keep an `RSlice` after the callback returns.

## Step 5: Implement Discovery

Discovery is separate from opening a device. It should be cheap and should not
open every camera just to list it.

```rust
use abi_stable::std_types::{RResult, RString, RVec};
use openevt::hal::device::plugin::{
    DeviceDiscoveryPlugin, DevicePluginBox, PluginCameraDescriptionAbi,
};

struct ExampleDiscovery;

impl DeviceDiscoveryPlugin for ExampleDiscovery {
    fn discover(&self) -> RVec<PluginCameraDescriptionAbi> {
        // Replace this with SDK enumeration or configuration-file loading.
        vec![PluginCameraDescriptionAbi {
            serial: "CAM-001".into(),
            connection: ConnectionType::Usb,
        }]
        .into()
    }

    fn open_device(&self, serial: abi_stable::std_types::RStr<'_>)
        -> RResult<DevicePluginBox, RString>
    {
        match ExampleCamera::open(serial.as_str()) {
            Ok(camera) => RResult::ROk(DevicePlugin_TO::from_value(
                camera,
                abi_stable::type_level::downcasting::TD_Opaque,
            )),
            Err(message) => RResult::RErr(message.into()),
        }
    }
}
```

The serial string is a plugin-defined lookup key, but it should be stable and
unique within the plugin. For a file plugin, the path can be the serial. For a
camera SDK, use the camera’s hardware serial number.

If the serial is unknown or opening fails, return `RResult::RErr` with a useful
message. The host layer’s registry tries each loaded discovery plugin until one can
open the requested serial.

## Step 6: Export the Root Module

The root module is the entry point the loader looks for. It provides a plugin
name and a function that constructs the discovery object.

```rust
use abi_stable::{
    prefix_type::PrefixTypeTrait,
    std_types::RString,
    type_level::downcasting::TD_Opaque,
};
use openevt::hal::device::plugin::{
    DeviceDiscoveryPluginBox, DeviceDiscoveryPlugin_TO, DevicePluginModuleRef,
};

extern "C" fn plugin_name() -> RString {
    "example_camera".into()
}

extern "C" fn create_discovery() -> DeviceDiscoveryPluginBox {
    DeviceDiscoveryPlugin_TO::from_value(ExampleDiscovery, TD_Opaque)
}

#[abi_stable::export_root_module]
pub fn instantiate_root_module() -> DevicePluginModuleRef {
    openevt::hal::device::plugin::DevicePluginModuleVtable {
        name: plugin_name,
        create_discovery,
    }
    .leak_into_prefix()
}
```

The function name is conventional, but the `export_root_module` attribute is
essential. The module’s base name, name, and version are defined by OpenEVT’s
`DevicePluginModuleRef`; do not invent a different root module contract.

## Building and Loading

Build the plugin in release mode:

```sh
cargo build --release --features plugins
```

On common platforms, the resulting library is under `target/release/`:

- Linux: `libexample_camera.so`
- macOS: `libexample_camera.dylib`
- Windows: `example_camera.dll`

The host layer searches:

1. directories listed in `OPENEVT_PLUGIN_PATH`;
2. `/usr/lib/openevt/plugins`;
3. `/usr/local/lib/openevt/plugins`.

`OPENEVT_PLUGIN_PATH` uses the platform’s normal path-list separator. A
temporary development setup can use a directory containing the built library:

```sh
OPENEVT_PLUGIN_PATH="$PWD/target/release" cargo run --example plugin-host
```

The host layer loads plugins through `PluginRegistry`:

```rust,no_run
use openevt::hal::device::discovery::PluginRegistry;

let mut registry = PluginRegistry::new();
let loaded = registry.load_default_paths();
println!("loaded {loaded} plugin(s)");

for camera in registry.list_devices() {
    println!(
        "{}: {}",
        camera.plugin_name,
        camera.plugin_info.serial,
    );
}

let device = registry.open_device("CAM-001")?;
println!("opened {}", device.serial());
# Ok::<(), Box<dyn std::error::Error>>(())
```

For deterministic tests, load a specific file or directory instead:

```rust,no_run
use std::path::Path;
use openevt::hal::device::discovery::PluginRegistry;

let mut registry = PluginRegistry::new();
registry.load_file(Path::new("target/release/libexample_camera.so"))?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Testing Strategy

Test the plugin in layers:

1. Unit-test the native camera adapter without loading a dynamic library.
2. Test discovery with fake or recorded device identifiers.
3. Test `get_facilities` and `get_facility` together so the two methods never
   disagree.
4. Test errors such as an unknown serial, disconnected camera, and calling
   `load_batch` before initialization.
5. Test event callbacks with a counting sink and verify batch counts and event
   counts.
6. Build the `cdylib` and load it through `PluginRegistry` in an integration
   test.

A callback test should verify ownership too: copy events inside the callback,
then inspect the copies after the callback returns. Never rely on the borrowed
slice remaining valid.

## Common Mistakes

### Returning the Wrong Facility List

`get_facilities` is a capability declaration. Returning every value in
`PluginFacilityType::ALL` claims support for every operation. Return only what
the device actually implements.

### Passing Native Rust Types Through the Boundary

Use `RString`, `RVec`, `RResult`, `ROption`, `RBox`, and `RSlice` in exported
interfaces. Native channels and synchronization primitives belong in private
fields of the plugin object.

### Holding an Event Slice Too Long

`RSlice` is borrowed. Copy it during the callback if it must outlive that
callback.

### Forgetting `cdylib`

An `rlib` is a Rust library, not a loadable plugin for the registry. Set
`crate-type = ["cdylib"]` in a standalone plugin crate.

### Loading the Wrong File

`PluginRegistry::load_directory` skips files whose extension is not `so`,
`dll`, or `dylib`. Check that the library was built for the host layer’s operating
system and CPU architecture.

### ABI or Version Mismatch

Build the host layer and plugin against compatible OpenEVT and `abi_stable` versions.
If loading fails, first rebuild both from clean, matching dependency locks and
check the exact library path being loaded.

### Doing Expensive Work in Discovery

`discover` is called to enumerate devices. Avoid opening streams or starting
threads there. Defer connection setup to `open_device`.

## Recommended Implementation Order

For a first plugin, implement in this order:

1. A native `ExampleCamera::open` function and a unit test for it.
2. `DevicePlugin::serial` and `connection_type`.
3. `DevicePlugin::get_facilities` and `get_facility`.
4. `DeviceDiscoveryPlugin::discover` with one known test device.
5. `open_device` and the root module.
6. Loading through `PluginRegistry`.
7. Event callbacks and `load_batch`.
8. Additional versioned facility operations as the hardware requires them.

This order gives you a loadable, discoverable plugin early, before adding the
more complicated device I/O and event lifetime behavior.
