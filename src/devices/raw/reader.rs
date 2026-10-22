//! High-level API for reading raw event files.
//!
//! `RawFileReader` is the high-level convenience API for reading raw files.
//! It opens a file, wires up the stream and decoder facilities, and provides helper
//! methods for:
//!
//! - subscribing to decoded CD or external-trigger batches
//! - loading decoded batches synchronously
//! - seeking by timestamp when an index is available
//! - exposing the parsed file time range

use crate::EventWindowIterator;
use crate::raw::decoder::RawEventStreamDecoder;
use crate::raw::device::RawFileHandler;
use crate::raw::stream::RawEventStream;
use crate::raw::{IterUnconfigured, index};
use crate::types::EventTimestamp;
use crate::device_types::{DeviceFileError, FileIndex};
use crossbeam::channel::Receiver;
use openevt_core::hal::device::device::Device;
use openevt_core::hal::facilities::{
    EventSubscriptionFacilityHandle, FacilityError, FacilityType, ROIFacilityHandle,
    RawEventStreamDecoderFacilityHandle, RawEventStreamFacility, RawEventStreamFacilityHandle,
};
use openevt_core::types::{EventCD, EventExtTrigger};
use std::sync::Arc;

/// Reads and decodes an event-camera raw file.
pub struct RawFileReader<const BUFFER_SIZE: usize> {
    _device: RawFileHandler<BUFFER_SIZE>,
    stream_handle: Option<RawEventStreamFacilityHandle>,
    decoder_handle: Option<RawEventStreamDecoderFacilityHandle>,
    event_decoder_handle: Option<EventSubscriptionFacilityHandle>,
    index: Option<Arc<FileIndex>>,
    initialized: bool,
}

impl<const BUFFER_SIZE: usize> RawFileReader<BUFFER_SIZE> {
    /// Returns `true` when the reader has an active stream handle.
    pub fn ready(&self) -> bool {
        self.stream_handle.is_some()
    }
}

impl<const BUFFER_SIZE: usize> RawFileReader<BUFFER_SIZE> {
    /// Creates an empty reader that has not opened a file yet.
    pub fn new() -> Self {
        let device = RawFileHandler::<BUFFER_SIZE>::new();
        Self {
            _device: device,
            stream_handle: None,
            decoder_handle: None,
            event_decoder_handle: None,
            index: None,
            initialized: false,
        }
    }

    /// Opens a raw file and builds the internal handles.
    ///
    /// If `do_index` is `true`, a timestamp index is also built so `seek` and
    /// `seek_to_next_ext` can operate efficiently.
    pub fn try_from_file(file_path: &str, do_index: bool) -> Result<Self, DeviceFileError> {
        let device = RawFileHandler::<BUFFER_SIZE>::new_from_path(file_path)?;

        let index = match do_index {
            true => Some(Arc::new(index::build_index(
                file_path,
                device.header_end_pos(),
                1024 * 1024,
            )?)),

            false => None,
        };

        let stream_handle: Option<RawEventStreamFacilityHandle> = Some(
            device
                .get_facility(FacilityType::RawEventStreamFacility)
                .ok_or(DeviceFileError::UnsupportedFacility(
                    "RawEventStreamFacility".to_string(),
                ))?
                .try_into()?,
        );

        let decoder_handle: Option<RawEventStreamDecoderFacilityHandle> = Some(
            device
                .get_facility(FacilityType::RawEventStreamDecoderFacility)
                .ok_or(DeviceFileError::UnsupportedFacility(
                    "RawEventStreamDecoderFacility".to_string(),
                ))?
                .try_into()?,
        );

        let event_decoder_handle: Option<EventSubscriptionFacilityHandle> = Some(
            device
                .get_facility(FacilityType::EventSubscriptionFacility)
                .ok_or(DeviceFileError::UnsupportedFacility(
                    "EventSubscriptionFacility".to_string(),
                ))?
                .try_into()?,
        );

        Ok(Self {
            _device: device,
            stream_handle,
            decoder_handle,
            event_decoder_handle,
            index,
            initialized: true,
        })
    }

