//! EVT3 raw decoder.
//!
//! This decoder turns 16-bit EVT3 words into typed CD and external-trigger
//! events. It also maintains timing state so callers can seek, index, and
//! optionally time-shift the output stream relative to the first observed event.

use std::sync::Arc;

use crate::hal::dispatcher::{ErrorDispatcher, EventDispatcher};
use crate::hal::errors::DecoderProtocolViolation;
use crate::hal::facilities::{
    DecoderErrorCallback, EventCDCallback, EventExtTriggerCallback, EventSubscriptionFacility,
    FacilityResult, RawDecoderFacility, RawEventStreamDecoderFacility,
};
use crate::types::{EventCD, EventExtTrigger, EventTimestamp};
use log::warn;
use macros::new;
use slotmap::{DefaultKey, SlotMap};

/// Decoder for the EVT3 event data format.
///
/// The decoder is stateful: it tracks spatial context, timestamp reconstruction,
/// and buffer boundaries so it can decode a continuous raw stream safely.
pub struct Evt3Decoder {
    /// Thread-safe dispatcher used to route decoded event batches.
    pub evt_dispatcher: Arc<EventDispatcher>,
    /// Thread-safe dispatcher used to route decode and protocol errors.
    pub err_dispatcher: Arc<ErrorDispatcher>,
    /// First decoded absolute timestamp, used when time shifting is enabled.
    first_ts: Option<EventTimestamp>,
    /// Accumulated wrap-around offset used to reconstruct a monotonic timestamp.
    time_offset: EventTimestamp,
    /// Last observed 24-bit timestamp value.
    last_t24: EventTimestamp,
    /// Last reported timestamp value.
    last_t: EventTimestamp,
    /// When true, timestamps are shifted so the first decoded event starts at 0.
    pub do_time_shift: bool,
    /// Trailing byte from a split 16-bit word at a buffer boundary.
    split_byte: Option<u8>,
    /// High portion of the current event timestamp.
    time_high: EventTimestamp,
    /// Low portion of the current event timestamp.
    time_low: EventTimestamp,
    /// Current Y coordinate in the EVT3 state machine.
    y: Option<u16>,
    /// Base X coordinate for vector payloads.
    base_x: u16,
    /// Current event polarity.
    polarity: bool,
    /// Previous decoded EVT3 word type.
    prev_word: Option<EVTWord>,
    /// Accumulates payload bits for "others" style multiword sequences.
    payload_accumulator: u64,
    /// Subtype marker for vendor-specific or otherwise unhandled `Others` words.
    others_subtype: u16,
    /// Current shift offset within `payload_accumulator`.
    payload_bit_shift: u8,
    /// Batched CD events waiting to be dispatched.
    cd_buffer: Vec<EventCD>,
    /// Batched external-trigger events waiting to be dispatched.
    ext_trigger_buffer: Vec<EventExtTrigger>,
    /// Maximum allowed X coordinate.
    pub max_x: u16,
    /// Maximum allowed Y coordinate.
    pub max_y: u16,
    /// Previous high timestamp value, preserved for timing validation.
    prev_time_high: EventTimestamp,

    markers: SlotMap<DefaultKey, u64>,
}

impl Default for Evt3Decoder {
    fn default() -> Self {
        Self {
            evt_dispatcher: Default::default(),
            err_dispatcher: Default::default(),
            first_ts: Default::default(),
            time_offset: 0,
            last_t24: Default::default(),
            last_t: Default::default(),
            do_time_shift: false,
            split_byte: Default::default(),
            time_high: Default::default(),
            time_low: Default::default(),
            y: Default::default(),
            base_x: Default::default(),
            polarity: Default::default(),
            prev_word: Default::default(),
            payload_accumulator: Default::default(),
            others_subtype: Default::default(),
            payload_bit_shift: Default::default(),
            cd_buffer: Vec::with_capacity(Self::BATCH_SIZE),
            ext_trigger_buffer: Vec::with_capacity(Self::BATCH_SIZE),
            max_x: 640,
            max_y: 480,
            prev_time_high: Default::default(),
            markers: SlotMap::with_capacity(512),
        }
    }
}

