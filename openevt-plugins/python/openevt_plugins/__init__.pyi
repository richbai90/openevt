"""Type stubs for the openevt_plugins Python bindings."""

from typing import List, Optional, Tuple, final


__all__ = ["Simulator"]

@final
class Simulator:
    """Simulates an event camera from a video file.

    Events are returned as ``(x, y, polarity, timestamp)`` tuples with
    microsecond timestamps.
    """

    def __new__(
        cls,
        video_path: str,
        config_toml: Optional[str] = None,
        width: int = 160,
        height: int = 90,
    ) -> Simulator: ...
    def next_batch(self) -> List[Tuple[int, int, bool, int]]:
        """Simulate and return one decoded frame as
        ``(x, y, polarity, timestamp)`` tuples."""
        ...
    def seek(self, timestamp: int) -> None: ...
    @property
    def t_max(self) -> int: ...
