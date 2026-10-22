use crate::header::Header;
use crate::device_types::FileFormat;
use openevt_core::hal::decoders::evt3::{DecoderTimingState, Evt3Decoder};
use openevt_core::hal::decoders::raw_fmt_decoder::RawFormatDecoder;
use openevt_core::hal::facilities::{
    DecoderErrorCallback, EventCDCallback, EventExtTriggerCallback, EventSubscriptionFacility,
    FacilityError, FacilityResult, RawDecoderFacility, RawEventStreamDecoderFacility,
};
use openevt_core::types::EventTimestamp;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
/// Facility-backed decoder for the event format declared by a raw-file header.
pub struct RawEventStreamDecoder {
    inner: Arc<RwLock<Box<dyn RawFormatDecoder + Send + Sync>>>,
    /// Format selected from the input header.
    pub event_format: FileFormat,
}

impl RawEventStreamDecoder {
    /// Creates a decoder for a parsed header.
    ///
    /// `do_time_shift` controls whether decoder timestamps are normalized.
    pub fn new(header: &Header, do_time_shift: bool) -> Self {
        let decoder: Box<dyn RawFormatDecoder + Send + Sync> = match header.format {
            FileFormat::EVT3 => Box::new(Evt3Decoder::new(
                header.width as u16,
                header.height as u16,
                do_time_shift,
            )),
            FileFormat::EVT2 => todo!("Implement EVT2 Decoder"),
            FileFormat::DAT => todo!("Implement DAT Decoder"),
            FileFormat::HDF5 => todo!("Implement HDF5 Decoder"),
            FileFormat::UNKNOWN => unimplemented!("Cannot construct decoder for UNKNOWN format"),
        };

        Self {
            event_format: header.format,
            inner: Arc::new(RwLock::new(decoder)),
        }
    }

    pub(crate) fn set_evt3_timing_state(
        &mut self,
        state: DecoderTimingState,
    ) -> FacilityResult<()> {
        let mut decoder = self.inner.write().unwrap();
        let evt3_decoder = decoder
            .as_any_mut()
            .downcast_mut::<Evt3Decoder>()
            .ok_or_else(|| {
                FacilityError::FacilityDowncastError(
                    "RawFormatDecoder".to_string(),
                    "Evt3Decoder".to_string(),
                )
            })?;

        evt3_decoder.set_timing_state(state);
        Ok(())
    }
}

impl EventSubscriptionFacility for RawEventStreamDecoder {
    fn subscribe_to_cd_events(&mut self, callback: EventCDCallback) -> FacilityResult<()> {
        self.inner.write().unwrap().subscribe_to_cd_events(callback)
    }

    fn subscribe_to_ext_events(&mut self, callback: EventExtTriggerCallback) -> FacilityResult<()> {
        self.inner
            .write()
            .unwrap()
            .subscribe_to_ext_events(callback)
    }
}

impl RawEventStreamDecoderFacility for RawEventStreamDecoder {
    fn decode(&mut self, raw_data: &[u8]) -> FacilityResult<()> {
        self.inner.write().unwrap().decode(raw_data)
    }

    fn get_last_timestamp(&self) -> EventTimestamp {
        self.inner.read().unwrap().get_last_timestamp()
    }

    fn get_timestamp_shift(&self) -> Option<EventTimestamp> {
        self.inner.read().unwrap().get_timestamp_shift()
    }

    fn is_time_shifting_enabled(&self) -> bool {
        self.inner.read().unwrap().is_time_shifting_enabled()
    }

    fn reset_last_timestamp(&mut self, timestamp: EventTimestamp) {
        self.inner.write().unwrap().reset_last_timestamp(timestamp)
    }

    fn reset_timestamp_shift(&mut self, shift: EventTimestamp) {
        self.inner.write().unwrap().reset_timestamp_shift(shift)
    }

    fn is_decoded_event_stream_indexable(&self) -> bool {
        self.inner
            .read()
            .unwrap()
            .is_decoded_event_stream_indexable()
    }

    fn add_marker(&mut self, timestamp: EventTimestamp) -> slotmap::DefaultKey {
        self.inner.write().unwrap().add_marker(timestamp)
    }

    fn remove_marker(&mut self, key: slotmap::DefaultKey) -> Option<EventTimestamp> {
        self.inner.write().unwrap().remove_marker(key)
    }
}

impl RawDecoderFacility for RawEventStreamDecoder {
    fn subscribe_to_protocol_violation(
        &mut self,
        callback: DecoderErrorCallback,
    ) -> FacilityResult<()> {
        self.inner
            .write()
            .unwrap()
            .subscribe_to_protocol_violation(callback)
    }

    fn get_raw_event_size_bytes(&self) -> FacilityResult<u8> {
        self.inner.read().unwrap().get_raw_event_size_bytes()
    }
}
