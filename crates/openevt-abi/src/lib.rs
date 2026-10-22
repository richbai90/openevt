//! Stable ABI contract for openevt device plugins.
//!
//! This crate isolates the unsafe FFI boundary and `abi_stable` conversions
//! from the native HAL traits in `openevt-core`. It hosts the plugin ABI
//! definitions, the host-side plugin registry/discovery loading, the
//! configuration schema parsing, and the adapter that wraps ABI plugins in the
//! native `Device` interface.
//!
//! ABI type layouts must remain unchanged across refactors or existing
//! plugin libraries lose binary compatibility.

/// Host-layer parsing and validation for plugin creation schemas.
pub mod configuration;
/// Host-layer adapter that wraps ABI plugins in the native `Device` interface.
pub mod device;
/// Plugin discovery and library loading.
pub mod discovery;
/// Stable ABI definitions shared with third-party device plugins.
pub mod plugin;
/// ABI-stable event-to-frame generation interfaces.
#[cfg(feature = "framegen")]
pub mod framegen;