    /// Replaces the current file contents with a new file.
    ///
    /// This updates the device, facilities, and optional index in place.
    pub fn try_open(&mut self, file_path: &str, do_index: bool) -> Result<(), DeviceFileError> {
        self._device.try_open(file_path)?;
        let device = &self._device;

        self.index = match do_index {
            true => Some(Arc::new(index::build_index(
                file_path,
                device.header_end_pos(),
                1024 * 1024,
            )?)),

            false => None,
        };

        self.stream_handle = Some(
            device
                .get_facility(FacilityType::RawEventStreamFacility)
                .ok_or(DeviceFileError::UnsupportedFacility(
                    "RawEventStreamFacility".to_string(),
                ))?
                .try_into()?,
        );

        self.decoder_handle = Some(
            device
                .get_facility(FacilityType::RawEventStreamDecoderFacility)
                .ok_or(DeviceFileError::UnsupportedFacility(
                    "RawEventStreamDecoderFacility".to_string(),
                ))?
                .try_into()?,
        );

        self.event_decoder_handle = Some(
            device
                .get_facility(FacilityType::EventSubscriptionFacility)
                .ok_or(DeviceFileError::UnsupportedFacility(
                    "EventSubscriptionFacility".to_string(),
                ))?
                .try_into()?,
        );

        self.initialized = true;

        Ok(())
    }

    /// Returns the minimum timestamp recorded in the optional index.
    pub fn t_min(&self) -> Option<EventTimestamp> {
        if let Some(index) = &self.index {
            return Some(index.t_min);
        }

        None
    }

    /// Returns the maximum timestamp recorded in the optional index.
    pub fn t_max(&self) -> Option<EventTimestamp> {
        if let Some(index) = &self.index {
            return Some(index.t_max);
        }

        None
    }

    /// Seeks to the timestamp nearest the requested position.
    ///
    /// This requires an index. The stream and decoder state are repositioned so
    /// the next decode call resumes from the chosen marker.
    pub fn seek(&mut self, ts: EventTimestamp) -> Result<(), DeviceFileError> {
        if !self.initialized {
            return Err(DeviceFileError::NotInitialized);
        }
        let index = self
            .index
            .clone()
            .ok_or(DeviceFileError::UnsupportedBehavior(
                "File must be indexed in order to use seek.".to_string(),
            ))?;
        let stream_handle = self.get_stream_handle()?;
        let decoder_handle = self.get_decoder_handle()?;
        let mut stream_facility = stream_handle
            .as_ref()
            .try_write()
            .map_err(|_| DeviceFileError::LockError)?;
        let mut decoder_facility = decoder_handle
            .as_ref()
            .try_write()
            .map_err(|_| DeviceFileError::LockError)?;

        let stream = crate::facility_downcast_mut!(stream_facility, RawEventStream<BUFFER_SIZE>)?;
        let decoder = crate::facility_downcast_mut!(decoder_facility, RawEventStreamDecoder)?;

        index::seek_to_timestamp(index.as_ref(), ts, stream, decoder)
    }

    /// Returns the ROIFacilityHandle allowing the reader to retain
    /// state about the ROI. For the reader type this is a stateful
    /// representation. Filtering will take place down stream at consumption.
    pub fn roi(&self) -> Result<ROIFacilityHandle, DeviceFileError> {
        let roi_handle: ROIFacilityHandle = self
            ._device
            .get_facility(FacilityType::ROIFacility)
            .ok_or(DeviceFileError::UnsupportedFacility(
                "ROIFacility".to_string(),
            ))?
            .try_into()?;

        Ok(roi_handle)
    }

    /// Seeks to the timestamp of the next external trigger event.
    ///
    /// This consumes decoded batches until an external trigger appears, then
    /// uses that trigger timestamp as the seek target.
    pub fn seek_to_next_ext(&mut self) -> Result<(), DeviceFileError> {
        let recv = self.ext_receiver()?;
        loop {
            self.load_batch()?;
            match recv.try_recv() {
                Ok(evts) if !evts.is_empty() => return self.seek(evts[0].t),
                Ok(_) => continue,
                Err(err) => return Err(DeviceFileError::TryRecv(err)),
            }
        }
    }

