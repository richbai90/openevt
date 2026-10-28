use super::RawFileReader;
use super::device::RawFileHandler;
use super::stream::RawEventStream;
use openevt_core::hal::device::device::Device;
use openevt_core::hal::errors::StreamError;
use openevt_core::hal::facilities::RawEventStreamFacility;
use openevt_core::hal::facilities::{
    EventSubscriptionFacilityHandle, FacilityError, FacilityType,
    RawEventStreamDecoderFacilityHandle, RawEventStreamFacilityHandle,
};
use openevt_core::types::{EventCD, Polarity};
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn sample_raw_path() -> PathBuf {
    let mut file_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    file_path.push("tests");
    file_path.push("sample.raw");
    file_path
}

#[test]
fn test_read_and_decode_raw_evt3() -> TestResult {
    let file_path = sample_raw_path();

    let device = RawFileHandler::<131_072>::new_from_path(
        file_path
            .into_os_string()
            .to_str()
            .expect("A cargo manifest dir must be specified."),
    )
    .expect(
        "Failed to initialize device from path. Check if the file exists and the header is valid.",
    );

    let stream_handle: RawEventStreamFacilityHandle = device
        .get_facility(FacilityType::RawEventStreamFacility)
        .expect("RawEventStreamFacility was not registered")
        .try_into()
        .unwrap();

    let mut stream = stream_handle.write().unwrap();

    let decoder_handle: RawEventStreamDecoderFacilityHandle = device
        .get_facility(FacilityType::RawEventStreamDecoderFacility)
        .expect("RawEventStreamDecoderFacility was not registered")
        .try_into()
        .unwrap();

    let event_decoder_handle: EventSubscriptionFacilityHandle = device
        .get_facility(FacilityType::EventSubscriptionFacility)
        .expect("EventSubscriptionFacility was not registered")
        .try_into()
        .unwrap();

    let mut decoder = decoder_handle.write().unwrap();
    let mut event_decoder = event_decoder_handle.write().unwrap();

    let (event_sender, cd_receiver) = crossbeam::channel::unbounded::<Vec<EventCD>>();
    event_decoder.subscribe_to_cd_events(Box::new(move |events| {
        let _ = event_sender.send(events.to_vec());
    }))?;

    stream.start().expect("Failed to start stream");

    let mut total_bytes_read = 0;
    let mut chunks_processed = 0;

    loop {
        match stream.wait_next_buffer() {
            Ok((buffer, size)) => {
                chunks_processed += 1;
                total_bytes_read += size;
                decoder.decode(&buffer)?;

                while let Ok(event_batch) = cd_receiver.try_recv() {
                    for event in event_batch.iter() {
                        dbg!("Event: {}", event);
                    }
                }
            }
            Err(FacilityError::Stream(StreamError::EndOfFile)) => {
                break;
            }
            Err(e) => {
                panic!("Unexpected stream error: {:?}", e);
            }
        }
    }

    stream.stop().expect("Failed to stop stream");

    assert!(
        total_bytes_read > 0,
        "Stream completed but zero bytes were read."
    );
    assert!(chunks_processed > 0, "No chunks were processed.");

    println!(
        "Successfully parsed {} bytes across {} chunks.",
        total_bytes_read, chunks_processed
    );

    Ok(())
}

#[test]
fn test_raw_file_reader_requires_index_for_seek() -> TestResult {
    let file_path = sample_raw_path();
    let mut reader = RawFileReader::<131_072>::try_from_file(
        file_path
            .to_str()
            .expect("A cargo manifest dir must be specified."),
        false,
    )?;

    let err = reader.seek(0).expect_err("Seek without an index must fail");
    assert!(
        matches!(err, crate::device_types::DeviceFileError::UnsupportedBehavior(_)),
        "Expected UnsupportedBehavior, got {err:?}"
    );

    Ok(())
}

#[test]
fn default_reader_rejects_operations_until_opened() {
    let mut reader = RawFileReader::<128>::new();

    assert!(!reader.ready());
    assert_eq!(reader.shape(), (0, 0));
    assert!(matches!(
        reader.load_batch(),
        Err(crate::device_types::DeviceFileError::NotInitialized)
    ));
    assert!(matches!(
        reader.cd_receiver(),
        Err(crate::device_types::DeviceFileError::NotInitialized)
    ));
    assert!(matches!(
        reader.roi(),
        Err(crate::device_types::DeviceFileError::UnsupportedFacility(_))
    ));
    assert!(matches!(
        reader.seek(0),
        Err(crate::device_types::DeviceFileError::NotInitialized)
    ));
}

#[test]
fn opened_reader_exposes_geometry_and_roi_facilities() -> TestResult {
    let path = sample_raw_path();
    let reader = RawFileReader::<131_072>::try_from_file(path.to_str().unwrap(), false)?;

    assert!(reader.ready());
    assert_eq!(reader.shape(), (720, 1280));

    let roi = reader.roi()?;
    let mut roi = roi.write().unwrap();
    assert!(!roi.get_enabled()?);
    roi.set_enabled(true)?;
    roi.set_roi((1, 2, 3, 4))?;
    assert!(roi.get_enabled()?);
    assert_eq!(roi.roi(), Some((1, 2, 3, 4)));

    Ok(())
}

