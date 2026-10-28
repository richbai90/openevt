use openevt_abi::discovery::PluginRegistry;
use openevt_abi::plugin::{DevicePluginBox, EventBatchSink, EventBatchSink_TO};
use openevt_core::types::{EventCD, EventCount, EventTimestamp, Polarity};
use abi_stable::{std_types::RSlice, type_level::downcasting::TD_Opaque};
use crossbeam::channel::{Receiver, Sender};
use numpy::{Element, PyArray1, PyArrayDescr, PyReadonlyArray1};
use pyo3::{
    exceptions::{PyIOError, PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyDict, PyModule},
};
use std::{
    mem::{offset_of, size_of},
    sync::{Arc, Mutex},
};

type EventReceiver = Arc<Mutex<Receiver<Vec<EventCD>>>>;

const PYTHON_BUFFER_SIZE: usize = 131_072;

struct PyEventSink {
    sender: Sender<Vec<EventCD>>,
}

impl EventBatchSink for PyEventSink {
    fn on_cd_events(&self, events: RSlice<'_, EventCD>) {
        let _ = self.sender.send(events.to_vec());
    }

    fn on_ext_events(&self, _events: RSlice<'_, openevt_core::types::EventExtTrigger>) {}
}

struct PluginReader {
    // Keep the loaded module alive for the lifetime of the ABI trait object.
    _registry: PluginRegistry,
    device: Arc<Mutex<DevicePluginBox>>,
    receiver: EventReceiver,
    shape: (u32, u32),
    t_min: Option<EventTimestamp>,
    t_max: Option<EventTimestamp>,
}

impl PluginReader {
    fn open(path: Option<&str>, _index: bool) -> PyResult<Self> {
        let mut registry = PluginRegistry::new();
        if registry.load_default_paths() == 0 {
            return Err(PyRuntimeError::new_err(
                "no compatible device plugins were found; configure OPENEVT_PLUGIN_PATH",
            ));
        }

        let serial = path.ok_or_else(|| {
            PyValueError::new_err("a raw-file path is required when using plugin-backed input")
        })?;
        let mut device = registry
            .open_device(serial)
            .map_err(PyRuntimeError::new_err)?;
        let (sender, receiver) = crossbeam::channel::unbounded();
        let sink = EventBatchSink_TO::from_value(PyEventSink { sender }, TD_Opaque);
        device
            .start_events(sink)
            .into_result()
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
        let geometry = device.geometry();
        let t_min = device.t_min().into_option();
        let t_max = device.t_max().into_option();

        Ok(Self {
            _registry: registry,
            device: Arc::new(Mutex::new(device)),
            receiver: Arc::new(Mutex::new(receiver)),
            shape: (geometry.height, geometry.width),
            t_min,
            t_max,
        })
    }

    fn load_batch(&self) -> PyResult<()> {
        self.device
            .lock()
            .map_err(|_| PyRuntimeError::new_err("plugin device lock poisoned"))?
            .load_batch()
            .into_result()
            .map_err(|error| PyIOError::new_err(error.to_string()))
    }

    fn seek(&self, timestamp: EventTimestamp) -> PyResult<()> {
        self.device
            .lock()
            .map_err(|_| PyRuntimeError::new_err("plugin device lock poisoned"))?
            .seek(timestamp)
            .into_result()
            .map_err(|error| PyIOError::new_err(error.to_string()))
    }

