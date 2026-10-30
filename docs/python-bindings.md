# Python bindings

The `openevt` PyPI package wraps the Rust bindings in `crates/openevt-py`. It
ships type stubs and a `py.typed` marker, so type checkers resolve the API
from the installed wheel.

```bash
python -m pip install openevt
```

## Plugin requirement

The Python `RawFileReader` works through the device-plugin ABI. It loads
plugins from `OPENEVT_PLUGIN_PATH` and raises `RuntimeError` when no
compatible plugin exists. Build the standard plugins and point the variable at
the output directory:

```sh
cargo build --release -p openevt-plugins
export OPENEVT_PLUGIN_PATH="$PWD/target/release"
```

## Reading a raw file

The constructor requires a path; `RawFileReader()` without one raises
`ValueError`. Set `index=True` to enable `seek` and `t_min`/`t_max`.

```python
import openevt

reader = openevt.RawFileReader("events.raw", index=True)
iterator = reader.iter()

batch = iterator.next_batch()
while batch is not None:
    process(batch)
    batch = iterator.next_batch()
```

Batch methods (`next_batch`, `next_n`, `next_delta`) return a structured
NumPy array or `None` at end of file. The `for` protocol still works:
`__next__` raises `StopIteration` when the stream ends. `reader.aiter()`
returns an async iterator; its `__anext__` raises `StopAsyncIteration`.

Batches use the dtype `{"names": ("x", "y", "p", "t"), "formats": ("u8",
"u8", "u1", "u8"), "offsets": (0, 8, 16, 24)}`. Field `p` holds polarity as
`0` (negative) or `1` (positive), matching the Rust `Polarity` enum.

## Factory-only classes

`CDEventReceiver`, `CDEventIterator`, `AsyncCDEventIterator`, and `Tensor`
have no Python constructor. Create them through their factories:
`reader.cd_receiver()`, `reader.iter()`, `reader.aiter()`, and
`surface.to_tensor(dual)`. Calling them directly raises `TypeError`, and the
stubs mark them non-instantiable.

## Algorithms

`openevt.algorithms` exposes filters (`PolarityFilter`, `RoiFilter`,
`ActivityNoiseFilter`, `SpatioTemporalContrastFilter`, `TrailFilter`,
`AntiFlickerFilter`), time surfaces (`BaseTimeSurface`,
`LinearDecayTimeSurface`, `ExponentialDecayTimeSurface`), accumulators
(`Histogram`, `IntegralImage`), `Tensor`, and the matching transformers.
Time surfaces, histograms, and integral images accept `p: bool` for polarity.
`Tensor.to_numpy()` returns a `float64` array.

## Simulator bindings

The separate `openevt_plugins` package (built from the `openevt-plugins`
crate with the `python` feature) exposes `Simulator(video_path,
config_toml=None, width=160, height=90)`. Its `next_batch()` returns a list of
`(x, y, polarity, timestamp)` tuples and raises `IOError` at the end of the
video.
