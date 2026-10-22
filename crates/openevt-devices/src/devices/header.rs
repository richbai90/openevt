use crate::device_types::{DeviceFileError, FileFormat};
use openevt_core::hal::facilities::SensorInfo;
use std::collections::HashMap;
use std::io::BufRead;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
/// Metadata and geometry parsed from a raw event-file header.
pub struct Header {
    /// Detected event-file format.
    pub format: FileFormat,
    /// Sensor width in pixels.
    pub width: u32,
    /// Sensor height in pixels.
    pub height: u32,
    /// All metadata entries found in the header.
    pub metadata: HashMap<String, String>,
}

impl Header {
    /// Parses header lines from `reader`, leaving the first raw-data byte unread.
    pub fn parse<R: BufRead>(reader: &mut R) -> Result<Header, DeviceFileError> {
        let mut metadata = HashMap::new();

        loop {
            let buf = reader.fill_buf()?;
            if buf.is_empty() || buf[0] != b'%' {
                break;
            }

            let mut line = String::new();
            reader.read_line(&mut line)?;

            let line = line.trim();
            if let Some(rest) = line.strip_prefix('%') {
                let parts: Vec<&str> = rest.splitn(2, ':').collect();
                if parts.len() == 2 {
                    metadata.insert(parts[0].trim().to_string(), parts[1].trim().to_string());
                } else {
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if parts.len() >= 2 {
                        metadata.insert(parts[0].to_string(), parts[1..].join(" "));
                    }
                }
            }
        }

        let raw_format_str = metadata
            .get("Data format")
            .or_else(|| metadata.get("format"))
            .map(|s| s.as_str())
            .unwrap_or("UNKNOWN");

        let parts = raw_format_str.splitn(3, ';');
        let mut format_str: Option<String> = None;
        let mut width: Option<String> = None;
        let mut height: Option<String> = None;
        for (i, p) in parts.enumerate() {
            match i {
                0 => format_str = Some(p.trim().to_string()),
                1 => width = Some(p.trim().to_string()),
                2 => height = Some(p.trim().to_string()),
                _ => unreachable!("splitn(3, ';') cannot produce more than three parts"),
            }
        }

        let fmt_str = format_str.unwrap_or_else(|| raw_format_str.to_string());

        let format = match fmt_str.as_str() {
            "EVT2" => FileFormat::EVT2,
            "EVT3" => FileFormat::EVT3,
            "DAT" => FileFormat::DAT,
            "HDF5" => FileFormat::HDF5,
            _ => FileFormat::UNKNOWN,
        };

        if let (Some(w), Some(h)) = (width, height) {
            metadata.insert("Geometry".to_string(), format!("{},{}", w, h));
        }

        let geometry_str = metadata
            .get("Geometry")
            .ok_or_else(|| metadata.get("geometry"))
            .or(Err(DeviceFileError::UnknownGeometry()))?;

        let coords = {
            if geometry_str.contains("=") {
                geometry_str
                    .split(',')
                    .try_fold(HashMap::<&str, &str>::new(), |mut acc, s| {
                        let parts: Vec<&str> = s.split("=").collect();
                        if parts.len() != 2 {
                            return Err(DeviceFileError::UnknownGeometry());
                        }
                        acc.insert(parts[0], parts[1]);
                        Ok(acc)
                    })
            } else {
                let mut coord_map = HashMap::<&str, &str>::new();
                let coord_values: Vec<&str> = geometry_str.split(',').collect();
                if coord_values.len() != 2 {
                    return Err(DeviceFileError::UnknownGeometry());
                }
                coord_map.insert("width", coord_values[0]);
                coord_map.insert("height", coord_values[1]);

                Ok(coord_map)
            }
        }?;
        if coords.len() != 2 {
            return Err(DeviceFileError::UnknownGeometry());
        }

        let width = coords
            .get("width")
            .ok_or(DeviceFileError::UnknownGeometry())?
            .parse::<u32>()?;
        let height = coords
            .get("height")
            .ok_or(DeviceFileError::UnknownGeometry())?
            .parse::<u32>()?;

        Ok(Header {
            format,
            width,
            height,
            metadata,
        })
    }
}

/// Builds core sensor information from the corresponding raw-file header.
pub fn sensor_info_from_header(header: &Header) -> SensorInfo {
    let name = header
        .metadata
        .get("sensor_name")
        .map_or("UNKNOWN".to_string(), |v| v.to_string());
    let integrator = header
        .metadata
        .get("integrator_name")
        .map_or("UNKNOWN".to_string(), |v| v.to_string());
    let version = header
        .metadata
        .get("sensor_generation")
        .or_else(|| header.metadata.get("generation"))
        .map_or("x.x".to_string(), |v| v.to_string());

    SensorInfo {
        name,
        integrator,
        version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn parse(input: &str) -> Result<Header, DeviceFileError> {
        Header::parse(&mut Cursor::new(input.as_bytes()))
    }

    #[test]
    fn parses_colon_metadata_and_evt3_geometry() {
        let header = parse(
            "%Data format: EVT3; 640; 480\n% sensor_name: Test Sensor\n%integrator_name: ACME\n%generation: 3\n\nraw",
        )
        .unwrap();

        assert_eq!(header.format, FileFormat::EVT3);
        assert_eq!((header.width, header.height), (640, 480));
        assert_eq!(header.metadata["Geometry"], "640,480");
        assert_eq!(sensor_info_from_header(&header).name, "Test Sensor");
        assert_eq!(sensor_info_from_header(&header).version, "3");
    }

    #[test]
    fn parses_whitespace_metadata_and_keyed_geometry() {
        let header = parse("% format EVT2\n% Geometry: width=12,height=34\ndata").unwrap();

        assert_eq!(header.format, FileFormat::EVT2);
        assert_eq!((header.width, header.height), (12, 34));
    }

    #[test]
    fn leaves_data_at_the_start_of_the_reader() {
        let mut reader = Cursor::new(b"%Geometry: 2,3\n\x01\x02".to_vec());
        let header = Header::parse(&mut reader).unwrap();

        assert_eq!((header.width, header.height), (2, 3));
        assert_eq!(reader.position(), b"%Geometry: 2,3\n".len() as u64);
        let position = reader.position().try_into().unwrap();
        assert_eq!(&reader.get_ref()[position..], &[1, 2]);
    }

    #[test]
    fn rejects_missing_or_malformed_geometry_without_panicking() {
        assert!(matches!(
            parse("%Data format: EVT3; 640\n"),
            Err(DeviceFileError::UnknownGeometry())
        ));
        assert!(matches!(
            parse("%Geometry: 640\n"),
            Err(DeviceFileError::UnknownGeometry())
        ));
        assert!(matches!(
            parse("%Geometry: width=640,height=bad\n"),
            Err(DeviceFileError::GeometryParsing(_))
        ));
    }

    #[test]
    fn uses_defaults_for_missing_sensor_metadata() {
        let header = parse("%Geometry: 1,2\n").unwrap();
        let info = sensor_info_from_header(&header);

        assert_eq!(info.name, "UNKNOWN");
        assert_eq!(info.integrator, "UNKNOWN");
        assert_eq!(info.version, "x.x");
    }
}
