"""The greeter driven from Python over the C ABI, with ctypes and no library.

Everything a foreign host needs to know is in ABI.md at the repository root;
this file is that document as ~100 lines of Python. The greeter's own tag
table (which byte means which effect) is the one thing that is the program's
rather than the ABI's, and it is `TAGS` below.

    cargo build -p greeter_cdylib
    python3 demo/python/main.py            # alice, bob, quit
    python3 demo/python/main.py --fanout   # two waits per batch
"""

import ctypes
import struct
import sys
import time
from collections import deque
from pathlib import Path

# ---- ABI.md, as code ------------------------------------------------------

OK = AWAITING = 0
COMPLETE, STALLED = 1, 2
ERRORS = {-1: "BUSY", -2: "FINISHED", -4: "PANICKED", -5: "BAD_HANDLE", -6: "BAD_INPUT"}

# Input records: tag, then request id, then payload by kind.
START = b"\x00"


def reply_str(id_: int, s: str) -> bytes:
    b = s.encode()
    return struct.pack("<BQI", 1, id_, len(b)) + b


def reply_u64(id_: int, n: int) -> bytes:
    return struct.pack("<BQQ", 2, id_, n)


def reply_unit(id_: int) -> bytes:
    return struct.pack("<BQ", 3, id_)


def reply_bytes(id_: int, b: bytes) -> bytes:
    return struct.pack("<BQI", 4, id_, len(b)) + b


class Reader:
    """Little-endian records; `str` is u32 length + UTF-8."""

    def __init__(self, data: bytes):
        self.data, self.at = data, 0

    def _take(self, fmt: str):
        (v,) = struct.unpack_from(fmt, self.data, self.at)
        self.at += struct.calcsize(fmt)
        return v

    def u8(self) -> int:
        return self._take("<B")

    def u64(self) -> int:
        return self._take("<Q")

    def str(self) -> str:
        n = self._take("<I")
        s = self.data[self.at : self.at + n].decode()
        self.at += n
        return s

    def done(self) -> bool:
        return self.at >= len(self.data)


class Library:
    """`new / step / free / buf_free` over a loaded cdylib, prefix `greeter_`."""

    def __init__(self, path: Path):
        self.lib = ctypes.CDLL(str(path))
        self.lib.greeter_new.restype = ctypes.c_uint64
        self.lib.greeter_new_fanout.restype = ctypes.c_uint64
        self.lib.greeter_step.restype = ctypes.c_int32
        self.lib.greeter_step.argtypes = [
            ctypes.c_uint64,
            ctypes.c_char_p,
            ctypes.c_size_t,
            ctypes.POINTER(ctypes.POINTER(ctypes.c_uint8)),
            ctypes.POINTER(ctypes.c_size_t),
        ]
        self.lib.greeter_free.restype = ctypes.c_int32
        self.lib.greeter_buf_free.argtypes = [ctypes.POINTER(ctypes.c_uint8), ctypes.c_size_t]

    def new(self, fanout: bool) -> int:
        return self.lib.greeter_new_fanout() if fanout else self.lib.greeter_new()

    def step(self, handle: int, record: bytes) -> tuple[int, bytes]:
        """One input record in; (status, effect bytes) out. Raises on an error code."""
        out_ptr = ctypes.POINTER(ctypes.c_uint8)()
        out_len = ctypes.c_size_t()
        code = self.lib.greeter_step(handle, record, len(record), ctypes.byref(out_ptr), ctypes.byref(out_len))
        if code < 0:
            raise RuntimeError(ERRORS.get(code, code))
        data = ctypes.string_at(out_ptr, out_len.value)
        self.lib.greeter_buf_free(out_ptr, out_len.value)
        return code, data

    def free(self, handle: int) -> None:
        code = self.lib.greeter_free(handle)
        if code < 0:
            raise RuntimeError(ERRORS.get(code, code))


# ---- the greeter's tag table (the program's, not the ABI's) ---------------

TAGS = {1: "count", 2: "lookup", 3: "read_line", 4: "sleep", 5: "write"}


def decode(data: bytes) -> list[dict]:
    """Effect records → dicts with a `kind`, its fields, and an `id` if awaiting."""
    r, effects = Reader(data), []
    while not r.done():
        kind = TAGS[r.u8()]
        if kind == "count":
            effects.append({"kind": kind, "id": r.u64()})
        elif kind == "lookup":
            effects.append({"kind": kind, "name": r.str(), "id": r.u64()})
        elif kind == "read_line":
            effects.append({"kind": kind, "id": r.u64()})
        elif kind == "sleep":
            effects.append({"kind": kind, "millis": r.u64(), "id": r.u64()})
        elif kind == "write":
            effects.append({"kind": kind, "text": r.str()})
    return effects


# ---- the host loop ----------------------------------------------------------

GREETINGS = {"alice": "Hello", "bob": "Hi", "carol": "Hey"}


def drive(lib: Library, handle: int, script: list[str]) -> list[str]:
    """Perform each effect and reply by id until the routine completes."""
    lines, written, greeted = iter(script), [], 0
    status, data = lib.step(handle, START)
    queue = deque(decode(data))

    while queue:
        e = queue.popleft()
        if e["kind"] == "write":
            written.append(e["text"])
            print(e["text"])
            continue
        if e["kind"] == "read_line":
            record = reply_str(e["id"], next(lines, "quit"))
        elif e["kind"] == "lookup":
            record = reply_str(e["id"], GREETINGS.get(e["name"], "Greetings"))
        elif e["kind"] == "sleep":
            time.sleep(e["millis"] / 1000)
            record = reply_unit(e["id"])
        elif e["kind"] == "count":
            greeted += 1
            record = reply_u64(e["id"], greeted)
        status, data = lib.step(handle, record)
        queue.extend(decode(data))

    assert status == COMPLETE, f"routine ended with status {status}"
    return written


def find_library() -> Path:
    root = Path(__file__).resolve().parents[2]
    for profile in ("debug", "release"):
        for name in ("libgreeter_cdylib.so", "libgreeter_cdylib.dylib", "greeter_cdylib.dll"):
            p = root / "target" / profile / name
            if p.exists():
                return p
    sys.exit("build it first: cargo build -p greeter_cdylib")


if __name__ == "__main__":
    fanout = "--fanout" in sys.argv
    lib = Library(find_library())
    handle = lib.new(fanout)
    try:
        drive(lib, handle, ["alice", "bob"] if not fanout else ["bob", "carol"])
    finally:
        lib.free(handle)