    /// Returns a receiver for decoded CD event batches and starts the stream.
    pub fn cd_receiver(&mut self) -> Result<Receiver<Vec<EventCD>>, DeviceFileError> {
        let (sender, receiver) = crossbeam::channel::unbounded();
        let stream_handle = self.get_stream_handle()?;
        let event_decoder_handle = self.get_event_decoder_handle()?;
        let mut stream_facility = stream_handle
            .as_ref()
            .try_write()
            .map_err(|_| DeviceFileError::LockError)?;

        let mut event_decoder_facility = event_decoder_handle
            .as_ref()
            .try_write()
            .map_err(|_| DeviceFileError::LockError)?;

        let stream = crate::facility_downcast_mut!(stream_facility, RawEventStream<BUFFER_SIZE>)?;

        event_decoder_facility
            .subscribe_to_cd_events(Box::new(move |events| {
                let _ = sender.send(events.to_vec());
            }))
            .map_err(DeviceFileError::FacilityError)?;
        stream.start()?;
        Ok(receiver)
    }

    /// Returns a receiver for decoded external-trigger event batches and starts the stream.
    pub fn ext_receiver(&mut self) -> Result<Receiver<Vec<EventExtTrigger>>, DeviceFileError> {
        let (sender, receiver) = crossbeam::channel::unbounded();
        let stream_handle = self.get_stream_handle()?;
        let event_decoder_handle = self.get_event_decoder_handle()?;
        let mut stream_facility = stream_handle
            .as_ref()
            .try_write()
            .map_err(|_| DeviceFileError::LockError)?;

        let mut ext_evt_decoder_facility = event_decoder_handle
            .as_ref()
            .try_write()
            .map_err(|_| DeviceFileError::LockError)?;

        let stream = crate::facility_downcast_mut!(stream_facility, RawEventStream<BUFFER_SIZE>)?;

        ext_evt_decoder_facility
            .subscribe_to_ext_events(Box::new(move |events| {
                let _ = sender.send(events.to_vec());
            }))
            .map_err(DeviceFileError::FacilityError)?;
        stream.start()?;

        Ok(receiver)
    }

    /// Loads one raw buffer from the stream and decodes it immediately.
    pub fn load_batch(&mut self) -> Result<(), DeviceFileError> {
        let stream_handle = self.get_stream_handle()?;
        let decoder_handle = self.get_decoder_handle()?;
        let mut stream_facility = stream_handle
            .as_ref()
            .try_write()
            .map_err(|_| DeviceFileError::LockError)?;

        let mut decoder_facility = decoder_handle
            .as_ref()
            .try_write()
            .map_err(|_| DeviceFileError::LockError)?;

        let stream = crate::facility_downcast_mut!(stream_facility, RawEventStream<BUFFER_SIZE>)?;
        let (buffer, _) = stream.poll_buffer()?;
        decoder_facility.decode(&buffer)?;
        Ok(())
    }

    /// Creates an unconfigured iterator over decoded CD event windows.
    /// Configure it with [`EventWindowIterator::into_sync`] or
    /// [`EventWindowIterator::into_async`] before consuming events.
    pub fn as_windows(
        &mut self,
    ) -> Result<EventWindowIterator<BUFFER_SIZE, IterUnconfigured>, DeviceFileError> {
        let receiver = self.cd_receiver()?;
        let decoder_handle = self.get_decoder_handle()?;
        let stream_handle = self.get_stream_handle()?;
        let shape = self._device.shape();
        Ok(EventWindowIterator::<BUFFER_SIZE>::new(
            receiver,
            shape,
            stream_handle,
            decoder_handle,
        ))
    }

    /// Returns the sensor dimensions `(width, height)` from the file header.
    pub fn shape(&self) -> (u32, u32) {
        self._device.shape()
    }

    fn assert_initialized(&self) -> Result<(), DeviceFileError> {
        if !self.initialized {
            return Err(DeviceFileError::NotInitialized);
        }

        return Ok(());
    }

    fn get_stream_handle(&self) -> Result<RawEventStreamFacilityHandle, DeviceFileError> {
        let _ = self.assert_initialized()?;
        Ok(self.stream_handle.clone().unwrap())
    }

    fn get_event_decoder_handle(&self) -> Result<EventSubscriptionFacilityHandle, DeviceFileError> {
        let _ = self.assert_initialized()?;
        Ok(self.event_decoder_handle.clone().unwrap())
    }

    fn get_decoder_handle(&self) -> Result<RawEventStreamDecoderFacilityHandle, DeviceFileError> {
        let _ = self.assert_initialized()?;
        Ok(self.decoder_handle.clone().unwrap())
    }
}
