#!/usr/bin/env python3
"""Generate the deterministic synthetic EVT3 fixture used by the test suite.

Writes sample.raw next to this script. The file must satisfy the margins the
tests assert against:

- header declares EVT3 with 1280x720 so readers report shape (720, 1280)
- at least 4096 CD events so a 4096-event batch fills completely
- more than 131_072 events so the Python iterator's 131_072-event batch
  leaves a remainder for the time-window tests
- every shifted timestamp stays below 10_000 for the next_delta window tests
- timestamps are non-decreasing so window slicing and next_n ordering hold

EVT3 words are little-endian u16. Data starts immediately after the last
header line: a stray newline byte between header and data would shift the
word stream and misalign every decode.
"""

import struct
from pathlib import Path

WIDTH = 1280
HEIGHT = 720
EVENT_COUNT = 150_000
MAX_TIMESTAMP = 10_000
TIME_UPDATE_EVERY = 512

HEADER = b"%Data format: EVT3; 1280; 720\n"

TIME_HIGH = 0x8000
TIME_LOW = 0x6000
ADDR_Y = 0x0000
ADDR_X = 0x2000


def build_words():
    words = [TIME_HIGH]
    last_high = 0
    for i in range(EVENT_COUNT):
        t = (i * MAX_TIMESTAMP) // EVENT_COUNT
        if i % TIME_UPDATE_EVERY == 0:
            high = t >> 12
            if high > last_high:
                words.append(TIME_HIGH | high)
                last_high = high
            words.append(TIME_LOW | (t & 0x0FFF))
        y = i % HEIGHT
        x = (i * 7) % WIDTH
        polarity = i % 2
        words.append(ADDR_Y | y)
        words.append(ADDR_X | (polarity << 11) | x)
    return words


def main() -> None:
    out = Path(__file__).with_name("sample.raw")
    data = b"".join(struct.pack("<H", word) for word in build_words())
    out.write_bytes(HEADER + data)
    print(f"wrote {out} ({out.stat().st_size} bytes, {EVENT_COUNT} events)")


if __name__ == "__main__":
    main()
