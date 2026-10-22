# Creating a frame-generation plugin

Frame-generation plugins turn decoded CD events into images. They are not
device plugins and do not discover cameras. The contract is in
[`crates/openevt-abi/src/framegen.rs`](../crates/openevt-abi/src/framegen.rs).

## Create the crate

```toml
[package]
name = "example-framegen"
version = "0.1.0"
edition = "2024"

[dependencies]
abi_stable = "0.11.3"
openevt = { path = "../openevt", features = ["framegen"] }

[lib]
crate-type = ["cdylib"]
```

The host and plugin must use matching `FrameSpec`, `Color`, `EventCD`, and
`FrameGenerator` definitions. Do not duplicate these types locally.

## Implement the generator

`FrameGenerator` requires four operations:

- `colormap(t)`: optionally map a scalar to an RGB color;
- `consume(events)`: update internal state from a borrowed event batch;
- `render(current_t, out_rgba)`: fill exactly `width * height * 4` bytes;
- `reset()`: clear temporal state.

Minimal generator skeleton:

```rust
use abi_stable::{sabi_trait, std_types::{ROption, RSlice, RSliceMut}};
use openevt::{
    framegen::{FrameGenerator, FrameSpec},
    types::{EventCD, EventTimestamp},
};

pub struct Example {
    spec: FrameSpec,
    activity: Vec<u32>,
}

impl Example {
    fn new(spec: FrameSpec) -> Self {
        let pixel_count = spec
            .width
            .checked_mul(spec.height)
            .and_then(|value| value.try_into().ok())
            .expect("frame is too large for this platform");
        Self { spec, activity: vec![0; pixel_count] }
    }
}

impl FrameGenerator for Example {
    fn colormap(&self, _t: f64) -> ROption<openevt::framegen::Color> {
        ROption::RNone
    }

    fn consume(&mut self, events: RSlice<'_, EventCD>) {
        for event in events.iter() {
            let Some(index) = event
                .y
                .checked_mul(self.spec.width)
                .and_then(|row| row.checked_add(event.x))
                .and_then(|value| value.try_into().ok())
            else {
                continue;
            };
            if let Some(pixel) = self.activity.get_mut(index) {
                *pixel += 1;
            }
        }
    }

    fn render(&self, _current_t: EventTimestamp, mut out_rgba: RSliceMut<'_, u8>) {
        for (index, pixel) in self.activity.iter().enumerate() {
            let value = (*pixel).min(255) as u8;
            let offset = index * 4;
            if offset + 3 < out_rgba.len() {
                out_rgba[offset..offset + 4].copy_from_slice(&[value, value, value, 255]);
            }
        }
    }

    fn reset(&mut self) { self.activity.fill(0); }
}
```

In production, validate the output length once and define what happens to
coordinates outside the declared `FrameSpec`. A renderer should never write
past the supplied slice. For temporal renderers, interpret `current_t` as
microseconds and document whether state decays before or during `render`.

> **Current API limitation:** `Color` is ABI-visible, but its `r`, `g`, and
> `b` fields are private in the current source. External plugins can safely
> return `ROption::RNone` from `colormap`, as the example does, but cannot
> currently construct a color. If a plugin needs colormap support, add a
> public constructor or public channel fields to `openevt::framegen::Color`
> and keep that change synchronized between host and plugin builds.

## Export the root module

The loader expects the root name `frame_generator_plugin` and a constructor
that receives the requested frame size:

```rust
use abi_stable::{export_root_module, std_types::RString,
    type_level::downcasting::TD_Opaque};
use openevt::framegen::{FrameGeneratorBox, FrameSpec, PluginModuleRef,
    PluginModuleVtable};

extern "C" fn name() -> RString { "example_framegen".into() }

extern "C" fn create(spec: FrameSpec) -> FrameGeneratorBox {
    openevt::framegen::FrameGenerator_TO::from_value(
        Example::new(spec), TD_Opaque,
    )
}

#[export_root_module]
pub fn instantiate_root_module() -> PluginModuleRef {
    PluginModuleVtable { name, create }.leak_into_prefix()
}
```

The generated `FrameGenerator_TO` type is the ABI-safe trait object. Keep the
generator `Send + Sync`; the trait contract requires it even though mutation
is performed through `&mut self`.

## Rendering contract

`FrameSpec::rgba_buffer_size()` is the authoritative buffer size. The format is
packed RGBA, one byte per channel, row-major, with four bytes per pixel. The
host owns the output buffer and may reuse it between calls. A generator should
overwrite every pixel on every render, including pixels with no recent events,
so stale image data cannot leak between frames.

## Testing a generator

Test the pure generator without loading a dynamic library first: construct a
small `FrameSpec`, feed known events, render into a zeroed buffer, and assert
pixel values. Add tests for reset, empty batches, repeated rendering, boundary
coordinates, and undersized buffers. Then build the `cdylib` and test loading
the root module from a separate host process.
