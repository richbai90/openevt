"""Python API tests for the openevt extension module.

These tests load the debug extension produced by ``cargo build`` directly, so
they do not require maturin or a separately installed wheel.
"""

import asyncio
import importlib.util
import os
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
import pytest


ROOT = Path(__file__).parents[2]
SAMPLE_RAW = ROOT / "crates" / "openevt-devices" / "tests" / "sample.raw"
EVENT_DTYPE = np.dtype(
    {
        "names": ("x", "y", "p", "t"),
        "formats": ("u8", "u8", "u1", "u8"),
        "offsets": (0, 8, 16, 24),
        "itemsize": 32,
    }
)


def events_array(rows):
    return np.array(rows, dtype=EVENT_DTYPE)


def assert_event_dtype(events):
    assert isinstance(events, np.ndarray)
    assert events.dtype.names == ("x", "y", "p", "t")
    assert events.dtype.itemsize == 32
    assert events.dtype.fields["x"][1] == 0
    assert events.dtype.fields["y"][1] == 8
    assert events.dtype.fields["p"][1] == 16
    assert events.dtype.fields["t"][1] == 24
    assert events["x"].dtype == np.dtype("u8")
    assert events["y"].dtype == np.dtype("u8")
    assert events["p"].dtype == np.dtype("u1")
    assert events["t"].dtype == np.dtype("u8")


@pytest.fixture(scope="session")
def openevt():
    subprocess.run(
        ["cargo", "build", "--quiet", "--features", "python"],
        cwd=ROOT,
        check=True,
    )
    extension = ROOT / "target" / "debug" / "libopenevt.so"
    assert extension.exists()

    # The Python extension and the raw-file plugin are built from the same
    # crate, but the plugin must be loaded as a separate shared-library
    # instance across the ABI boundary.
    plugin_dir = ROOT / "target" / "debug" / "python-plugin"
    plugin_dir.mkdir(exist_ok=True)
    shutil.copy2(extension, plugin_dir / "libopenevt_raw_file.so")
    os.environ["OPENEVT_PLUGIN_PATH"] = str(plugin_dir)

    spec = importlib.util.spec_from_file_location("openevt", extension)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules["openevt"] = module
    spec.loader.exec_module(module)
    return module


def test_module_registers_binding_classes(openevt):
    assert openevt.RawFileReader
    assert openevt.CDEventReceiver
    assert openevt.CDEventIterator
    assert openevt.AsyncCDEventIterator
    assert openevt.EventCD
    assert openevt.PolarityFilter
    assert openevt.RoiFilter
    assert openevt.ActivityNoiseFilter
    assert openevt.SpatioTemporalContrastFilter
    assert openevt.TrailFilter
    assert openevt.AntiFlickerFilter
    assert openevt.BaseTimeSurface
    assert openevt.LinearDecayTimeSurface
    assert openevt.ExponentialDecayTimeSurface
    assert openevt.BaseTimeSurfaceTransformer
    assert openevt.LinearDecayTimeSurfaceTransformer
    assert openevt.ExponentialDecayTimeSurfaceTransformer


def test_algorithm_submodules_export_shared_filter_names(openevt):
    algorithms = importlib.import_module("openevt.algorithms")
    filters = importlib.import_module("openevt.filters")
    algorithm_filters = importlib.import_module("openevt.algorithms.filters")

    for module in (openevt, algorithms, filters, algorithm_filters):
        assert module.PolarityFilter
        assert module.RoiFilter
        assert module.ActivityNoiseFilter
        assert module.SpatioTemporalContrastFilter
        assert module.TrailFilter
        assert module.AntiFlickerFilter

    assert algorithms.BaseTimeSurface
    assert algorithms.LinearDecayTimeSurface
    assert algorithms.ExponentialDecayTimeSurface
    assert algorithms.BaseTimeSurfaceTransformer
    assert algorithms.LinearDecayTimeSurfaceTransformer
    assert algorithms.ExponentialDecayTimeSurfaceTransformer


def test_polarity_filter_processes_structured_numpy_events(openevt):
    events = events_array(
        [
            (0, 0, 0, 10),
            (1, 0, 1, 20),
            (2, 0, 1, 30),
            (3, 0, 0, 40),
        ]
    )

    output = openevt.filters.PolarityFilter(True).process(events)

    assert_event_dtype(output)
    assert output["x"].tolist() == [1, 2]
    assert output["p"].tolist() == [1, 1]


def test_all_python_filters_process_events(openevt):
    events = events_array(
        [
            (0, 0, 1, 100),
            (1, 1, 1, 105),
            (2, 2, 0, 110),
            (1, 1, 1, 120),
        ]
    )

    filters = [
        openevt.RoiFilter(0, 3, 0, 3),
        openevt.ActivityNoiseFilter(4, 4, 10),
        openevt.SpatioTemporalContrastFilter(4, 4, 20, False),
        openevt.TrailFilter(4, 4, 10),
        openevt.AntiFlickerFilter(4, 4, 3, 45, 55),
    ]

    for filter_ in filters:
        output = filter_.process(events)
        assert_event_dtype(output)


