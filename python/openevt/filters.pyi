"""Event-camera filters (alias of ``openevt.algorithms.filters``)."""

from .algorithms.filters import (
    ActivityNoiseFilter as ActivityNoiseFilter,
    AntiFlickerFilter as AntiFlickerFilter,
    PolarityFilter as PolarityFilter,
    RoiFilter as RoiFilter,
    SpatioTemporalContrastFilter as SpatioTemporalContrastFilter,
    TrailFilter as TrailFilter,
)

__all__ = [
    "PolarityFilter",
    "RoiFilter",
    "ActivityNoiseFilter",
    "SpatioTemporalContrastFilter",
    "TrailFilter",
    "AntiFlickerFilter",
]