#[test]
fn stream_requires_start_and_reports_eof_until_repositioned() -> TestResult {
    let path = std::env::temp_dir().join(format!("openevt-stream-{}", std::process::id()));
    {
        let mut file = File::create(&path)?;
        file.write_all(&[10, 20, 30])?;
    }
    let file = File::open(&path)?;
    let mut stream = RawEventStream::<2>::new(file);

    assert!(matches!(
        stream.poll_buffer(),
        Err(FacilityError::Stream(StreamError::Disconnected))
    ));
    stream.start()?;
    assert_eq!(stream.poll_buffer()?.1, 2);
    assert_eq!(stream.poll_buffer()?.1, 1);
    assert!(matches!(
        stream.poll_buffer(),
        Err(FacilityError::Stream(StreamError::EndOfFile))
    ));
    assert!(matches!(
        stream.wait_next_buffer(),
        Err(FacilityError::Stream(StreamError::EndOfFile))
    ));
    stream.seek_to_offset(0)?;
    assert_eq!(stream.poll_buffer()?.0, &[10, 20]);
    stream.stop()?;
    assert!(matches!(
        stream.poll_buffer(),
        Err(FacilityError::Stream(StreamError::Disconnected))
    ));

    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn sync_iterator_caps_batches_and_preserves_leftovers() -> TestResult {
    let (sender, receiver) = crossbeam::channel::unbounded();
    let events = vec![
        EventCD {
            x: 1,
            y: 2,
            p: Polarity::Positive,
            t: 10,
        },
        EventCD {
            x: 3,
            y: 4,
            p: Polarity::Negative,
            t: 11,
        },
        EventCD {
            x: 5,
            y: 6,
            p: Polarity::Positive,
            t: 12,
        },
    ];
    sender.send(events)?;
    drop(sender);

    let stream: Arc<RwLock<dyn RawEventStreamFacility + Send>> = Arc::new(RwLock::new(
        RawEventStream::<2>::new(File::open(sample_raw_path())?),
    ));
    let decoder: Arc<
        RwLock<dyn openevt_core::hal::facilities::RawEventStreamDecoderFacility + Send>,
    > = Arc::new(RwLock::new(
        crate::raw::decoder::RawEventStreamDecoder::new(
            &crate::header::Header {
                format: crate::device_types::FileFormat::EVT3,
                width: 1280,
                height: 720,
                metadata: Default::default(),
            },
            true,
        ),
    ));
    let mut iter =
        super::iterator::EventWindowIterator::<2>::new(receiver, (720, 1280), stream, decoder)
            .into_async();

    assert_eq!(iter.next_batch()?.len(), 2);
    assert_eq!(iter.next_batch()?.len(), 1);
    assert!(iter.next_batch()?.is_empty());
    Ok(())
}

#[test]
fn test_raw_file_reader_can_seek_when_indexed() -> TestResult {
    let file_path = sample_raw_path();
    let mut reader = RawFileReader::<131_072>::try_from_file(
        file_path
            .to_str()
            .expect("A cargo manifest dir must be specified."),
        true,
    )?;

    reader.seek(0)?;

    Ok(())
}

#[test]
fn test_raw_file_reader_can_open_after_default_construction() -> TestResult {
    let file_path = sample_raw_path();
    let mut reader = RawFileReader::<131_072>::new();

    reader.try_open(
        file_path
            .to_str()
            .expect("A cargo manifest dir must be specified."),
        true,
    )?;
    reader.seek(0)?;

    Ok(())
}

#[test]
fn test_raw_file_reader_load_batch_publishes_cd_events() -> TestResult {
    let file_path = sample_raw_path();
    let mut reader = RawFileReader::<131_072>::try_from_file(
        file_path
            .to_str()
            .expect("A cargo manifest dir must be specified."),
        false,
    )?;
    let receiver = reader.cd_receiver()?;

    reader.load_batch()?;

    let mut event_count = 0;
    while let Ok(batch) = receiver.try_recv() {
        event_count += batch.len();
    }

    assert!(event_count > 0, "Expected at least one decoded CD event");

    Ok(())
}

#[test]
fn test_sync_event_window_iterator_loads_batches_on_demand() -> TestResult {
    let file_path = sample_raw_path();
    let mut reader = RawFileReader::<4_096>::try_from_file(
        file_path
            .to_str()
            .expect("A cargo manifest dir must be specified."),
        false,
    )?;
    let mut windows = reader.as_windows()?.into_sync();
    let batch = windows.next_batch()?;

    assert_eq!(batch.len(), 4_096);
    Ok(())
}

#[test]
fn test_sync_event_window_iterator_slices_delta_windows() -> TestResult {
    let file_path = sample_raw_path();
    let mut reader = RawFileReader::<4_096>::try_from_file(
        file_path
            .to_str()
            .expect("A cargo manifest dir must be specified."),
        false,
    )?;
    let mut windows = reader.as_windows()?.into_sync();
    let events = windows.next_delta(10_000)?;

    assert!(!events.is_empty());
    assert!(events.iter().all(|event| event.t < 10_000));
    Ok(())
}

#[test]
fn test_sync_event_window_iterator_returns_partial_window_at_eof() -> TestResult {
    let file_path = sample_raw_path();
    let mut reader = RawFileReader::<4_096>::try_from_file(
        file_path
            .to_str()
            .expect("A cargo manifest dir must be specified."),
        false,
    )?;
    let mut windows = reader.as_windows()?.into_sync();
    let events = windows.next_delta(1_000_000_000)?;

    assert!(!events.is_empty());
    Ok(())
}
