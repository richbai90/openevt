//! Error types used by the HAL and decoder layers.
//!
//! The error taxonomy is layered:
//!
//! - low-level `StreamError` and `DecoderProtocolViolation` values describe
//!   local failures
//! - `DecoderError`, `HardwareError`, and `ProcessingError` wrap those failures
//!   in subsystem-specific contexts
//! - `FacilityError` in `facilities.rs` aggregates them for the public facility API
//! - `SharedError` erases the concrete error type for threaded dispatch

use std::{error::Error, io, sync::Arc};
use thiserror::Error;

/// Shared error type for threaded dispatch.
pub type SharedError = Arc<dyn Error + Send + Sync>;

/// HAL status codes that can be surfaced through the public APIs.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum HALErrorCode {
    #[error("Camera error")]
    CameraError = 0x100000,
    #[error("Failed initialization")]
    FailedInitialization = 0x101000,
    #[error("Camera not found")]
    CameraNotFound = 0x101001,
    #[error("Golden fallback booted")]
    GoldenFallbackBooted = 0x101002,
    #[error("Internal initialization error")]
    InternalInitializationError = 0x101100,
    #[error("Invalid argument")]
    InvalidArgument = 0x102000,
    #[error("Value out of range")]
    ValueOutOfRange = 0x102001,
    #[error("Non existing value")]
    NonExistingValue = 0x102002,
    #[error("Operation not permitted")]
    OperationNotPermitted = 0x102003,
    #[error("Unsupported value")]
    UnsupportedValue = 0x102004,
    #[error("Deprecated function called")]
    DeprecatedFunctionCalled = 0x103000,
    #[error("Operation not implemented")]
    OperationNotImplemented = 0x104000,
    #[error("Maximum retries exceeded")]
    MaximumRetriesExceeded = 0x105000,
}

/// Decoder protocol violations detected while parsing raw event words.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum DecoderProtocolViolation {
    #[error("Unsupported Word {0}")]
    UnsupportedWord(u16),
    #[error("Null protocol violation")]
    NullProtocolViolation,
    #[error("Non-monotonic time high violation")]
    NonMonotonicTimeHigh,
    #[error("Partial vector 12_12_8 violation")]
    PartialVect,
    #[error("Partial continued 12_12_4 violation")]
    PartialContinued,
    #[error("Non continuous time high violation")]
    NonContinuousTimeHigh,
    #[error("Missing Y address violation")]
    MissingYAddr,
    #[error("Invalid vector base violation")]
    InvalidVectBase,
    #[error("Out of bounds event coordinate violation")]
    OutOfBoundsEventCoordinate,
}

/// Stream-level errors raised by raw input sources.
#[derive(Error, Debug)]
pub enum StreamError {
    #[error("End of file reached")]
    EndOfFile,
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),
    #[error("Stream is disconnected")]
    Disconnected,
}

/// Errors that occur while decoding a raw event stream.
#[derive(Error, Debug)]
pub enum DecoderError {
    #[error("Protocol violation: {0}")]
    ProtocolViolation(#[from] DecoderProtocolViolation),
    #[error("Corrupt frame at offset {offset}")]
    CorruptFrame { offset: u64 },
    #[error("Stream read failure: {0}")]
    StreamError(#[from] StreamError),
    #[error("HAL status error: {0}")]
    HalStatus(#[from] HALErrorCode),
}

/// Errors produced by hardware register or identification operations.
#[derive(Error, Debug)]
pub enum HardwareError {
    #[error("HAL status error: {0}")]
    HalStatus(#[from] HALErrorCode),
    #[error("Register read failed at {register:#X}")]
    RegisterRead { register: u32 },
}

/// Errors produced while applying or validating higher-level processing state.
#[derive(Error, Debug)]
pub enum ProcessingError {
    #[error("HAL status error: {0}")]
    HalStatus(#[from] HALErrorCode),
    #[error("Invalid configuration: {parameter}")]
    InvalidConfiguration { parameter: String },
}
