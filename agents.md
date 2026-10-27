# AGENTS.md

## 1. System Architecture & Crate Layout

`openevt` provides hardware abstraction layers, event stream decoders, and device plugin interfaces for event-based vision sensors.

```
openevt/
├── crates/
│   ├── openevt-core/        # Fundamental types, traits, and ABI definitions
│   ├── openevt-decoder/     # Event stream decoders (EVT3.0, etc.)
│   ├── openevt-ffi/         # C/C++ ABI-compatible bindings
│   └── openevt-py/          # Optional PyO3 Python bindings
```

### Core Architectural Patterns

- **Facility Traits (`openevt-core`)**: Hardware and sensor capabilities (e.g., streaming, illumination control, trigger input/output, ROI selection) are exposed strictly through Rust traits called *facilities*. Device abstractions query and expose capabilities by returning option/result handles to these facility traits.
- **ABI-Safe Plugin Model**: Hardware drivers and device implementations are compiled as dynamic plugins. Communication across dynamic library boundaries follows explicit FFI/ABI types defined in `openevt-core` to guarantee cross-compiler and cross-language stability.
- **Event Decoders (`openevt-decoder`)**: Raw event stream decoders (e.g., EVT3.0) parse incoming byte buffers into structured event formats with minimal overhead, maintaining independence from device driver crates.

---

## 2. Feature Development Workflow (TDD Standard)

All feature development and implementation within `openevt` must strictly follow a Test-Driven Development (TDD) cycle. Code must never be written before tests are in place to ensure tests validate expectations rather than mirroring implementation details.

### Development Steps

1. **Step 1: Understand the Task**
   - Thoroughly review requirements, system implications, and API constraints before writing any code or tests.
2. **Step 2: Conceptualize and Create Unit Tests (Test-First)**
   - Draft and implement appropriate unit tests reflecting expected behavior, edge cases, and API guarantees.
   - **Mandatory Rule**: Tests must always be written **before** writing the feature implementation so tests are never written to fit existing code.
3. **Step 3: Write the Code**
   - Implement the feature or fix targeting the requirements validated by the unit tests.
4. **Step 4: Check if Code Passes Tests**
   - Execute the test suite against the target feature set:
     ```bash
     cargo test --features full
     ```
5. **Step 5: Evaluate Failures and Repeat**
   - If tests fail, analyze the root cause, refine the implementation or test expectations where appropriate, and re-run until all tests pass cleanly.

---

## 3. PyO3 Feature-Gating Architecture

This repository strictly isolates domain features from Python bindings using a single `python` feature flag combined with Rust conditional compilation.

### Cargo Feature Hierarchy Guidelines

- **Domain Features**: Core logic features (`evt3`, `decoder`, `full`) must remain agnostic of PyO3 and Python runtime dependencies.
- **PyO3 Isolation**: PyO3 is configured as an optional dependency tied strictly to the `python` feature flag in `Cargo.toml`.

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
pyo3 = { version = "0.21", optional = true, features = ["extension-module"] }

[features]
default = []
evt3 = []
decoder = []
full = ["evt3", "decoder"]
python = ["dep:pyo3"]
```

### Code Gating Structure

- **Core Logic**: Domain logic lives in `crates/openevt-core` (types, HAL,
  algorithms) and `crates/openevt-devices` (raw-file device), guarded by
  domain features (e.g., `#[cfg(feature = "evt3")]`).
- **Bindings Crate**: All PyO3 bindings live in the dedicated `openevt-py`
  workspace crate (`crates/openevt-py`), gated by `#[cfg(feature = "python")]`.
  The domain crates never link PyO3.
- **Granular Guards**: Feature gates apply inside the `python` module of
  `openevt-py` for individual functions, classes, and module registrations
  using `#[cfg(feature = "...")]`.

```rust
// Core domain logic (openevt-core, agnostic of PyO3)
pub mod evt3 {
    #[cfg(feature = "evt3")]
    pub fn decode_evt3_stream(data: &[u8]) -> usize {
        data.len()
    }
}

// Dedicated Python bindings crate (crates/openevt-py)
#[cfg(feature = "python")]
mod py_bindings {
    use openevt_core::evt3::decode_evt3_stream;
    use pyo3::prelude::*;

    #[cfg(feature = "evt3")]
    #[pyfunction]
    fn decode_evt3_stream_py(data: &[u8]) -> PyResult<usize> {
        Ok(decode_evt3_stream(data))
    }

    #[pymodule]
    fn _openevt(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add("HAS_EVT3", cfg!(feature = "evt3"))?;
        m.add("HAS_DECODER", cfg!(feature = "decoder"))?;

        #[cfg(feature = "evt3")]
        m.add_function(wrap_pyfunction!(decode_evt3_stream_py, m)?)?;

        Ok(())
    }
}
```

### PyPI Wheel Build Commands

When compiling wheels via `maturin`, combine the `python` feature flag with target core domain features:

```bash
# EVT3 Only Wheel
maturin build --release --features "python, evt3"

# Decoder Only Wheel
maturin build --release --features "python, decoder"

# Full Feature Set Wheel
maturin build --release --features "python, full"
```

### Python API Fallback Patterns

When exposing APIs where Python introspection or uniform interfaces are needed across feature builds:

- **Constant Flags**: Expose compile-time constants (e.g., `m.add("HAS_EVT3", cfg!(feature = "evt3"))?`).
- **Explicit Exceptions**: Provide fallback stubs raising `PyNotImplementedError` when optional domain features are omitted from a build.

```rust
#[cfg(feature = "python")]
#[cfg(not(feature = "evt3"))]
#[pyfunction]
fn decode_evt3_stream(_data: &[u8]) -> PyResult<usize> {
    Err(pyo3::exceptions::PyNotImplementedError::new_err(
        "EVT3 decoding feature was not compiled into this binary.",
    ))
}
```

---

## 4. Git Workflow Rules

1. **Rule 1: `main` is Always Passing**  
   The `main` branch must remain green at all times. All CI pipelines must pass completely before merging code.
2. **Rule 2: `main` Contains Latest Features**  
   `main` serves as the active integration branch containing validated features, which may be unreleased or undergoing active development.
3. **Rule 3: Branch Naming Convention**  
   All work must take place on dedicated branches using the prefix `feat/<short-description>` or `fix/<short-description>`. Direct commits to `main` are restricted.
4. **Rule 4: Pull Requests Required**  
   Code is merged into `main` strictly through formal Pull Requests (PRs) after satisfying CI build and test steps.
5. **Rule 5: Conventional Commit Standard**  
   All commits must adhere strictly to the Conventional Commits specification:
   - `feat: add EVT3 stream payload header parsing`
   - `fix: handle timestamp rollover in ring buffer decoder`
   - `refactor: update facility trait dynamic dispatch lookup`
   - `docs: detail plugin interface ABI invariants`

---

## Verification & Execution Checklist

1. Save the block above directly to `AGENTS.md` at the workspace root of `openevt`.
2. Ensure new feature developments follow the 5-step TDD cycle (tests created prior to writing feature code).
3. Run `cargo check --workspace --exclude openevt-py` to verify core facilities build cleanly without Python runtime overhead.
4. Run `cargo check -p openevt --features python,full` to confirm feature-gated PyO3 binding integration.
