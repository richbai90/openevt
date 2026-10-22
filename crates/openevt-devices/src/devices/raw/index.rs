//! Timestamp index construction for raw files.
//!
//! The index is a coarse map from byte offsets to decoder timing state. It lets
//! the reader seek near a target timestamp without decoding the entire file from
//! the beginning.

use crate::raw::decoder::RawEventStreamDecoder;
use crate::raw::stream::RawEventStream;
use openevt_core::types::EventTimestamp;
use crate::device_types::{DeviceFileError, FileIndex, IndexMarker};
use openevt_core::hal::decoders::evt3::Evt3Decoder;
use openevt_core::hal::facilities::RawEventStreamDecoderFacility;
use std::io::{Read, Seek, SeekFrom};

/// Builds a coarse timestamp index for a raw file.
///
/// The index is sampled every `chunk_size_bytes`, which makes seeking faster but
/// not exact. The decoder state stored in each marker is used to resume decoding
/// after a seek.
pub(crate) fn build_index(
    path: &str,
    header_end_pos: u64,
    chunk_size_bytes: u64,
) -> Result<FileIndex, DeviceFileError> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(header_end_pos))?;

    let mut decoder = Evt3Decoder::new(640, 480, true);
    let mut markers = Vec::new();

    let mut buffer = vec![0u8; 65536];
    let mut current_byte_offset = header_end_pos;
    let mut last_marked_offset = header_end_pos;

    markers.push(IndexMarker {
        byte_offset: header_end_pos,
        timestamp: 0,
        state: decoder.get_timing_state(),
    });

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }

        for chunk in buffer[..bytes_read].chunks_exact(2) {
            let word = u16::from_le_bytes([chunk[0], chunk[1]]);

            let msb = (word >> 12) as u8;
            match msb {
                0b0110 => {
                    decoder._set_time_low((word & 0x0FFF).into());
                }
                0b1000 => {
                    decoder._set_time_high((word & 0x0FFF).into());
                }
                // Match the EVT3 decoder's timestamp sampling points. TimeLow and
                // TimeHigh words only update clock state; event words consume it.
                0b0010 | 0b0100 | 0b0101 | 0b1010 => {
                    let _ = decoder.current_timestamp();
                }
                _ => {}
            }
        }

        current_byte_offset += bytes_read as u64;

        if current_byte_offset - last_marked_offset >= chunk_size_bytes {
            markers.push(IndexMarker {
                byte_offset: current_byte_offset,
                timestamp: decoder.get_last_timestamp(),
                state: decoder.get_timing_state(),
            });
            last_marked_offset = current_byte_offset;
        }
    }
    let t_min = decoder.get_timestamp_shift().unwrap_or(0);
    let t_max = decoder.get_last_timestamp();
    Ok(FileIndex {
        markers,
        t_min,
        t_max,
    })
}

/// Seeks the stream and decoder to the closest indexed position at or before `target_timestamp`.
pub(crate) fn seek_to_timestamp<const N: usize>(
    index: &FileIndex,
    target_timestamp: EventTimestamp,
    stream: &mut RawEventStream<N>,
    decoder: &mut RawEventStreamDecoder,
) -> Result<(), DeviceFileError> {
    if let Some(marker) = index.find_closest_marker(target_timestamp) {
        stream.seek_to_offset(marker.byte_offset)?;
        decoder.set_evt3_timing_state(marker.state)?;
    }

    Ok(())
}
