//! File-backed event stream implementation.
//!
//! This stream adapts a `File` to the `RawEventStreamFacility` interface expected
//! by the HAL. It reads fixed-size buffers and keeps track of whether the stream
//! has started and whether EOF has already been reached.

use crate::device_types::RawEventBuffer;
use openevt_core::hal::errors::StreamError;
use openevt_core::hal::facilities::{FacilityError, FacilityResult, RawEventStreamFacility};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// Buffered file stream for raw event data.
pub(crate) struct RawEventStream<const N: usize> {
    file: File,
    buffer: RawEventBuffer<N>,
    eof: bool,
    started: bool,
}

impl<const N: usize> RawEventStream<N> {
    /// Creates a new stream around an open file.
    pub(crate) fn new(file: File) -> Self {
        Self {
            file,
            buffer: RawEventBuffer::<N>::new(),
            eof: false,
            started: false,
        }
    }

    /// Seeks the underlying file to a byte offset and clears EOF state.
    pub(crate) fn seek_to_offset(&mut self, byte_offset: u64) -> Result<(), std::io::Error> {
        self.file.seek(SeekFrom::Start(byte_offset))?;
        self.eof = false;
        Ok(())
    }
}

impl<const N: usize> RawEventStreamFacility for RawEventStream<N> {
    /// Marks the stream as started.
    fn start(&mut self) -> FacilityResult<()> {
        self.started = true;
        Ok(())
    }

    /// Marks the stream as stopped.
    fn stop(&mut self) -> FacilityResult<()> {
        self.started = false;
        Ok(())
    }

    /// Polls the next buffer if the stream has started.
    fn poll_buffer(&mut self) -> FacilityResult<openevt_core::hal::facilities::StreamBuffer> {
        if !self.started {
            return Err(FacilityError::Stream(StreamError::Disconnected));
        }
        self.wait_next_buffer()
    }

    /// Reads the next buffer from disk, blocking on I/O as needed.
    fn wait_next_buffer(&mut self) -> FacilityResult<openevt_core::hal::facilities::StreamBuffer> {
        if self.eof {
            return Err(FacilityError::Stream(StreamError::EndOfFile));
        }

        match self.file.read(&mut self.buffer) {
            Ok(0) => {
                self.eof = true;
                Err(FacilityError::Stream(StreamError::EndOfFile))
            }
            Ok(bytes_read) => Ok((self.buffer[..bytes_read].to_vec(), bytes_read as u64)),
            Err(err) => {
                self.eof = true;
                Err(FacilityError::Stream(StreamError::IoError(err)))
            }
        }
    }
}
