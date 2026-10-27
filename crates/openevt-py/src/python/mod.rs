pub mod pyalgorithms;
pub mod pydevice;

use pyo3::prelude::*;

/// Python module entry point.
#[pymodule]
fn openevt(m: &Bound<'_, PyModule>) -> PyResult<()> {
    pydevice::register(m)?;
    pyalgorithms::register(m)
}
