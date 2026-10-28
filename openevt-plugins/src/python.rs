use crate::simulator::plugin::VideoSimulator;
use crate::simulator::solver::EvsParameters;
use openevt_core::types::EventTimestamp;
use pyo3::exceptions::{PyIOError, PyValueError};
use pyo3::prelude::*;

#[pyclass(unsendable)]
struct Simulator {
    inner: VideoSimulator,
}

#[pymethods]
impl Simulator {
    #[new]
    #[pyo3(signature = (video_path, config_toml = None, width = 160, height = 90))]
    fn new(
        video_path: String,
        config_toml: Option<String>,
        width: u64,
        height: u64,
    ) -> PyResult<Self> {
        if width == 0 || height == 0 {
            return Err(PyValueError::new_err("width and height must be positive"));
        }
        let width = usize::try_from(width)
            .map_err(|_| PyValueError::new_err("width is too large for this platform"))?;
        let height = usize::try_from(height)
            .map_err(|_| PyValueError::new_err("height is too large for this platform"))?;
        let params = match config_toml {
            Some(source) => toml::from_str::<EvsParameters>(&source)
                .map_err(|error| PyValueError::new_err(error.to_string()))?,
            None => EvsParameters::default(),
        };
        let inner = VideoSimulator::open(&video_path, None, params, width, height)
            .map_err(PyIOError::new_err)?;
        Ok(Self { inner })
    }

    /// Simulate and return one decoded frame as (x, y, polarity, timestamp).
    fn next_batch(&mut self) -> PyResult<Vec<(u64, u64, bool, EventTimestamp)>> {
        let events = self
            .inner
            .next_events_batch()
            .map_err(PyIOError::new_err)?
            .ok_or_else(|| PyIOError::new_err("end of simulator video"))?;
        Ok(events
            .into_iter()
            .map(|event| (event.x, event.y, bool::from(event.p), event.t))
            .collect())
    }

    fn seek(&mut self, timestamp: EventTimestamp) -> PyResult<()> {
        self.inner.seek(timestamp).map_err(PyIOError::new_err)
    }

    #[getter]
    fn t_max(&self) -> EventTimestamp {
        self.inner.duration_us
    }
}

#[pymodule]
fn openevt_plugins(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Simulator>()?;
    Ok(())
}
