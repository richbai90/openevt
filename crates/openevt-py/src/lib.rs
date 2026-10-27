//! PyO3 Python bindings for the openevt stack.
//!
//! All PyO3 bindings live in this edge-of-the-dependency-tree crate. The
//! bindings compile only when the `python` feature is enabled; domain logic
//! stays in `openevt-core` and `openevt-devices`, agnostic of PyO3.
//!
//! Wheel builds go through maturin; see the workspace root `pyproject.toml`.

#[cfg(feature = "python")]
pub mod python;