impl Evt3Decoder {
    /// Creates a decoder configured for the supplied sensor geometry.
    pub fn new(max_x: u16, max_y: u16, do_time_shift: bool) -> Self {
        let decoder: Evt3Decoder = Evt3Decoder {
            max_x,
            max_y,
            do_time_shift,
            ..Default::default()
        };

        decoder
    }
}

/// Represents the different 16-bit EVT3 word classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, new)]
enum EVTWord {
    /// X-coordinate address event.
    AddrX,
    /// Y-coordinate address event.
    AddrY,
    /// Base X vector event.
    VectBaseX,
    /// 12-bit vector event.
    Vect12,
    /// 8-bit vector event.
    Vect8,
    /// Lower bits of the timestamp.
    TimeLow,
    /// Higher bits of the timestamp.
    TimeHigh,
    /// 4-bit continued data event.
    Continued4,
    /// External trigger event.
    ExtTrigger,
    /// Other or unspecified event types.
    Others,
    /// 12-bit continued data event.
    Continued12,
}

/// Parses an EVT3 word class from the top four bits of a raw 16-bit word.
impl TryFrom<&u16> for EVTWord {
    type Error = DecoderProtocolViolation;

    fn try_from(value: &u16) -> Result<Self, Self::Error> {
        // Extract the 4 most significant bits by shifting right by 12.
        let msb = (value >> 12) as u8;
        match msb {
            0b0000 => Ok(Self::AddrY),
            0b0010 => Ok(Self::AddrX),
            0b0011 => Ok(Self::VectBaseX),
            0b0100 => Ok(Self::Vect12),
            0b0101 => Ok(Self::Vect8),
            0b0110 => Ok(Self::TimeLow),
            0b0111 => Ok(Self::Continued4),
            0b1000 => Ok(Self::TimeHigh),
            0b1010 => Ok(Self::ExtTrigger),
            0b1110 => Ok(Self::Others),
            0b1111 => Ok(Self::Continued12),
            // Catch any unmapped 4-bit patterns
            _ => Err(DecoderProtocolViolation::UnsupportedWord(msb.into())),
        }
    }
}

impl Evt3Decoder {
    const BATCH_SIZE: usize = 4096;
    /// Reconstructs a monotonic timestamp from the current decoder state.
    ///
    /// EVT3 timestamps are built from the current `time_high` and `time_low`
    /// fields. The decoder tracks wrap-around and optionally shifts timestamps so
    /// the first event starts at zero.
    #[inline(always)]
    pub fn current_timestamp(&mut self) -> EventTimestamp {
        let t24 = (self.time_high << 12) | self.time_low;

        if t24 < self.last_t24 {
            // If the time dropped by more than half the maximum 24-bit value,
            // it is a genuine hardware counter rollover.
            if (self.last_t24 - t24) > (1 << 23) {
                self.time_offset += 1 << 24;
            } else {
                warn!("Out-of-order multiplexing");
            }
            // A small backward step can result from out-of-order multiplexing.
            // We do not increment the offset.
        }

        self.last_t24 = t24;

        // Return the continuous 64-bit time
        let abs_ts = self.time_offset + t24;
        if self.do_time_shift {
            let first = *self.first_ts.get_or_insert(abs_ts);
            self.last_t = abs_ts.saturating_sub(first);
        } else {
            self.last_t = abs_ts
        }

        self.last_t
    }

    /// Returns the current low timestamp field.
    pub fn _get_time_low(&self) -> EventTimestamp {
        self.time_low
    }

    /// Returns the current high timestamp field.
    pub fn _get_time_high(&self) -> EventTimestamp {
        self.time_high
    }

    /// Updates the current low timestamp field.
    pub fn _set_time_low(&mut self, value: EventTimestamp) {
        self.time_low = value
    }

    /// Updates the current high timestamp field.
    pub fn _set_time_high(&mut self, value: EventTimestamp) {
        self.time_high = value
    }

    /// Flushes any buffered events to their dispatchers.
    pub fn flush(&mut self) {
        self.dispatch();
    }

    fn reset(&mut self) {
        self.prev_word = None;
    }

