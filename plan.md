# Lucid Triton2 EVS Plugin — Integration Plan

## Overview

Create an external ABI-stable device plugin that connects a LUCID Triton2 EVS
event camera to OpenEVT via the viva-genicam fork at
`github.com/richbai90/viva-genicam:feat/lucid-evt-support`.

The plugin discovers Lucid EVS cameras over GigE Vision, opens a GVSP stream,
receives raw EVT3 blocks via the new `StreamBlock::Evs(EvsBlock)` payload type,
decodes them with OpenEVT's `Evt3Decoder`, and delivers `EventCD` batches
through the standard `EventBatchSink` ABI callback.

## Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Location | New workspace member `openevt-plugin-lucid-evs/` | Clean external ABI boundary matching Layout B in the plugin dev guide |
| Async bridge | Embedded tokio runtime + background thread | viva-genicam is fully async (tokio); plugin ABI is sync. Single-thread runtime spawned on init, driving a channel bridge |
| Discovery | Auto-discover via GVCP, filter by `manufacturer` matching `"LUCID"` | `viva_gige::gvcp::discover()` returns `DeviceInfo` with manufacturer field. Expose camera IPs as discoverable devices |
| EVT3 decoding | In-plugin via `openevt::hal::decoders::evt3::Evt3Decoder` | Matches existing raw-file plugin pattern, delivers decoded `EventCD` through sink |
| Configuration schema | TOML schema accepts optional `frame_timeout_ms`, defaults fine for most setups | No mandatory config needed — camera connects immediately on open |
| Dependencies | viva-genicam via git ref, openevt with `plugins` feature, abi_stable | Aligns `abi_stable` version across both |

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                  openevt-plugin-lucid-evs                 │
│                                                          │
│  ┌──────────────────┐    ┌────────────────────────────┐  │
│  │ LucidEvsDiscovery │───▶│ LucidEvsDevice             │  │
│  │  - discover()     │    │  - start_events()          │  │
│  │  - config_schema()│    │  - load_batch()            │  │
│  └──────────────────┘    │  - get_facilities()         │  │
│                          │  - get_facility_handle()    │  │
│                          └───────────┬────────────────┘  │
│                                      │                    │
│                          ┌───────────▼────────────────┐  │
│                          │   CameraBridge (internal)   │  │
│                          │  - tokio runtime            │  │
│                          │  - async worker thread      │  │
│                          │  - viva-genicam connect     │  │
│                          │  - FrameStream::next_block() │  │
│                          │  - Evt3Decoder dispatch      │  │
│                          └────────────────────────────┘  │
└──────────────────────────────────────────────────────────┘

         │                                     │
         │ abi_stable ABI                      │ internal
         ▼                                     ▼
┌──────────────────┐              ┌──────────────────────────┐
│  OpenEVT Host    │              │  viva-genicam (git dep)   │
│  Layer           │              │  feat/lucid-evt-support   │
│  PluginRegistry  │              │  - GigeDevice            │
│  DevicePlugin    │              │  - FrameStream           │
│  EventBatchSink  │              │  - StreamBlock::Evs      │
│  Evt3Decoder     │              │  - GVCP discovery        │
└──────────────────┘              └──────────────────────────┘
```

## Files to Create

### `openevt-plugin-lucid-evs/Cargo.toml`

```toml
[package]
name = "openevt-plugin-lucid-evs"
version = "0.1.0"
edition = "2024"

[dependencies]
abi_stable = "0.11.3"
openevt = { version = "1.0.4", default-features = false, features = [
    "devices",
    "plugins",
    "library-link",
], path = "../" }
viva-genicam = { git = "https://github.com/richbai90/viva-genicam", branch = "feat/lucid-evt-support" }
tokio = { version = "1", features = ["rt", "macros", "sync", "time", "net"] }
crossbeam = "0.8"
bytes = "1"
tracing = "0.1"

