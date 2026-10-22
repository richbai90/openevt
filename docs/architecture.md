# Architecture and mental model

## Workspace layout

The project is a Cargo workspace split along dependency lines:

| Crate | Contents |
| --- | --- |
| `openevt-core` | Shared event types, HAL traits and facilities, dispatcher, algorithms, buffers, decoders (EVT2/EVT3/DAT/raw). Depends on nothing else in the workspace. |
| `openevt-devices` | File-backed raw event device: header parsing, raw file reader, stream, index, iterators. Depends on `openevt-core`. |
| `openevt-abi` | The stable plugin ABI: `plugin.rs` type definitions, plugin registry/loading, configuration schema, the host-side `PluginDevice` adapter, and framegen ABI. Depends on `openevt-core`. |
| `openevt-py` | PyO3 bindings behind the `python` feature. Edge of the dependency tree; depends on `openevt-core`, `openevt-devices`, and `openevt-abi`. |
| `openevt` (root) | Facade crate re-exporting the workspace crates for single-crate consumers. |

PyO3 bindings compile only in `openevt-py`; the domain crates never link
PyO3. The `sample.raw` test fixture lives in `crates/openevt-devices/tests/`.

## Runtime flow

```text
application
    │  discovers / configures / subscribes
    ▼
OpenEVT host layer ── loads ──► dynamic plugin library
    │                              │
    │ ABI-safe calls                ├─ discovery
    │ callbacks                     ├─ device or generator state
    ▼                              └─ native SDK / decoder / renderer
event consumers or frames
```

Device plugins are discovered through `PluginRegistry`. The registry scans
`OPENEVT_PLUGIN_PATH`, `/usr/lib/openevt/plugins`, and
`/usr/local/lib/openevt/plugins` for `.so`, `.dll`, or `.dylib` files. It loads
libraries whose root module is `openevt_device_plugin`, creates a discovery
object, and aggregates the devices advertised by that object.

Frame generators use the same `abi_stable` approach but have a smaller root
module. A host loads `frame_generator_plugin`, calls `create(FrameSpec)`, then
feeds events and asks the generator to render.

## ABI-safe types

Use the `abi_stable` types in every exported trait or root-module function:

| Native idea | ABI type |
| --- | --- |
| text | `RString` / `RStr` |
| owned list | `RVec<T>` |
| optional value | `ROption<T>` |
| fallible result | `RResult<T, RString>` |
| borrowed events or bytes | `RSlice<'_, T>` |
| mutable output bytes | `RSliceMut<'_, u8>` |
| owned trait object | generated `*_TO` wrapped in `RBox` |

Native `Vec`, `String`, channels, mutexes, and ordinary trait objects may be
used internally, but they must not appear in the ABI surface. A borrowed
`RSlice` is valid only during the call; copy it if it must outlive the callback.

## Facilities are capabilities

A device advertises capabilities with `PluginFacilityType` and returns a
type-erased `PluginFacilityHandle` for callable facilities. Only advertise
what is implemented. The most common capabilities are `Geometry`,
`EventSubscription`, `RawEventStream`, `Index`, `Seek`, and `Roi`.

The older `DevicePlugin` lifecycle methods remain ABI shims for compatibility.
New code should implement and expose the corresponding facility handles, while
still supplying required legacy methods until the host API removes them.

## Ownership and callbacks

The plugin owns its decoder and device state. Event subscriptions pass a sink
into the plugin; the plugin invokes `on_cd_events` and
`on_ext_events`. The callback does not transfer ownership of the slice. A
plugin should define its backpressure policy explicitly: synchronous delivery,
a bounded queue, or a dropped-batch policy are all possible, but unbounded
queues are usually unsafe for live sources.