def test_time_surfaces_update_and_report_values(openevt):
    base = openevt.BaseTimeSurface(2, 1)
    base.update(0, 0, True, 100)
    assert base.width() == 2
    assert base.height() == 1
    assert base.get(0, 0, True) == 100
    assert base.get(1, 0, True) == 0

    linear = openevt.LinearDecayTimeSurface(2, 1, 10)
    linear.update(0, 0, True, 100)
    linear.update(1, 0, True, 105)
    assert linear.get(0, 0, True) == pytest.approx(0.5)
    assert linear.get(1, 0, True) == pytest.approx(1.0)
    assert linear.timestamp(0, 0, True) == 100

    exponential = openevt.ExponentialDecayTimeSurface(2, 1, 10)
    exponential.update(0, 0, True, 100)
    exponential.update(1, 0, True, 110)
    assert exponential.get(0, 0, True) == pytest.approx(np.exp(-1.0))
    assert exponential.get(1, 0, True) == pytest.approx(1.0)
    assert exponential.timestamp(1, 0, True) == 110


def test_time_surface_transformers_consume_structured_numpy_events(openevt):
    events = events_array([(0, 0, 1, 100), (1, 0, 0, 110)])

    base = openevt.BaseTimeSurfaceTransformer(2, 1).transform(events)
    assert base.get(0, 0, True) == 100
    assert base.get(1, 0, False) == 110

    linear = openevt.LinearDecayTimeSurfaceTransformer(2, 1, 20).transform(events)
    assert linear.get(0, 0, True) == pytest.approx(0.5)
    assert linear.get(1, 0, False) == pytest.approx(1.0)

    exponential = openevt.ExponentialDecayTimeSurfaceTransformer(2, 1, 10).transform(events)
    assert exponential.get(0, 0, True) == pytest.approx(np.exp(-1.0))
    assert exponential.get(1, 0, False) == pytest.approx(1.0)


def test_reader_lifecycle_and_metadata(openevt):
    reader = openevt.RawFileReader(str(SAMPLE_RAW))
    assert reader.ready()
    assert reader.t_min() is None
    assert reader.t_max() is None


def test_sync_iterator_is_pythonic_and_returns_structured_numpy(openevt):
    reader = openevt.RawFileReader(str(SAMPLE_RAW))
    iterator = reader.iter()

    assert iter(iterator) is iterator
    assert iterator.shape() == (720, 1280)

    batch = next(iterator)
    assert isinstance(batch, np.ndarray)
    assert batch.dtype.names == ("x", "y", "p", "t")
    assert batch.dtype.itemsize == 32
    assert batch.dtype.fields["x"][1] == 0
    assert batch.dtype.fields["y"][1] == 8
    assert batch.dtype.fields["p"][1] == 16
    assert batch.dtype.fields["t"][1] == 24
    assert batch.size > 0
    assert batch["x"].dtype == np.dtype("u8")
    assert batch["y"].dtype == np.dtype("u8")
    assert batch["p"].dtype == np.dtype("u1")
    assert batch["t"].dtype == np.dtype("u8")


def test_sync_iterator_exposes_batch_and_time_windows(openevt):
    reader = openevt.RawFileReader(str(SAMPLE_RAW))
    iterator = reader.iter()

    batch = iterator.next_batch()
    assert isinstance(batch, np.ndarray)
    assert batch.size > 0

    time_window = iterator.next_delta(10_000)
    assert isinstance(time_window, np.ndarray)
    assert time_window.dtype.names == ("x", "y", "p", "t")
    assert np.all(time_window["t"] < batch["t"][-1] + 10_000)


def test_sync_iterator_next_n_preserves_remaining_events(openevt):
    reader = openevt.RawFileReader(str(SAMPLE_RAW))
    iterator = reader.iter()

    first = iterator.next_n(10)
    second = iterator.next_n(10)

    assert first.size == 10
    assert second.size == 10
    assert first["t"][-1] <= second["t"][0]


def test_receiver_try_recv_returns_a_numpy_batch(openevt):
    reader = openevt.RawFileReader(str(SAMPLE_RAW))
    receiver = reader.cd_receiver()

    assert receiver.try_recv() is None
    reader.load_batch()

    batch = receiver.try_recv()
    assert isinstance(batch, np.ndarray)
    assert batch.dtype.names == ("x", "y", "p", "t")
    assert batch.size > 0


def test_async_iterator_protocol_and_batch(openevt):
    async def collect_one_batch():
        reader = openevt.RawFileReader(str(SAMPLE_RAW))
        iterator = reader.aiter()

        assert iterator.__aiter__() is iterator

        # IterAsync expects another producer to decode raw data.
        reader.load_batch()
        batch = await asyncio.wait_for(anext(iterator), timeout=10)
        return batch

    batch = asyncio.run(collect_one_batch())
    assert isinstance(batch, np.ndarray)
    assert batch.dtype.names == ("x", "y", "p", "t")
    assert batch.size > 0


def test_receiver_async_iteration(openevt):
    async def collect_one_batch():
        reader = openevt.RawFileReader(str(SAMPLE_RAW))
        receiver = reader.cd_receiver()
        reader.load_batch()
        batch = await asyncio.wait_for(anext(receiver), timeout=10)
        return batch

    batch = asyncio.run(collect_one_batch())
    assert isinstance(batch, np.ndarray)
    assert batch.dtype.names == ("x", "y", "p", "t")
    assert batch.size > 0
