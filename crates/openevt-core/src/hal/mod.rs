//! Hardware-abstraction-layer types and facility contracts.
//!
//! The HAL represents a device as a set of independently discoverable
//! facilities. Decoders, streams, geometry, identification, and controls can
//! therefore be implemented independently and composed by a device backend.

/// Raw event decoders and decoder traits.
pub mod decoders;
/// Device registry and discovery abstractions.
pub mod device;
/// Event and error fan-out dispatchers.
pub mod dispatcher;
/// HAL and decoder error types.
pub mod errors;
/// Facility traits, handles, and capability identifiers.
pub mod facilities;
