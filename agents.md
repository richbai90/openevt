# AGENTS.md

## 1. System Architecture & Crate Layout

`openevt` provides hardware abstraction layers, event stream decoders, and device plugin interfaces for event-based vision sensors.

```
openevt/
├── Cargo.toml                 # workspace + facade package `openevt`
├── src/lib.rs                 # facade re-exporting the workspace crates
├── crates/
│   ├── openevt-core/          # types, HAL traits/facilities, decoders, algorithms
│   ├── openevt-devices/       # file-backed raw event device (RawFileReader)
│   ├── openevt-abi/           # stable plugin ABI, registry, host adapters, framegen
│   └── openevt-py/            # optional PyO3 Python bindings
├── openevt-plugins/           # raw-file replay and video simulator plugins
├── openevt-plugin-lucid-evs/  # Lucid Triton EVS network device plugin
├── python/openevt/            # Python package with type stubs (.pyi) and py.typed
├── examples/                  # probe_plugins host example
└── tests/                     # integration tests, including Python bindings
```

### Core Architectural Patterns

- **Facility Traits (`openevt-core`)**: The HAL exposes hardware and sensor capabilities (streaming, illumination control, trigger input/output, ROI selection) strictly through Rust traits called *facilities*. Device abstractions query and expose capabilities by returning option/result handles to these facility traits.
- **ABI-Safe Plugin Model**: Compile device drivers as dynamic plugins. The plugin ABI, registry, and host-side adapters live in `openevt-abi` and use `abi_stable` types to guarantee cross-compiler and cross-language stability. A device plugin exports the root module `openevt_device_plugin`; a frame-generation plugin exports `frame_generator_plugin`.
- **Event Decoders (`openevt-core::hal::decoders`)**: Raw event stream decoders parse incoming byte buffers into structured events with minimal overhead. The `evt3` and `decoder` features gate them. EVT3 is the only path wired into the raw device; EVT2, DAT, and HDF5 decoding remain unfinished.
- **End-of-File Convention**: Stream reads return `Option`. Rust returns `Ok(None)` at end of file, the plugin ABI returns `ROk(RNone)`, and Python batch methods return `None` while `__next__` raises `StopIteration`.
- **Polarity**: `Polarity` is a `#[repr(u8)]` enum (`Negative = 0`, `Positive = 1`) carried inside `EventCD`. `TryFrom<u8>` rejects any byte outside `{0, 1}`, so invalid polarity cannot cross the ABI boundary. Python exposes polarity as a `bool` in constructors and as a `u8` field in NumPy batches.

### Feature Flags (root `Cargo.toml`)

```toml
[features]
default = ["all"]
all = ["devices", "framegen", "plugins", "evt3", "decoder"]
# Alias kept for the verification commands below.
full = ["all"]
devices = ["dep:openevt-devices", "openevt-core/full"]
python = ["devices", "plugins", "dep:openevt-py", "openevt-py/python"]
framegen = ["dep:openevt-abi", "openevt-abi/framegen"]
plugins = ["dep:openevt-abi"]
evt3 = ["openevt-core/evt3"]
decoder = ["openevt-core/decoder"]
```

The `all` feature excludes `python`. Request `python` explicitly for a bindings build.

---

## 2. Feature Development Workflow (TDD Standard)

All feature development and implementation within `openevt` must follow a Test-Driven Development (TDD) cycle. Never write code before the tests are in place, so tests validate expectations rather than mirror implementation details.

### Development Steps

1. **Step 1: Understand the Task**
   - Review requirements, system implications, and API constraints before writing any code or tests.
2. **Step 2: Conceptualize and Create Unit Tests (Test-First)**
   - Draft and implement unit tests reflecting expected behavior, edge cases, and API guarantees.
   - **Mandatory Rule**: Write tests **before** the feature implementation, so tests never fit existing code.
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

## 3. Python Bindings (`openevt-py`)

All PyO3 code lives in the dedicated `crates/openevt-py` workspace crate and is gated behind the single `python` feature. Domain crates never link PyO3.

- PyO3 0.29 with the `abi3-py39`, `extension-module`, and `experimental-async` features.
- The compiled extension module is named `openevt`. The pure-Python package in `python/openevt/` re-exports it and ships type stubs (`__init__.pyi`, `openevt.pyi`, `algorithms/*.pyi`) plus `py.typed`.
- `pyproject.toml` fixes the wheel build to `features = ["python", "full"]` with `module-name = "openevt"` and `manifest-dir = "crates/openevt-py"`. Build wheels from the repository root:

```bash
maturin build --release
```

The bindings expose the device classes (`EventCD`, `RawFileReader`, `CDEventReceiver`, `CDEventIterator`, `AsyncCDEventIterator`) and the filter and time-surface algorithms under `openevt.algorithms`.

### API Conventions

- **Plugin-backed reader**: `RawFileReader(path, index=False)` requires a path and loads devices through the plugin ABI. Without a compatible plugin on `OPENEVT_PLUGIN_PATH` it raises `RuntimeError`; without a path it raises `ValueError`.
- **End of file**: `next_batch()`, `next_n(n)`, and `next_delta(dt)` return a structured NumPy array or `None` at end of file. `__next__` raises `StopIteration`; `CDEventReceiver.try_recv()` returns `None` when nothing is queued.
- **Factory-only classes**: `CDEventReceiver`, `CDEventIterator`, `AsyncCDEventIterator`, and `Tensor` have no Python constructor. Host APIs (`RawFileReader.iter()`, `RawFileReader.aiter()`, `to_tensor()`) create them, and the stubs mark them non-instantiable.
- **No feature-introspection constants**: the module registers no `HAS_*` flags and no `PyNotImplementedError` fallback stubs; the `python` feature is the only gate.

---

## 4. Git Workflow Rules

1. **Rule 1: `main` is Always Passing**
   The `main` branch must remain green at all times. All CI pipelines must pass before merging code.
2. **Rule 2: `main` Contains Latest Features**
   `main` serves as the active integration branch containing validated features, which may be unreleased or undergoing active development.
3. **Rule 3: Branch Naming Convention**
   All work must take place on dedicated branches using the prefix `feat/<short-description>`, `fix/<short-description>`, or `docs/<short-description>`. Direct commits to `main` are restricted.
4. **Rule 4: Pull Requests Required**
   Merge code into `main` only through formal Pull Requests (PRs), once the CI build and test steps pass.
5. **Rule 5: Conventional Commit Standard**
   All commits must follow the Conventional Commits specification:
   - `feat: add EVT3 stream payload header parsing`
   - `fix: handle timestamp rollover in ring buffer decoder`
   - `refactor: update facility trait dynamic dispatch lookup`
   - `docs: detail plugin interface ABI invariants`

---

## Verification & Execution Checklist

1. Run `cargo test --features full` after any feature change; it covers the workspace default feature set without linking the Python extension module.
2. Run `cargo check --workspace --exclude openevt-py` to verify core facilities build cleanly without Python runtime overhead.
3. Run `cargo check -p openevt --features python,full` to confirm feature-gated PyO3 binding integration.
4. Build the debug extension and plugins with `cargo build --features python`, then run `pytest tests/python` for the Python API tests.
5. Run `cargo test -p openevt-plugins --test plugin_discovery` to verify plugin discovery.