    fn seek_to_next_ext(&self) -> PyResult<()> {
        self.device
            .lock()
            .map_err(|_| PyRuntimeError::new_err("plugin device lock poisoned"))?
            .seek_to_next_ext()
            .into_result()
            .map_err(|error| PyIOError::new_err(error.to_string()))
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
#[pyclass(from_py_object)]
pub struct PyEventCD {
    #[pyo3(get, set)]
    x: u64,
    #[pyo3(get, set)]
    y: u64,
    #[pyo3(get, set)]
    p: u8,
    #[pyo3(get, set)]
    t: EventTimestamp,
}

#[pymethods]
impl PyEventCD {
    #[new]
    fn new(x: u64, y: u64, p: bool, t: EventTimestamp) -> Self {
        Self {
            x,
            y,
            p: p.into(),
            t,
        }
    }
}

impl From<EventCD> for PyEventCD {
    fn from(value: EventCD) -> Self {
        Self {
            x: value.x,
            y: value.y,
            p: value.p as u8,
            t: value.t,
        }
    }
}

impl From<PyEventCD> for EventCD {
    fn from(value: PyEventCD) -> Self {
        EventCD::new(value.x, value.y, Polarity::from(value.p != 0), value.t)
    }
}

impl From<&EventCD> for PyEventCD {
    fn from(value: &EventCD) -> Self {
        Self {
            x: value.x,
            y: value.y,
            p: value.p as u8,
            t: value.t,
        }
    }
}

// The unsafe Element impl below treats PyEventCD arrays as interchangeable
// with EventCD layouts, so byte-compatibility is asserted, not assumed.
const _: () = {
    assert!(size_of::<PyEventCD>() == size_of::<EventCD>());
    assert!(offset_of!(PyEventCD, x) == offset_of!(EventCD, x));
    assert!(offset_of!(PyEventCD, y) == offset_of!(EventCD, y));
    assert!(offset_of!(PyEventCD, p) == offset_of!(EventCD, p));
    assert!(offset_of!(PyEventCD, t) == offset_of!(EventCD, t));
};

// Safety: PyEventCD is #[repr(C)] and byte-compatible with EventCD, enforced
// by the layout assertions above. Every field is a plain integer: `p` stays
// `u8` rather than `Polarity` because numpy arrays can contain arbitrary
// bytes, and only 0 and 1 are valid `Polarity` discriminants. Conversions
// therefore clamp through `!= 0` on input and `as u8` on output, so no
// invalid byte value can ever be reinterpreted as a `Polarity`.
unsafe impl Element for PyEventCD {
    const IS_COPY: bool = true;
    fn vec_from_slice(py: Python<'_>, slc: &[Self]) -> Vec<Self> {
        slc.iter().map(|elem| elem.clone_ref(py)).collect()
    }

    fn array_from_view<D>(
        py: Python<'_>,
        view: numpy::ndarray::ArrayView<'_, Self, D>,
    ) -> numpy::ndarray::Array<Self, D>
    where
        D: numpy::ndarray::Dimension,
    {
        view.map(|elem| elem.clone_ref(py))
    }

    fn get_dtype(py: Python<'_>) -> Bound<'_, numpy::PyArrayDescr> {
        // Building the structured dtype calls into Python; cache it so each
        // decoded batch does not re-import numpy and rebuild the descriptor.
        static DTYPE: std::sync::OnceLock<pyo3::Py<PyArrayDescr>> = std::sync::OnceLock::new();
        let cached = DTYPE.get_or_init(|| {
            let np = py.import("numpy").expect("Failed to import numpy");
            let dtype_spec = PyDict::new(py);
            dtype_spec.set_item("names", ("x", "y", "p", "t")).unwrap();
            dtype_spec
                .set_item("formats", ("u8", "u8", "u1", "u8"))
                .unwrap();
            dtype_spec
                .set_item(
                    "offsets",
                    (
                        offset_of!(PyEventCD, x),
                        offset_of!(PyEventCD, y),
                        offset_of!(PyEventCD, p),
                        offset_of!(PyEventCD, t),
                    ),
                )
                .unwrap();
            dtype_spec
                .set_item("itemsize", size_of::<PyEventCD>())
                .unwrap();

            np.call_method1("dtype", (dtype_spec,))
                .unwrap()
                .cast_into::<PyArrayDescr>()
                .unwrap()
                .unbind()
        });
        cached.bind(py).clone()
    }

    fn clone_ref(&self, _py: Python<'_>) -> Self {
        *self
    }
}

#[pyclass(name = "CDEventReceiver")]
pub struct PyCDEventReceiver {
    inner: EventReceiver,
}

#[pymethods]
impl PyCDEventReceiver {
    /// Receive one already-decoded batch without waiting.
    fn try_recv<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        let receiver = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("event receiver lock poisoned"))?;
        match receiver.try_recv() {
            Ok(events) if !events.is_empty() => Ok(Some(events_to_numpy(py, &events))),
            Ok(_) => Ok(None),
            Err(crossbeam::channel::TryRecvError::Empty) => Ok(None),
            Err(crossbeam::channel::TryRecvError::Disconnected) => Ok(None),
        }
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.try_recv(py)?
            .ok_or_else(|| pyo3::exceptions::PyStopIteration::new_err(()))
    }

    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let events = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("event receiver lock poisoned"))?
            .recv()
            .map_err(|err| PyRuntimeError::new_err(err.to_string()))?;
        async_result(
            py,
            if events.is_empty() {
                None
            } else {
                Some(events_to_numpy(py, &events))
            },
        )
    }
}

pub(crate) fn events_to_numpy<'py>(py: Python<'py>, events: &[EventCD]) -> Bound<'py, PyAny> {
    // `PyEventCD` is `Copy`, so `from_vec` performs one memcpy into the freshly
    // allocated numpy buffer after the (vectorizable) field conversion.
    PyArray1::from_vec(py, events.iter().map(PyEventCD::from).collect()).into_any()
}

