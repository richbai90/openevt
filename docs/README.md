# OpenEVT plugin development

OpenEVT plugins are dynamic Rust libraries that connect the host crate to an
event source or an event visualization pipeline. This book documents the
contracts in this repository, the ABI rules behind them, and the workflow for
building a plugin that another OpenEVT application can discover and load.

There are two independent plugin families:

| Plugin | Input | Output | ABI root |
| --- | --- | --- | --- |
| Device plugin | A camera, file, simulator, or network source | Device metadata, facilities, and CD/trigger batches | `openevt_device_plugin` |
| Frame generator | Decoded `EventCD` batches | Caller-owned RGBA image buffers | `frame_generator_plugin` |

Start with [Architecture and mental model](architecture.md), then choose the
[device plugin](plugin-development-guide.md) or
[frame-generation plugin](framegen-plugin.md) guide. The
[video simulator](simulator-plugin.md) is a complete working device plugin
that is especially useful when implementing a new adapter.

## Repository map

The root `openevt` crate is a facade that re-exports the workspace crates: the
public HAL and types live in `crates/openevt-core`, the raw-file device in
`crates/openevt-devices`, and both ABI contracts in `crates/openevt-abi`. The
`openevt-plugins` crate ships the standard device plugins — raw-file replay
and the video simulator — in one library, with optional Python bindings for
the simulator. The `openevt-plugin-lucid-evs` crate adds a network device
plugin for LUCID GigE Vision EVT cameras.

The contracts are deliberately small. Keep vendor SDKs, decoders, threads,
channels, locks, and mutable buffers inside the plugin; only ABI-safe values
cross the dynamic-library boundary.

## Compatibility rule

Build the plugin and host against compatible versions of `openevt`,
`abi_stable`, and the Rust ABI definitions. A plugin is not a generic shared
object: it must export the exact root-module contract the loader expects.