    /// Processes one EVT3 word and updates decoder state.
    ///
    /// The state machine updates timestamp and coordinate context, emits CD and
    /// external-trigger events, and returns a protocol violation when the word
    /// sequence is not valid.
    fn process_word(&mut self, word: u16) -> Result<(), DecoderProtocolViolation> {
        // Bitmasks for extracting payloads of various sizes
        const MASK_12: u16 = 0x0FFF;
        const MASK_11: u16 = 0x07FF;
        const MASK_4: u16 = 0x000F;
        const MASK_8: u16 = 0x00FF;

        let evt_result = EVTWord::try_from(&word);
        if let Err(e) = evt_result {
            self.reset();
            return Err(e);
        }

        let evt_type = evt_result.expect("If this happens, there's something strange going on.");
        match evt_type {
            EVTWord::AddrY => {
                // Decode and validate the Y coordinate
                let new_y = word & MASK_11;
                self.y = Some(new_y);
                self.prev_word = Some(EVTWord::AddrY);
            }
            EVTWord::AddrX => {
                // Ensure a Y coordinate was previously received
                let y_result = self.y.ok_or(DecoderProtocolViolation::MissingYAddr);
                if let Err(e) = y_result {
                    self.reset();
                    return Err(e);
                }
                let y = y_result.expect("Somehow y is an error but didn't get consumed in the error condition check. Weird.");
                // Decode and validate the X coordinate
                let x = word & MASK_11;
                if x > self.max_x {
                    self.reset();
                    return Err(DecoderProtocolViolation::OutOfBoundsEventCoordinate);
                }
                // Extract polarity and generate a Contrast Detector (CD) event
                // Only if y is valid
                if y < self.max_y {
                    let p = ((word >> 11) & 0x01) == 1;
                    let t = self.current_timestamp();
                    self.cd_buffer.push(EventCD::new(x.into(), y.into(), p, t));
                    self.prev_word = Some(EVTWord::AddrX);
                }
            }
            EVTWord::VectBaseX => {
                // Establish the base X coordinate and polarity for subsequent vector events
                if self.y.is_none() {
                    self.reset();
                    return Err(DecoderProtocolViolation::MissingYAddr);
                }
                self.base_x = word & MASK_11;
                self.polarity = ((word & MASK_12) >> 11 & 0x01) != 0;
                self.prev_word = Some(EVTWord::VectBaseX);
            }
            EVTWord::Vect12 | EVTWord::Vect8 => {
                // Ensure vector events follow a valid base or previous vector event
                if !matches!(
                    self.prev_word,
                    Some(EVTWord::VectBaseX) | Some(EVTWord::Vect12)
                ) {
                    self.reset();
                    return Err(DecoderProtocolViolation::InvalidVectBase);
                }

                let is_12 = matches!(evt_type, EVTWord::Vect12);
                let bit_count = if is_12 { 12 } else { 8 };
                let mask = if is_12 { MASK_12 } else { MASK_8 };

                // Validate that the vector length won't exceed maximum X coordinate
                if self.base_x + (bit_count - 1) > self.max_x {
                    self.reset();
                    return Err(DecoderProtocolViolation::OutOfBoundsEventCoordinate);
                }

                let t = self.current_timestamp();
                let p = self.polarity;
                let x = self.base_x;
                let y = self.y.unwrap();
                let valid = word & mask;

                // Generate CD events for each active bit in the vector payload
                if y < self.max_y {
                    for i in 0..bit_count {
                        if (valid >> i) & 0x01 == 1 {
                            self.cd_buffer
                                .push(EventCD::new((x + i).into(), y.into(), p, t));
                        }
                    }
                }

                // Advance the base X coordinate for the next potential vector event
                self.base_x += bit_count;
                self.prev_word = Some(evt_type);
            }
            EVTWord::ExtTrigger => {
                // Decode external trigger events (e.g., synchronization signals)
                let t = self.current_timestamp();
                let channel = u64::from((word >> 8) & MASK_4);
                let val = word & 0x01 == 1;
                self.ext_trigger_buffer
                    .push(EventExtTrigger::new(val, t, channel));
                self.prev_word = Some(EVTWord::ExtTrigger);
            }
            EVTWord::TimeLow => {
                // Update the lower bits of the current timestamp
                self.time_low = (word & MASK_12).into();
                self.prev_word = Some(EVTWord::TimeLow);
            }
            EVTWord::TimeHigh => {
                let new_time_high = u64::from(word & MASK_12);
                let wrap = self.time_high > 0xF00 && new_time_high < 0x0FF;

                // Track the specific error instead of a boolean
                let mut violation = None;

                if new_time_high < self.time_high && !wrap && self.first_ts.is_some() {
                    violation = Some(DecoderProtocolViolation::NonMonotonicTimeHigh);
                } else if new_time_high > self.time_high + 10 && self.first_ts.is_some() {
                    violation = Some(DecoderProtocolViolation::NonContinuousTimeHigh);
                }

                // Always synchronize the clock to the hardware stream.
                // Do not call reset() here, because this word successfully
                // establishes a new time baseline for subsequent TimeLow words.
                self.prev_time_high = self.time_high;
                self.time_high = new_time_high;
                self.prev_word = Some(EVTWord::TimeHigh);

                // Return the specific error if one occurred
                if let Some(err) = violation {
                    return Err(err);
                }
            }
            EVTWord::Continued12 | EVTWord::Continued4 => {
                // Accumulate multi-word payload bits for 'Others' events
                let is_12 = matches!(evt_type, EVTWord::Continued12);
                let valid_prev = if is_12 {
                    matches!(
                        self.prev_word,
                        Some(EVTWord::Others) | Some(EVTWord::Continued12)
                    )
                } else {
                    matches!(
                        self.prev_word,
                        Some(EVTWord::Others)
                            | Some(EVTWord::Continued12)
                            | Some(EVTWord::Continued4)
                    )
                };

                if !valid_prev {
                    // Discard the orphaned payload and clear the state machine to prevent cascading failures
                    self.payload_accumulator = 0;
                    self.payload_bit_shift = 0;
                    self.prev_word = None;
                    return Ok(());
                }

                let mask = if is_12 { MASK_12 } else { MASK_4 };
                let shift_inc = if is_12 { 12 } else { 4 };

                // Shift and accumulate the payload
                let payload = u64::from(word & mask);
                match payload.checked_shl(self.payload_bit_shift as u32) {
                    Some(shifted_payload) => {
                        self.payload_accumulator |= shifted_payload;
                        self.payload_bit_shift += shift_inc;
                        self.prev_word = Some(evt_type);
                    }
                    None => {
                        self.reset();
                        return Err(DecoderProtocolViolation::PartialContinued);
                    }
                }
            }
            EVTWord::Others => {
                // Initiate a new sequence for generic/other multi-word data types
                self.others_subtype = word & MASK_12;
                self.payload_accumulator = 0;
                self.payload_bit_shift = 0;
                self.prev_word = Some(EVTWord::Others);
            }
        };

        // Dispatch events in batches to avoid unbounded buffer growth
        if self.cd_buffer.len() >= Self::BATCH_SIZE
            || self.ext_trigger_buffer.len() >= Self::BATCH_SIZE
        {
            self.dispatch();
        }

        Ok(())
    }

