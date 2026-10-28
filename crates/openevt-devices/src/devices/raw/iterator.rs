//! Batch-oriented iterator helpers for decoded CD events.
//!
//! `EventWindowIterator` sits on top of `RawFileReader` and lets callers consume
//! decoded events either as fixed-size batches or as time windows. Two modes are
//! available:
//!
//! - `IterSync` actively loads more raw data when the decoded queue runs dry.
//! - `IterAsync` assumes another thread is feeding the decoder and only drains
//!   the receiver.

use std::{
    marker::PhantomData,
    sync::{Arc, RwLock},
};

use crossbeam::channel::Receiver;
use openevt_core::{
    hal::facilities::{RawEventStreamDecoderFacility, RawEventStreamFacility},
    types::{EventCD, EventCount, EventTimestamp},
};

use crate::{device_types::DeviceFileError, raw::stream::RawEventStream};

/// Type-state marker for an iterator that loads raw data synchronously.
pub struct IterSync;
/// Type-state marker for an iterator fed by another thread.
pub struct IterAsync;
/// Type-state marker for an iterator not yet configured for a mode.
pub struct IterUnconfigured;

/// Consumes decoded CD events in batch or time-window form.
pub struct EventWindowIterator<const BUFFER_SIZE: usize, State = IterUnconfigured> {
    stream_handle: Arc<RwLock<dyn RawEventStreamFacility + Send + 'static>>,
    decoder_handle: Arc<RwLock<dyn RawEventStreamDecoderFacility + Send + 'static>>,
    receiver: Receiver<Vec<EventCD>>,
    // Holds the decoded batch that has not been consumed yet. Consumption is a
    // cursor move, so events are never shifted or copied one at a time.
    internal_buffer: Vec<EventCD>,
    internal_cursor: usize,
    // Tracks the current temporal baseline for slicing fixed delta-t windows
    current_timestamp: Option<EventTimestamp>,
    shape: (u32, u32),
    _state: PhantomData<State>,
}

impl<const BUFFER_SIZE: usize> EventWindowIterator<BUFFER_SIZE, IterUnconfigured> {
    /// Creates an iterator in the unconfigured state.
    ///
    /// Call `into_sync` or `into_async` before using batch/window methods.
    pub fn new(
        receiver: Receiver<Vec<EventCD>>,
        shape: (u32, u32),
        stream_handle: Arc<RwLock<dyn RawEventStreamFacility + Send + 'static>>,
        decoder_handle: Arc<RwLock<dyn RawEventStreamDecoderFacility + Send + 'static>>,
    ) -> Self {
        Self {
            stream_handle,
            receiver,
            internal_buffer: Vec::new(),
            internal_cursor: 0,
            current_timestamp: None,
            shape,
            decoder_handle,
            _state: PhantomData,
        }
    }

    /// Converts the iterator into synchronous mode.
    pub fn into_sync(self) -> EventWindowIterator<BUFFER_SIZE, IterSync> {
        EventWindowIterator {
            stream_handle: self.stream_handle,
            decoder_handle: self.decoder_handle,
            receiver: self.receiver,
            internal_buffer: self.internal_buffer,
            internal_cursor: self.internal_cursor,
            current_timestamp: self.current_timestamp,
            shape: self.shape,
            _state: PhantomData,
        }
    }

    /// Converts the iterator into asynchronous mode.
    pub fn into_async(self) -> EventWindowIterator<BUFFER_SIZE, IterAsync> {
        EventWindowIterator {
            stream_handle: self.stream_handle,
            decoder_handle: self.decoder_handle,
            receiver: self.receiver,
            internal_buffer: self.internal_buffer,
            internal_cursor: self.internal_cursor,
            current_timestamp: self.current_timestamp,
            shape: self.shape,
            _state: PhantomData,
        }
    }
}

/// Supplies decoded events to an [`EventWindowIterator`] when its buffer runs dry.
pub trait BufferReplenisher {
    /// Ensures there are decoded events available in the internal buffer.
    ///
    /// Returns `true` when events are available and `false` when the stream
    /// has ended. Real failures are returned as `Err`.
    fn replenish_buffer(&mut self) -> Result<bool, DeviceFileError>;
}

// ASYNC MODE: Waits for an external thread to feed the channel.
impl<const BUFFER_SIZE: usize> BufferReplenisher for EventWindowIterator<BUFFER_SIZE, IterAsync> {
    fn replenish_buffer(&mut self) -> Result<bool, DeviceFileError> {
        if self.remaining() == 0 {
            self.drain_channel_once();
        }
        Ok(self.remaining() > 0)
    }
}

// SYNC MODE: Loads the batch locally first, then consumes the channel
impl<const BUFFER_SIZE: usize> BufferReplenisher for EventWindowIterator<BUFFER_SIZE, IterSync> {
    fn replenish_buffer(&mut self) -> Result<bool, DeviceFileError> {
        while self.remaining() == 0 {
            if self.try_drain_channel_once() {
                break;
            }
            if self.load_batch()?.is_none() {
                return Ok(false);
            }
        }
        Ok(self.remaining() > 0)
    }
}

// Generic implementation block for ALL states
impl<const BUFFER_SIZE: usize, State> EventWindowIterator<BUFFER_SIZE, State> {
    /// Returns the sensor shape associated with the underlying reader.
    pub fn shape(&self) -> (u32, u32) {
        self.shape
    }