pub(crate) fn events_from_numpy(events: PyReadonlyArray1<'_, PyEventCD>) -> PyResult<Vec<EventCD>> {
    let slice = events
        .as_slice()
        .map_err(|_| PyValueError::new_err("events array must be contiguous"))?;
    Ok(slice.iter().copied().map(EventCD::from).collect())
}

fn async_result<'py>(
    py: Python<'py>,
    value: Option<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    if let Some(value) = value {
        py.import("asyncio")?.getattr("sleep")?.call1((0, value))
    } else {
        Err(pyo3::exceptions::PyStopAsyncIteration::new_err(()))
    }
}

#[pyclass(name = "CDEventIterator")]
pub struct PyCDEventIterator {
    device: Arc<Mutex<DevicePluginBox>>,
    receiver: EventReceiver,
    // Current decoded batch and the cursor of consumed events inside it.
    // Consuming events only advances the cursor, so batches move from the
    // channel into the iterator without per-event copying.
    internal_buffer: Vec<EventCD>,
    internal_cursor: usize,
    // Reused output staging so each call does not allocate a fresh batch Vec.
    // Staging holds numpy elements so the conversion happens exactly once:
    // channel batch into staging, then one memcpy from staging into numpy.
    scratch: Vec<PyEventCD>,
    current_timestamp: Option<EventTimestamp>,
    shape: (u32, u32),
}

#[pymethods]
impl PyCDEventIterator {
    /// Return this iterator, allowing `for batch in reader.iter()`.
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (count, _ended) = self.take_up_to(PYTHON_BUFFER_SIZE)?;
        if count == 0 {
            return Err(pyo3::exceptions::PyStopIteration::new_err(()));
        }
        Ok(PyArray1::from_slice(py, &self.scratch[..count]).into_any())
    }

    fn next_batch<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (count, ended) = self.take_up_to(PYTHON_BUFFER_SIZE)?;
        if count == 0 && ended {
            return Err(PyIOError::new_err("End of file reached"));
        }
        Ok(PyArray1::from_slice(py, &self.scratch[..count]).into_any())
    }

    /// Return up to `n` events while preserving any remaining events.
    fn next_n<'py>(&mut self, py: Python<'py>, n: EventCount) -> PyResult<Bound<'py, PyAny>> {
        let limit = usize::try_from(n).map_err(|_| {
            PyValueError::new_err("requested event count does not fit this platform")
        })?;
        assert!(
            limit < PYTHON_BUFFER_SIZE,
            "requested event count must be smaller than the iterator buffer size"
        );
        let (count, ended) = self.take_up_to(limit)?;
        if count == 0 && ended {
            return Err(PyIOError::new_err("End of file reached"));
        }
        Ok(PyArray1::from_slice(py, &self.scratch[..count]).into_any())
    }

    /// Return the next time window containing events within `dt` timestamp
    /// units of the iterator's current time baseline.
    fn next_delta<'py>(
        &mut self,
        py: Python<'py>,
        dt: EventTimestamp,
    ) -> PyResult<Bound<'py, PyAny>> {
        let count = self.take_delta(dt)?;
        Ok(PyArray1::from_slice(py, &self.scratch[..count]).into_any())
    }

    fn shape(&self) -> (u32, u32) {
        self.shape
    }
}

impl PyCDEventIterator {
    /// Number of decoded events not yet consumed from the current batch.
    fn remaining(&self) -> usize {
        self.internal_buffer.len() - self.internal_cursor
    }

    /// Refills the current batch, loading raw data on demand.
    ///
    /// Returns `true` when the stream reached its end. A partial batch is kept
    /// for the caller to return before the end-of-file error surfaces, matching
    /// the native iterator's documented behavior.
    fn replenish(&mut self) -> PyResult<bool> {
        if self.remaining() > 0 {
            return Ok(false);
        }
        loop {
            if let Ok(events) = self
                .receiver
                .lock()
                .map_err(|_| PyRuntimeError::new_err("event receiver lock poisoned"))?
                .try_recv()
            {
                self.internal_buffer = events;
                self.internal_cursor = 0;
                return Ok(false);
            }
            let mut device = self
                .device
                .lock()
                .map_err(|_| PyRuntimeError::new_err("plugin device lock poisoned"))?;
            match device.load_batch().into_result() {
                Ok(()) => {}
                Err(error) => {
                    let message = error.to_string();
                    if message == "End of file reached" {
                        return Ok(true);
                    }
                    return Err(PyIOError::new_err(message));
                }
            }
        }
    }