[lib]
crate-type = ["cdylib"]
```

### `openevt-plugin-lucid-evs/src/lib.rs`

Root module with `export_root_module` and the three ABI pieces:

- `plugin_name()` → `"lucid_triton_evs"`
- `create_discovery()` → `LucidEvsDiscoveryBox`
- `DevicePluginModuleVtable` → leak into prefix

### `openevt-plugin-lucid-evs/src/discovery.rs`

- Implements `DeviceDiscoveryPlugin`
- Calls `viva_gige::gvcp::discover()` in `discover()`
- Filters `DeviceInfo::manufacturer` containing `"LUCID"` (case-insensitive)
- Returns `PluginCameraDescriptionAbi` with IP as serial and `ConnectionType::Usb`
- `configuration_schema()` returns TOML with optional `frame_timeout_ms` parameter
- `open_device_with_configuration()` creates `LucidEvsDevice` and spawns the camera bridge

### `openevt-plugin-lucid-evs/src/device.rs`

The main device struct `LucidEvsDevice`:

**Fields:**
- `serial: RString` — camera IP
- `bridge: Arc<Mutex<CameraBridge>>` — shared bridge state
- `cd_receiver: Option<Receiver<Vec<EventCD>>>` — crossbeam channel for CD events
- `ext_receiver: Option<Receiver<Vec<EventExtTrigger>>>` — (unused, reserved)
- `sink: Option<EventBatchSinkBox>` — ABI callback

**`CameraBridge` (internal):**
- Owns the tokio runtime handle
- Background thread connects via `viva_genicam::connect_gige()`
- Opens stream via `camera.open_stream()` → `FrameStream`
- Calls `next_block()` in a loop
- On `StreamBlock::Evs(block)`: feeds raw EVT3 bytes to `Evt3Decoder`
- Dispatches decoded `EventCD` onto a crossbeam channel
- Also sends `EventExtTrigger` if any appear (ext decoder not yet wired)

**`DevicePlugin` impl:**
- `serial()`, `connection_type()` → `ConnectionType::Usb`
- `get_facilities()` → `[Geometry, Index, EventSubscription]`
- `get_facility_handle()` → geometry from camera nodemap, index from decoder
- `start_events(sink)`: subscribes to the crossbeam channel, stores sink
- `load_batch()`: non-blocking drain from crossbeam receiver, call sink
- `start_external_triggers()`: future reserved

**Facility handles:**
- `PluginGeometryFacility` — reads width/height from camera nodemap
- `PluginIndexFacility` — delegates t_min/t_max from decoder state
- `PluginEventSubscriptionFacility` — wire up CD event subscription

### Bridging Async → Sync

The critical design detail: viva-genicam's `connect_gige()` and
`FrameStream::next_block()` are async. The plugin ABI is entirely sync.

Solution:
1. On device open, spawn a background thread with a `tokio::runtime::Runtime`
   (single-threaded, `rt` feature only).
2. The runtime runs the connect + stream loop; decoded EVT3 events are sent
   over a `crossbeam::channel::Sender<Vec<EventCD>>` (bounded, never blocks).
3. `load_batch()` receives from the `crossbeam::channel::Receiver<Vec<EventCD>>`
   and passes each batch to the stored `EventBatchSink::on_cd_events()`.
4. On device drop, an `AtomicBool` shutdown flag signals the thread to exit.

This is exactly the pattern used by the simulator plugin (FFmpeg decode
thread → crossbeam channel → sync `load_batch`).

## Files to Modify

### Root `Cargo.toml`

Add to workspace members:

```toml
[workspace]
members = [".", "openevt-plugins", "openevt-plugin-lucid-evs"]
```

## Test Plan

1. **Unit tests** (in `device.rs`):
   - `camera_bridge_constructor_creates_channel_pair()`
   - `device_serial_returns_discovered_ip()`
   - `device_connection_type_is_usb()`

2. **Integration test** (in `tests/discovery_integration.rs`):
   - Load via `PluginRegistry::load_file()`
   - Verify `discover()` is non-empty when a Lucid camera is on the network
   - Verify `configuration_schema()` parses

3. **Offline mock** (future, no hardware):
   - Use `viva-fake-gige` to simulate a Lucid EVS camera
   - Verify stream flow end-to-end

## Edge Cases / Failure Modes

| Scenario | Behavior |
|---|---|
| No Lucid cameras on network | `discover()` returns empty vec |
| Camera disconnected mid-stream | `next_block()` returns error → `load_batch()` returns ABI error |
| Camera sends image frames instead of EVS | Filter by `pixel_format`; non-EVS blocks are skipped with a warning |
| Plugin loaded while target network is unreachable | Discovery returns empty; opening specific IP fails with ABI error |
| Multiple Lucid cameras | Each IP is a separate discoverable device with its own serial |
| `abi_stable` version mismatch | Root module compatibility check rejects at load time |
