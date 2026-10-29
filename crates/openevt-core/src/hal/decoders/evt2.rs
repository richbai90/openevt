//! EVT2 raw decoder.
//!
//! This decoder is present to support the EVT2 file format, but it is more
//! limited than the EVT3 decoder and may still need protocol-level completion in
//! areas that are not exercised by the current reader path.

use std::sync::Arc;

use slotmap::{DefaultKey, SlotMap};

use crate::hal::dispatcher::{ErrorDispatcher, EventDispatcher};
use crate::hal::facilities::{
    DecoderErrorCallback, EventCDCallback, EventExtTriggerCallback, EventSubscriptionFacility,
    FacilityResult, RawDecoderFacility, RawEventStreamDecoderFacility,
};
use crate::types::{EventCD, EventExtTrigger, EventTimestamp, Polarity};

/// Decoder for EVT2 raw event streams.
pub struct Evt2Decoder {
    /// Publishes decoded events to subscribers.
    pub evt_dispatcher: Arc<EventDispatcher>,
    /// Publishes decoder errors and protocol violations.
    pub err_dispatcher: Arc<ErrorDispatcher>,

    // Time tracking
    first_ts: Option<EventTimestamp>,
    last_t: EventTimestamp,
    time_high: u32,
    /// Whether timestamps are shifted to begin at zero.
    pub do_time_shift: bool,

    // Geometry
    /// Maximum valid horizontal coordinate.
    pub max_x: u16,
    /// Maximum valid vertical coordinate.
    pub max_y: u16,

    // Buffers and Pools
    split_bytes: Vec<u8>,
    cd_buffer: Vec<EventCD>,
    ext_trigger_buffer: Vec<EventExtTrigger>,

    // markers
    markers: SlotMap<DefaultKey, EventTimestamp>,
}

impl Evt2Decoder {
    const BATCH_SIZE: usize = 4096;

    // EVT2 Bitmasks
    const TIME_LOW_MASK: u32 = 0x3F; // 6 bits
    const X_MASK: u32 = 0x7FF; // 11 bits
    const Y_MASK: u32 = 0x7FF; // 11 bits
    const TIME_HIGH_MASK: u32 = 0xFFFFFFF; // 28 bits
    const TRIGGER_ID_MASK: u32 = 0x1F; // 5 bits

    /// Creates a new EVT2 decoder for the supplied geometry.
    pub fn new(max_x: u16, max_y: u16, do_time_shift: bool) -> Self {
        Self {
            evt_dispatcher: Default::default(),
            err_dispatcher: Default::default(),
            first_ts: None,
            last_t: 0,
            time_high: 0,
            do_time_shift,
            max_x,
            max_y,
            split_bytes: Vec::with_capacity(4),
            cd_buffer: Vec::with_capacity(Self::BATCH_SIZE),
            ext_trigger_buffer: Vec::with_capacity(Self::BATCH_SIZE),
            markers: SlotMap::with_capacity(512),
        }
    }

    #[inline(always)]
    /// Reconstructs the current EVT2 timestamp from the stored high bits.
    fn current_timestamp(&mut self, time_low: u32) -> EventTimestamp {
        // Concatenate the 28-bit time_high with the 6-bit time_low
        let abs_ts = (u64::from(self.time_high) << 6) | u64::from(time_low);

        if self.do_time_shift {
            let first = *self.first_ts.get_or_insert(abs_ts);
            self.last_t = abs_ts.saturating_sub(first);
        } else {
            self.last_t = abs_ts;
        }

        self.last_t
    }

    /// Processes one EVT2 word and appends any decoded events to the buffers.
    ///
    /// TODO: verify the full EVT2 protocol coverage here. The current
    /// implementation handles the basic CD, time, and trigger word classes.
    fn process_word(&mut self, word: u32) {
        let evt_type = word >> 28;

        match evt_type {
            0x0 | 0x1 => {
                // 0x0 = CD_OFF (polarity 0), 0x1 = CD_ON (polarity 1)
                let p = Polarity::from(evt_type == 0x1);
                let time_low = (word >> 22) & Self::TIME_LOW_MASK;
                let x = (word >> 11) & Self::X_MASK;
                let y = word & Self::Y_MASK;

                if x <= self.max_x as u32 && y <= self.max_y as u32 {
                    let t = self.current_timestamp(time_low);
                    self.cd_buffer
                        .push(EventCD::new(u64::from(x), u64::from(y), p, t));
                }
            }
            0x8 => {
                // 0x8 = EVT_TIME_HIGH
                self.time_high = word & Self::TIME_HIGH_MASK;
            }
            0xA => {
                // 0xA = EXT_TRIGGER
                let time_low = (word >> 22) & Self::TIME_LOW_MASK;
                let id = (word >> 8) & Self::TRIGGER_ID_MASK;
                let value = Polarity::from((word & 0x01) == 1);
                let t = self.current_timestamp(time_low);

                self.ext_trigger_buffer
                    .push(EventExtTrigger::new(value, t, u64::from(id)));
            }
            _ => {
                // 0xE (OTHERS) and 0xF (CONTINUED) are vendor-specific and ignored by default
            }
        }

        if self.cd_buffer.len() >= Self::BATCH_SIZE
            || self.ext_trigger_buffer.len() >= Self::BATCH_SIZE
        {
            self.dispatch();
        }
    }