    /// Collect up to `n` events into `scratch`.
    ///
    /// Returns the event count and whether the stream ended. A partial batch is
    /// still delivered when the stream ends mid-collection.
    fn take_up_to(&mut self, n: usize) -> PyResult<(usize, bool)> {
        self.scratch.clear();
        let mut ended = false;
        while self.scratch.len() < n {
            let eof = self.replenish()?;
            if self.remaining() == 0 {
                ended = eof;
                break;
            }
            if self.current_timestamp.is_none() {
                self.current_timestamp = Some(self.internal_buffer[self.internal_cursor].t);
            }
            let take = (n - self.scratch.len()).min(self.remaining());
            let start = self.internal_cursor;
            self.scratch.extend(
                self.internal_buffer[start..start + take]
                    .iter()
                    .map(PyEventCD::from),
            );
            self.internal_cursor += take;
        }
        Ok((self.scratch.len(), ended))
    }

    fn take_delta(&mut self, dt: EventTimestamp) -> PyResult<usize> {
        self.scratch.clear();
        if self.replenish()? {
            return Err(PyIOError::new_err("End of file reached"));
        }
        let start_ts = match self.current_timestamp {
            Some(ts) => ts,
            None => self
                .internal_buffer
                .get(self.internal_cursor)
                .map(|event| event.t)
                .unwrap_or(0),
        };
        self.current_timestamp = Some(start_ts);
        let end_ts = start_ts + dt;
        loop {
            if self.replenish()? {
                break;
            }
            // Decoded batches are timestamp-ordered, so the window is a prefix.
            let rest = &self.internal_buffer[self.internal_cursor..];
            let take = rest.partition_point(|event| event.t < end_ts);
            if take == 0 {
                break;
            }
            self.scratch.extend(rest[..take].iter().map(PyEventCD::from));
            self.internal_cursor += take;
        }
        self.current_timestamp = Some(end_ts);
        Ok(self.scratch.len())
    }
}

#[pyclass(name = "AsyncCDEventIterator")]
pub struct PyAsyncCDEventIterator {
    inner: EventReceiver,
    shape: (u32, u32),
}

#[pymethods]
impl PyAsyncCDEventIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let events = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("event receiver lock poisoned"))?
            .recv()
            .map_err(|err| PyRuntimeError::new_err(err.to_string()))?;
        async_result(
            py,
            if events.is_empty() {
                None
            } else {
                Some(events_to_numpy(py, &events))
            },
        )
    }

    fn next_batch<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.__anext__(py)
    }

    fn shape(&self) -> PyResult<(u32, u32)> {
        Ok(self.shape)
    }
}

#[pyclass(name = "RawFileReader")]
pub struct PyRawFileReader {
    inner: PluginReader,
}

#[pymethods]
impl PyRawFileReader {
    #[new]
    #[pyo3(signature = (path = None, index = false))]
    fn new(path: Option<&str>, index: bool) -> PyResult<Self> {
        Ok(PyRawFileReader {
            inner: PluginReader::open(path, index)?,
        })
    }

    fn try_open(&mut self, file_path: &str, index: bool) -> PyResult<()> {
        self.inner = PluginReader::open(Some(file_path), index)?;
        Ok(())
    }

    fn ready(&self) -> bool {
        true
    }

    fn t_min(&self) -> Option<EventTimestamp> {
        self.inner.t_min
    }

    fn t_max(&self) -> Option<EventTimestamp> {
        self.inner.t_max
    }

    fn seek(&mut self, ts: EventTimestamp) -> PyResult<()> {
        self.inner.seek(ts)?;
        Ok(())
    }

    fn seek_to_next_ext(&mut self) -> PyResult<()> {
        self.inner.seek_to_next_ext()?;
        Ok(())
    }

    fn load_batch(&mut self) -> PyResult<()> {
        self.inner.load_batch()
    }

    fn cd_receiver(&mut self) -> PyResult<PyCDEventReceiver> {
        Ok(PyCDEventReceiver {
            inner: self.inner.receiver.clone(),
        })
    }

    /// Create a synchronous batch iterator.
    fn iter(&mut self) -> PyResult<PyCDEventIterator> {
        Ok(PyCDEventIterator {
            device: self.inner.device.clone(),
            receiver: self.inner.receiver.clone(),
            internal_buffer: Vec::new(),
            internal_cursor: 0,
            scratch: Vec::new(),
            current_timestamp: None,
            shape: self.inner.shape,
        })
    }

    /// Create an asynchronous batch iterator.
    fn aiter(&mut self) -> PyResult<PyAsyncCDEventIterator> {
        let shape = self.inner.shape;
        Ok(PyAsyncCDEventIterator {
            inner: self.inner.receiver.clone(),
            shape,
        })
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyEventCD>()?;
    module.add("EventCD", module.getattr("PyEventCD")?)?;
    module.add_class::<PyCDEventReceiver>()?;
    module.add_class::<PyCDEventIterator>()?;
    module.add_class::<PyAsyncCDEventIterator>()?;
    module.add_class::<PyRawFileReader>()?;
    Ok(())
}
