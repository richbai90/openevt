//! ABI-stable interfaces for converting event streams into image frames.
//!
//! A frame-generator plugin receives decoded CD events, maintains whatever
//! temporal state its modality requires, and renders into a caller-owned RGBA
//! buffer. The module is intentionally small so generators can be implemented
//! in separate dynamic libraries.

use abi_stable::{
    StableAbi,
    library::RootModule,
    package_version_strings, sabi_trait,
    sabi_types::VersionStrings,
    std_types::{ROption, RSlice, RSliceMut, RString},
};

use openevt_core::types::{EventCD, EventTimestamp};

/// Frame dimensions and pixel format metadata.
/// An RGB color returned by a frame-generator colormap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi)]
#[repr(C)]
pub struct FrameSpec {
    /// Frame width in pixels.
    pub width: u64,
    /// Frame height in pixels.
    pub height: u64,
}

impl FrameSpec {
    /// Returns the number of bytes required for an RGBA frame.
    pub fn rgba_buffer_size(&self) -> u64 {
        self.width * self.height * 4
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi)]
#[repr(C)]
pub struct Color {
    /// Red channel value.
    r: u8,
    /// Green channel value.
    g: u8,
    /// Blue channel value.
    b: u8,
}

/// Abstract interface for event-based frame generation modalities.
#[sabi_trait]
pub trait FrameGenerator: Send + Sync {
    /// Given a value t, returns an Optional color value.
    fn colormap(&self, t: f64) -> ROption<Color>;
    /// Ingests a stream of events into the internal generator state.
    fn consume(&mut self, events: RSlice<'_, EventCD>);

    /// Renders the accumulated state into a target RGBA buffer.
    ///
    /// `current_t`: Reference timestamp (in microseconds) for temporal modalities.
    /// `out_rgba`: Pre-allocated buffer of length `width * height * 4`.
    fn render(&self, current_t: EventTimestamp, out_rgba: RSliceMut<'_, u8>);

    /// Resets or clears internal state buffers.
    fn reset(&mut self);
}

/// Standard trait object wrapper for dynamic dispatch.
pub type FrameGeneratorBox = FrameGenerator_TO<'static, abi_stable::std_types::RBox<()>>;

/// The root module symbol exported by all dynamic plugins.
#[derive(StableAbi)]
#[repr(C)]
#[sabi(kind(Prefix(prefix_ref = PluginModuleRef)))]
pub struct PluginModuleVtable {
    /// Friendly name of the generator plugin modality.
    pub name: extern "C" fn() -> RString,

    /// Constructor function to instantiate the generator.
    pub create: extern "C" fn(spec: FrameSpec) -> FrameGeneratorBox,
}

impl RootModule for PluginModuleRef {
    abi_stable::declare_root_module_statics! {PluginModuleRef}

    const BASE_NAME: &'static str = "frame_generator_plugin";
    const NAME: &'static str = "frame_generator_plugin";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();
}
