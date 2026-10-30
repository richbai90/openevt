"""Event-camera filters for the openevt stack.

Each filter consumes a structured NumPy event array with fields
``(x, y, p, t)`` and returns a filtered array of the same dtype.
"""

from typing import Any, final

import numpy.typing as npt

__all__ = [
    "PolarityFilter",
    "RoiFilter",
    "ActivityNoiseFilter",
    "SpatioTemporalContrastFilter",
    "TrailFilter",
    "AntiFlickerFilter",
]

@final
class ActivityNoiseFilter:
    """Removes events from pixels whose neighbours are inactive."""

    def __new__(cls, width: int, height: int, dt: int) -> ActivityNoiseFilter: ...
    def process(self, events: npt.NDArray[Any]) -> npt.NDArray[Any]: ...

@final
class AntiFlickerFilter:
    """Removes events oscillating at an ambient flicker frequency."""

    def __new__(
        cls,
        width: int,
        height: int,
        min_measurements: int,
        min_freq: int,
        max_freq: int,
    ) -> AntiFlickerFilter: ...
    def process(self, events: npt.NDArray[Any]) -> npt.NDArray[Any]: ...

@final
class PolarityFilter:
    """Keeps only events matching one polarity."""

    def __new__(cls, polarity: bool) -> PolarityFilter: ...
    def process(self, events: npt.NDArray[Any]) -> npt.NDArray[Any]: ...

@final
class RoiFilter:
    """Keeps only events inside the rectangle ``[x0, x1) x [y0, y1)``."""

    def __new__(cls, x0: int, x1: int, y0: int, y1: int) -> RoiFilter: ...
    def process(self, events: npt.NDArray[Any]) -> npt.NDArray[Any]: ...

@final
class SpatioTemporalContrastFilter:
    """Keeps events whose local spatio-temporal contrast passes a threshold."""

    def __new__(
        cls, width: int, height: int, dt: int, cut_trail: bool
    ) -> SpatioTemporalContrastFilter: ...
    def process(self, events: npt.NDArray[Any]) -> npt.NDArray[Any]: ...

@final
class TrailFilter:
    """Removes events that trail a moving edge."""

    def __new__(cls, width: int, height: int, dt: int) -> TrailFilter: ...
    def process(self, events: npt.NDArray[Any]) -> npt.NDArray[Any]: ...