    /// Dispatches buffered CD and external-trigger events.
    ///
    /// Subscribers receive borrowed slices and must copy anything they keep, so
    /// the decoder retains its buffers and their capacity between dispatches.
    fn dispatch(&mut self) {
        if !self.cd_buffer.is_empty() {
            self.evt_dispatcher.send_cd(&self.cd_buffer);
            self.cd_buffer.clear();
        }

        if !self.ext_trigger_buffer.is_empty() {
            self.evt_dispatcher.send_ext(&self.ext_trigger_buffer);
            self.ext_trigger_buffer.clear();
        }
    }
}

impl RawEventStreamDecoderFacility for Evt2Decoder {
    /// Decodes a raw EVT2 byte buffer into typed events.
    fn decode(&mut self, raw_data: &[u8]) -> FacilityResult<()> {
        let mut data = raw_data;

        // Process any leftover bytes from the previous chunk
        if !self.split_bytes.is_empty() {
            let needed = 4 - self.split_bytes.len();
            if data.len() < needed {
                self.split_bytes.extend_from_slice(data);
                return Ok(());
            }

            self.split_bytes.extend_from_slice(&data[..needed]);

            // Unpack as little-endian 32-bit word
            let word = u32::from_le_bytes(self.split_bytes.as_slice().try_into().unwrap());
            self.process_word(word);

            self.split_bytes.clear();
            data = &data[needed..];
        }

        // Process exact 32-bit chunks
        let chunks = data.chunks_exact(4);
        let remainder = chunks.remainder();

        for chunk in chunks {
            let word = u32::from_le_bytes(chunk.try_into().unwrap());
            self.process_word(word);
        }

        // Store any trailing bytes for the next decode call
        if !remainder.is_empty() {
            self.split_bytes.extend_from_slice(remainder);
        }

        self.dispatch();
        Ok(())
    }

    /// Returns the last timestamp emitted by the decoder.
    fn get_last_timestamp(&self) -> EventTimestamp {
        self.last_t
    }

    /// Returns the current timestamp shift, if one has been established.
    fn get_timestamp_shift(&self) -> Option<EventTimestamp> {
        self.first_ts
    }

    /// Returns whether timestamps are shifted relative to the first event.
    fn is_time_shifting_enabled(&self) -> bool {
        self.do_time_shift
    }

    /// Updates the last decoded timestamp.
    fn reset_last_timestamp(&mut self, timestamp: EventTimestamp) {
        self.last_t = timestamp;
    }

    /// Updates the timestamp shift baseline.
    fn reset_timestamp_shift(&mut self, shift: EventTimestamp) {
        self.first_ts = Some(shift);
    }

    /// EVT2 streams are not currently treated as indexable.
    fn is_decoded_event_stream_indexable(&self) -> bool {
        false
    }

    fn add_marker(&mut self, timestamp: EventTimestamp) -> slotmap::DefaultKey {
        self.markers.insert(timestamp)
    }

    fn remove_marker(&mut self, key: slotmap::DefaultKey) -> Option<EventTimestamp> {
        self.markers.remove(key)
    }
}

impl RawDecoderFacility for Evt2Decoder {
    /// Subscribes to decoder protocol violation errors.
    fn subscribe_to_protocol_violation(
        &mut self,
        callback: DecoderErrorCallback,
    ) -> FacilityResult<()> {
        self.err_dispatcher
            .subscribe::<crate::hal::errors::DecoderProtocolViolation>(callback);
        Ok(())
    }

    /// Returns the EVT2 raw word size.
    fn get_raw_event_size_bytes(&self) -> FacilityResult<u8> {
        Ok(4) // EVT2 is strictly 32-bit / 4-byte words
    }
}

impl EventSubscriptionFacility for Evt2Decoder {
    /// Subscribes to decoded CD event batches.
    fn subscribe_to_cd_events(&mut self, callback: EventCDCallback) -> FacilityResult<()> {
        self.evt_dispatcher.subscribe_cd(callback);
        Ok(())
    }

    /// Subscribes to decoded external-trigger event batches.
    fn subscribe_to_ext_events(&mut self, callback: EventExtTriggerCallback) -> FacilityResult<()> {
        self.evt_dispatcher.subscribe_ext(callback);
        Ok(())
    }
}
