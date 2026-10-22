use crate::header::Header;
use crate::raw::decoder::RawEventStreamDecoder;
use crate::raw::facilities::{RawReaderGeometry, RawReaderHWIdentification, RawReaderROI};
use crate::raw::stream::RawEventStream;
use crate::device_types::{DeviceFileError, FileFormat};
use openevt_core::hal::device::device::Device;
use openevt_core::hal::facilities::{FacilityHandle, FacilityType};
use openevt_core::pack_facility;
use std::collections::HashMap;
use std::io::{Seek, SeekFrom};
use std::sync::Arc;

/// A file-backed OpenEB device exposing the facilities found in a raw event
/// file.
///
/// `RawFileHandler` parses the file header and registers geometry, hardware
/// identification, ROI, event-stream, and decoder facilities. The const
/// generic `N` controls the size, in bytes, of each read from the event
/// stream. Larger buffers can reduce I/O overhead, while smaller buffers use
/// less memory.
///
/// The handler implements [`Device`]. Retrieve facilities with
/// [`Device::get_facility`] and convert the returned [`FacilityHandle`] into
/// the corresponding typed handle.
///
/// This is the low-level implementation used by the raw-file plugin.
pub struct RawFileHandler<const N: usize> {
    header: Arc<Header>,
    facilities: HashMap<FacilityType, FacilityHandle>,
    header_end_pos: u64,
}

impl<const N: usize> RawFileHandler<N> {
    fn ensure_supported_format(format: FileFormat) -> Result<(), DeviceFileError> {
        if format == FileFormat::EVT3 {
            Ok(())
        } else {
            Err(DeviceFileError::Format(format.to_string()))
        }
    }

    /// Creates an empty handler without an open file.
    ///
    /// Use [`Self::try_open`] to populate the handler before requesting file
    /// facilities. The empty state is useful when a handler needs to be
    /// allocated before the input path is known.
    pub fn new() -> Self {
        Self {
            header: Arc::new(Header::default()),
            facilities: HashMap::default(),
            header_end_pos: u64::default(),
        }
    }

    /// Returns the sensor dimensions as `(height, width)` in pixels.
    ///
    /// An empty handler returns `(0, 0)`.
    pub fn shape(&self) -> (u32, u32) {
        (self.header.height, self.header.width)
    }

    /// Opens `path`, parses its header, and registers the file-backed
    /// facilities.
    ///
    /// Currently, event decoding is implemented for EVT3 files. Other raw
    /// formats may parse successfully but are not yet decodable.
    pub fn new_from_path(path: &str) -> Result<Self, DeviceFileError> {
        let mut file = std::fs::File::open(path)?;

        let mut reader = std::io::BufReader::new(&mut file);
        let header = Header::parse(&mut reader)?;
        Self::ensure_supported_format(header.format)?;
        let header_end_pos = reader.stream_position()?;

        drop(reader);
        file.seek(SeekFrom::Start(header_end_pos))?;

        let header = Arc::new(header);
        let mut device = RawFileHandler {
            header: header.clone(),
            facilities: HashMap::new(),
            header_end_pos,
        };

        let geometry =
            RawReaderGeometry::new(device.header.width as i32, device.header.height as i32);
        device.register_facility(
            FacilityType::GeometryFacility,
            pack_facility!(ro GeometryFacility, geometry),
        );

        let hw_identification = RawReaderHWIdentification::new(header.clone());
        device.register_facility(
            FacilityType::HWIdentificationFacility,
            pack_facility!(ro HWIdentificationFacility, hw_identification),
        );

        let roi = RawReaderROI::default();
        device.register_facility(
            FacilityType::ROIFacility,
            pack_facility!(mut ROIFacility, roi),
        );

        let stream = RawEventStream::<N>::new(file);
        device.register_facility(
            FacilityType::RawEventStreamFacility,
            pack_facility!(mut RawEventStreamFacility, stream),
        );

        let decoder = RawEventStreamDecoder::new(&header, true);
        device.register_facility(
            FacilityType::RawEventStreamDecoderFacility,
            pack_facility!(mut RawEventStreamDecoderFacility, decoder.clone()),
        );
        device.register_facility(
            FacilityType::EventSubscriptionFacility,
            pack_facility!(mut EventSubscriptionFacility, decoder),
        );

        Ok(device)
    }

    /// Opens `path` in an existing handler, replacing its current file and
    /// facilities.
    ///
    /// If opening or parsing fails, the existing handler remains unchanged.
    /// Replaces the current file with `path` and rebuilds its facilities.
    pub fn try_open(&mut self, path: &str) -> Result<(), DeviceFileError> {
        let mut file = std::fs::File::open(path)?;

        let mut reader = std::io::BufReader::new(&mut file);
        let header = Header::parse(&mut reader)?;
        Self::ensure_supported_format(header.format)?;
        let header_end_pos = reader.stream_position()?;

        drop(reader);
        file.seek(SeekFrom::Start(header_end_pos))?;

        let header = Arc::new(header);
        self.header = header.clone();
        self.header_end_pos = header_end_pos;
        self.facilities.clear();

        let geometry = RawReaderGeometry::new(header.width as i32, header.height as i32);
        self.register_facility(
            FacilityType::GeometryFacility,
            pack_facility!(ro GeometryFacility, geometry),
        );

        let hw_identification = RawReaderHWIdentification::new(header.clone());
        self.register_facility(
            FacilityType::HWIdentificationFacility,
            pack_facility!(ro HWIdentificationFacility, hw_identification),
        );

        let stream = RawEventStream::<N>::new(file);
        self.register_facility(
            FacilityType::RawEventStreamFacility,
            pack_facility!(mut RawEventStreamFacility, stream),
        );

        let roi = RawReaderROI::default();
        self.register_facility(
            FacilityType::ROIFacility,
            pack_facility!(mut ROIFacility, roi),
        );

        let decoder = RawEventStreamDecoder::new(&header, true);

        self.register_facility(
            FacilityType::RawEventStreamDecoderFacility,
            pack_facility!(mut RawEventStreamDecoderFacility, decoder.clone()),
        );
        self.register_facility(
            FacilityType::EventSubscriptionFacility,
            pack_facility!(mut EventSubscriptionFacility, decoder),
        );

        Ok(())
    }

    pub(crate) fn header_end_pos(&self) -> u64 {
        self.header_end_pos
    }
}

impl<const N: usize> Device for RawFileHandler<N> {
    fn get_facility(&self, facility_type: FacilityType) -> Option<FacilityHandle> {
        self.facilities.get(&facility_type).cloned()
    }

    fn get_facilities(&self) -> Vec<FacilityType> {
        self.facilities.keys().copied().collect()
    }

    fn register_facility(
        &mut self,
        facility_type: FacilityType,
        facility_handle: FacilityHandle,
    ) -> Option<FacilityHandle> {
        self.facilities.insert(facility_type, facility_handle)
    }
}
