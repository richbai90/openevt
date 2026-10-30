"""Type stubs for the openevt event-camera extension module."""

from typing import Any, AsyncIterator, Awaitable, Iterator, Optional, Tuple, final

import numpy.typing as npt

from . import algorithms as algorithms
from . import filters as filters
from .algorithms import (
    ActivityNoiseFilter as ActivityNoiseFilter,
    AntiFlickerFilter as AntiFlickerFilter,
    BaseTimeSurface as BaseTimeSurface,
    BaseTimeSurfaceTransformer as BaseTimeSurfaceTransformer,
    ExponentialDecayTimeSurface as ExponentialDecayTimeSurface,
    ExponentialDecayTimeSurfaceTransformer as ExponentialDecayTimeSurfaceTransformer,
    Histogram as Histogram,
    HistogramTransformer as HistogramTransformer,
    IntegralImage as IntegralImage,
    IntegralImageTransformer as IntegralImageTransformer,
    LinearDecayTimeSurface as LinearDecayTimeSurface,
    LinearDecayTimeSurfaceTransformer as LinearDecayTimeSurfaceTransformer,
    PolarityFilter as PolarityFilter,
    RoiFilter as RoiFilter,
    SpatioTemporalContrastFilter as SpatioTemporalContrastFilter,
    Tensor as Tensor,
    TrailFilter as TrailFilter,
)

__all__ = [
    "PyEventCD",
    "EventCD",
    "CDEventReceiver",
    "CDEventIterator",
    "AsyncCDEventIterator",
    "RawFileReader",
    "PolarityFilter",
    "RoiFilter",
    "ActivityNoiseFilter",
    "SpatioTemporalContrastFilter",
    "TrailFilter",
    "AntiFlickerFilter",
    "BaseTimeSurface",
    "LinearDecayTimeSurface",
    "ExponentialDecayTimeSurface",
    "Tensor",
    "Histogram",
    "IntegralImage",
    "BaseTimeSurfaceTransformer",
    "LinearDecayTimeSurfaceTransformer",
    "ExponentialDecayTimeSurfaceTransformer",
    "HistogramTransformer",
    "IntegralImageTransformer",
    "algorithms",
    "filters",
]

@final
class EventCD:
    """A change-detection event with coordinates, polarity, and timestamp.

    Coordinates are ``x``/``y`` pixel indices, ``p`` is ``0`` or ``1``,
    and ``t`` is a microsecond timestamp.
    """

    x: int
    y: int
    p: int
    t: int
    def __new__(cls, x: int, y: int, p: bool, t: int) -> EventCD: ...

PyEventCD = EventCD

@final
class CDEventReceiver:
    """Receives decoded change-detection event batches from a device."""

    def try_recv(self) -> Optional[npt.NDArray[Any]]:
        """Receive one already-decoded batch without waiting."""
        ...
    def __iter__(self) -> Iterator[npt.NDArray[Any]]: ...
    def __next__(self) -> npt.NDArray[Any]: ...
    def __aiter__(self) -> AsyncIterator[npt.NDArray[Any]]: ...
    def __anext__(self) -> Awaitable[npt.NDArray[Any]]: ...

@final
class CDEventIterator:
    """Synchronous iterator over decoded event batches.

    Each batch is a structured NumPy array with fields ``(x, y, p, t)``
    and item size 32.
    """

    def __iter__(self) -> CDEventIterator: ...
    def __next__(self) -> npt.NDArray[Any]: ...
    def next_batch(self) -> Optional[npt.NDArray[Any]]: ...
    def next_n(self, n: int) -> Optional[npt.NDArray[Any]]:
        """Return up to ``n`` events while preserving any remaining events."""
        ...
    def next_delta(self, dt: int) -> Optional[npt.NDArray[Any]]:
        """Return the next time window of events within ``dt`` of the
        current time baseline, or ``None`` at end of stream."""
        ...
    def shape(self) -> Tuple[int, int]: ...

@final
class AsyncCDEventIterator:
    """Asynchronous iterator over decoded event batches."""

    def __aiter__(self) -> AsyncCDEventIterator: ...
    def __anext__(self) -> Awaitable[npt.NDArray[Any]]: ...
    def next_batch(self) -> Awaitable[npt.NDArray[Any]]: ...
    def shape(self) -> Tuple[int, int]: ...

@final
class RawFileReader:
    """Reads raw event files through the openevt device-plugin ABI.

    Requires a compatible device plugin on ``OPENEVT_PLUGIN_PATH``.
    """

    def __new__(
        cls, path: Optional[str] = None, index: bool = False
    ) -> RawFileReader: ...
    def try_open(self, file_path: str, index: bool) -> None: ...
    def ready(self) -> bool: ...
    def t_min(self) -> Optional[int]: ...
    def t_max(self) -> Optional[int]: ...
    def seek(self, ts: int) -> None: ...
    def seek_to_next_ext(self) -> None: ...
    def load_batch(self) -> None: ...
    def cd_receiver(self) -> CDEventReceiver: ...
    def iter(self) -> CDEventIterator:
        """Create a synchronous batch iterator."""
        ...
    def aiter(self) -> AsyncCDEventIterator:
        """Create an asynchronous batch iterator."""
        ...