    /// Dispatches all currently buffered events.
    fn dispatch(&mut self) {
        // Process CD (Change Detection) buffer if it contains any events
        if !self.cd_buffer.is_empty() {
            let populated_buffer = std::mem::take(&mut self.cd_buffer);
            self.evt_dispatcher.send_cd(&populated_buffer);
        }

        // Process external trigger buffer if it contains any events
        if !self.ext_trigger_buffer.is_empty() {
            let populated_buffer = std::mem::take(&mut self.ext_trigger_buffer);
            self.evt_dispatcher.send_ext(&populated_buffer);
        }
    }
}

impl RawDecoderFacility for Evt3Decoder {
    /// Subscribes to decoder protocol violation errors.
    fn subscribe_to_protocol_violation(
        &mut self,
        callback: DecoderErrorCallback,
    ) -> FacilityResult<()> {
        self.err_dispatcher
            .subscribe::<DecoderProtocolViolation>(callback);
        Ok(())
    }

    /// Returns the EVT3 raw word size.
    fn get_raw_event_size_bytes(&self) -> crate::hal::facilities::FacilityResult<u8> {
        Ok(2)
    }
}

impl RawEventStreamDecoderFacility for Evt3Decoder {
    /// Decodes a raw byte buffer into typed EVT3 events.
    ///
    /// Buffer boundaries may split 16-bit words, so the decoder preserves one
    /// trailing byte between calls when needed.
    fn decode(&mut self, raw_data: &[u8]) -> crate::hal::facilities::FacilityResult<()> {
        let mut data = raw_data;

        if let Some(first_byte) = self.split_byte.take() {
            // If we don't have any data yet to work with, restore the split_byte and return
            if raw_data.is_empty() {
                self.split_byte = Some(first_byte);
                return Ok(());
            }

            // Otherwise, append the split byte to the first byte in the stream.
            // This is to handle a buffer that isn't aligned with a word boundary
            let word = u16::from_le_bytes([first_byte, data[0]]);
            if let Err(e) = self.process_word(word) {
                self.err_dispatcher.dispatch(e);
            }
            data = &data[1..] // Move the buffer forward past the consumed byte
        }

        let chunks = data.chunks_exact(2); // collect all 16-bit words
        let remainder = chunks.remainder(); // Save any trailing byte as a split

        for chunk in chunks {
            let lsw = chunk[0];
            let msw = chunk[1];
            let word = u16::from_le_bytes([lsw, msw]);

            if let Err(e) = self.process_word(word) {
                self.err_dispatcher.dispatch(e);
            }
        }

        if !remainder.is_empty() {
            self.split_byte = Some(remainder[0]);
        }

        self.dispatch();

        Ok(())
    }