    /// Number of decoded events not yet consumed from the current batch.
    fn remaining(&self) -> usize {
        self.internal_buffer.len() - self.internal_cursor
    }

    /// Internal logic for reading channel data into `internal_buffer`.
    ///
    /// The batch is taken from the channel by move, so no event bytes are
    /// copied here. Returns `false` when the channel is disconnected.
    fn drain_channel_once(&mut self) -> bool {
        match self.receiver.recv() {
            Ok(events) => {
                self.internal_buffer = events;
                self.internal_cursor = 0;
                true
            }
            Err(_) => false,
        }
    }

    /// Drains one already-decoded batch without blocking.
    ///
    /// Sync iterators must check the receiver before loading more raw data because
    /// a single raw buffer can decode into several pooled event batches.
    fn try_drain_channel_once(&mut self) -> bool {
        match self.receiver.try_recv() {
            Ok(events) => {
                self.internal_buffer = events;
                self.internal_cursor = 0;
                true
            }
            Err(_) => false,
        }
    }

    /// Returns the next batch of decoded CD events.
    ///
    /// The batch is capped at the iterator buffer size. If the stream ends after
    /// some events were already collected, those events are still returned.
    /// `Ok(None)` means the stream has ended with no events left.
    pub fn next_batch(&mut self) -> Result<Option<Vec<EventCD>>, DeviceFileError>
    where
        Self: BufferReplenisher,
    {
        self.next_up_to(BUFFER_SIZE)
    }

    /// Returns the next `n` decoded CD events, or fewer if the stream ends.
    ///
    /// The requested count must be smaller than the iterator buffer size.
    /// Events beyond `n` remain available for the next call.
    /// `Ok(None)` means the stream has ended with no events left.
    pub fn next_n(&mut self, n: EventCount) -> Result<Option<Vec<EventCD>>, DeviceFileError>
    where
        Self: BufferReplenisher,
    {
        let limit = usize::try_from(n)
            .expect("requested event count does not fit this platform's allocation size");
        assert!(
            limit < BUFFER_SIZE,
            "requested event count must be smaller than the iterator buffer size"
        );
        self.next_up_to(limit)
    }

    fn next_up_to(&mut self, limit: usize) -> Result<Option<Vec<EventCD>>, DeviceFileError>
    where
        Self: BufferReplenisher,
    {
        let mut batch = Vec::with_capacity(limit);

        while batch.len() < limit {
            if self.remaining() == 0 {
                if !self.replenish_buffer()? {
                    break; // Stream ended
                }
            }

            if batch.is_empty() && self.current_timestamp.is_none() {
                self.current_timestamp = Some(self.internal_buffer[self.internal_cursor].t);
            }

            let take = (limit - batch.len()).min(self.remaining());
            let start = self.internal_cursor;
            batch.extend_from_slice(&self.internal_buffer[start..start + take]);
            self.internal_cursor += take;
        }

        Ok((!batch.is_empty()).then_some(batch))
    }

    /// Returns the next window of CD events that fall within `dt` from the
    /// current timestamp baseline.
    ///
    /// The iterator advances its internal time baseline by `dt` after each call.
    /// `Ok(None)` means the stream has ended with no events left.
    pub fn next_delta(&mut self, dt: EventTimestamp) -> Result<Option<Vec<EventCD>>, DeviceFileError>
    where
        Self: BufferReplenisher,
    {
        if self.remaining() == 0 && !self.replenish_buffer()? {
            return Ok(None);
        }

        let start_ts = match self.current_timestamp {
            Some(ts) => ts,
            None => {
                if let Some(first_ev) = self.internal_buffer.get(self.internal_cursor) {
                    let ts = first_ev.t;
                    self.current_timestamp = Some(ts);
                    ts
                } else {
                    return Ok(None);
                }
            }
        };

        let end_ts = start_ts + dt;
        let mut window_events = Vec::new();

        loop {
            if self.remaining() == 0 {
                if !self.replenish_buffer()? {
                    break;
                }
            }

            // Decoded batches are timestamp-ordered, so the window is a prefix.
            let rest = &self.internal_buffer[self.internal_cursor..];
            let take = rest.partition_point(|event| event.t < end_ts);
            if take == 0 {
                break;
            }
            window_events.extend_from_slice(&rest[..take]);
            self.internal_cursor += take;
        }

        self.current_timestamp = Some(end_ts);
        Ok((!window_events.is_empty()).then_some(window_events))
    }
}

impl<const BUFFER_SIZE: usize> EventWindowIterator<BUFFER_SIZE, IterSync> {
    /// Loads and decodes one raw buffer into the iterator's event channel.
    ///
    /// `Ok(None)` means the stream reached its end.
    pub fn load_batch(&mut self) -> Result<Option<()>, DeviceFileError> {
        let mut stream_facility = self
            .stream_handle
            .as_ref()
            .try_write()
            .map_err(|_| DeviceFileError::LockError)?;

        let mut decoder_facility = self
            .decoder_handle
            .as_ref()
            .try_write()
            .map_err(|_| DeviceFileError::LockError)?;

        let stream = crate::facility_downcast_mut!(stream_facility, RawEventStream<BUFFER_SIZE>)?;
        match stream.fill_buffer()? {
            Some(buffer) => {
                decoder_facility.decode(buffer)?;
                Ok(Some(()))
            }
            None => Ok(None),
        }
    }
}
