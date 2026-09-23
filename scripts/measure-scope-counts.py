"""Linux-only, owned fixture: evict its page cache and verify residency before timing.

Build: cargo build -p rustrss-core --example scope_counts
Seed: target/debug/examples/scope_counts init target/verification-followups/fixture.sqlite
Run: python3 scripts/measure-scope-counts.py target/verification-followups/fixture.sqlite
This measures OS page-cache cold reads, not a power-cycled storage controller.
"""
import ctypes
import json
import mmap
import os
from pathlib import Path
import subprocess
import sys

path = Path(sys.argv[1]).resolve()
owned = Path("target/verification-followups").resolve()
if not path.is_relative_to(owned):
    raise SystemExit("Only isolated fixtures under target/verification-followups are allowed")
if any(Path(str(path) + suffix).exists() for suffix in ("-wal", "-shm")):
    raise SystemExit("Close all fixture connections before measuring")
libc = ctypes.CDLL(None, use_errno=True)
libc.mincore.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p]
libc.mincore.restype = ctypes.c_int

def resident_pages(fd):
    size = os.fstat(fd).st_size
    with mmap.mmap(fd, size, access=mmap.ACCESS_COPY) as mapping:
        address = ctypes.addressof(ctypes.c_char.from_buffer(mapping))
        pages = (size + mmap.PAGESIZE - 1) // mmap.PAGESIZE
        vector = (ctypes.c_ubyte * pages)()
        if libc.mincore(address, size, vector):
            raise OSError(ctypes.get_errno(), "mincore")
        return sum(v & 1 for v in vector), pages

results = []
for scope in ("feed", "folder", "tag"):
    for trial in range(3):
        with path.open("rb") as stream:
            os.fsync(stream.fileno())
            os.posix_fadvise(stream.fileno(), 0, 0, os.POSIX_FADV_DONTNEED)
            resident, pages = resident_pages(stream.fileno())
        if resident:
            raise SystemExit(f"Not cold: {resident}/{pages} pages remain resident")
        output = subprocess.check_output(
            ["target/debug/examples/scope_counts", scope, str(path)], text=True, timeout=30)
        row = json.loads(output)
        row.update(trial=trial + 1, resident_before=resident, pages=pages)
        results.append(row)
print(json.dumps({"bytes": path.stat().st_size, "cache": "OS page cache cold; controller cache unspecified",
                  "measurements": results}, indent=2))