    fn add_marker(&mut self, timestamp: EventTimestamp) -> DefaultKey {
        self.markers.insert(timestamp)
    }
    fn remove_marker(&mut self, key: DefaultKey) -> Option<u64> {
        self.markers.remove(key)
    }

    /// Returns the last timestamp emitted by the decoder.
    fn get_last_timestamp(&self) -> EventTimestamp {
        self.last_t
    }

    /// Returns the current time shift, if one has been established.
    fn get_timestamp_shift(&self) -> Option<EventTimestamp> {
        self.first_ts
    }

    /// Returns whether decoded timestamps are shifted relative to the first event.
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

    /// EVT3 streams can be indexed because time words and event words can be
    /// replayed into a stable timing state.
    fn is_decoded_event_stream_indexable(&self) -> bool {
        true
    }
}

impl EventSubscriptionFacility for Evt3Decoder {
    /// Subscribes to decoded CD event batches.
    fn subscribe_to_cd_events(&mut self, callback: EventCDCallback) -> FacilityResult<()> {
        self.evt_dispatcher.subscribe_cd(callback);
        Ok(())
    }

    /// Subscribes to decoded external-trigger batches.
    fn subscribe_to_ext_events(&mut self, callback: EventExtTriggerCallback) -> FacilityResult<()> {
        self.evt_dispatcher.subscribe_ext(callback);
        Ok(())
    }
}

/// Snapshot of the EVT3 decoder timing state.
#[derive(Clone, Copy, Debug, Default)]
pub struct DecoderTimingState {
    /// Reconstructed high-order timestamp component.
    pub time_high: EventTimestamp,
    /// Reconstructed low-order timestamp component.
    pub time_low: EventTimestamp,
    /// Applied timestamp shift.
    pub time_offset: EventTimestamp,
    /// Last 24-bit timestamp observed.
    pub last_t24: EventTimestamp,
    /// Last complete timestamp observed.
    pub last_t: EventTimestamp,
}

impl Evt3Decoder {
    /// Captures the current absolute clock state.
    pub fn get_timing_state(&self) -> DecoderTimingState {
        DecoderTimingState {
            time_high: self.time_high,
            time_low: self.time_low,
            time_offset: self.time_offset,
            last_t24: self.last_t24,
            last_t: self.last_t,
        }
    }

    /// Restores a previously saved timing context.
    ///
    /// Spatial state is cleared because timing snapshots are only valid for the
    /// timestamp pipeline, not for partially decoded coordinate sequences.
    pub fn set_timing_state(&mut self, state: DecoderTimingState) {
        self.time_high = state.time_high;
        self.time_low = state.time_low;
        self.time_offset = state.time_offset;
        self.last_t24 = state.last_t24;
        self.last_t = state.last_t;

        // Reset coordinate state machine
        self.y = None;
        self.base_x = 0;
        self.prev_word = None;
        self.payload_accumulator = 0;
        self.payload_bit_shift = 0;
        self.split_byte = None;
    }
}
