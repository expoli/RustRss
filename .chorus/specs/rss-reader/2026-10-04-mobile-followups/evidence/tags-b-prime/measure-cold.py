"""Linux owned fixture only: OS page-cache cold (verified with mincore), then warm queries.
Build query-probe.rs per record.md. No live database/global drop_caches access.
"""
import ctypes
import json
import mmap
import os
from pathlib import Path
import sqlite3
import subprocess

path = Path('target/verification-followups/effective.sqlite').resolve()
assert path.is_relative_to(Path('target/verification-followups').resolve())
assert not any(Path(str(path) + s).exists() for s in ('-wal', '-shm'))
with sqlite3.connect(path) as c:
    cursors = {}
    for sort, order in [('newest', 'DESC'), ('oldest', 'ASC'), ('unread_first', 'DESC')]:
        # unread_first breaks equal timestamps by id ASC, unlike newest.
        id_order = 'ASC' if sort == 'unread_first' else order
        cursors[sort] = c.execute(f'''SELECT COALESCE(published_at,fetched_at), id FROM entries
            WHERE feed_id <= 60 ORDER BY COALESCE(published_at,fetched_at) {order}, id {id_order}
            LIMIT 1 OFFSET 199''').fetchone()
libc = ctypes.CDLL(None, use_errno=True)
libc.mincore.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p]
libc.mincore.restype = ctypes.c_int
modes = ['count', 'sidebar'] + [f'{sort}-{page}' for sort in cursors for page in ['first', 'next']]
results = []
for mode in modes:
    for trial in range(3):
        with path.open('rb') as stream:
            os.fsync(stream.fileno())
            os.posix_fadvise(stream.fileno(), 0, 0, os.POSIX_FADV_DONTNEED)
            size = path.stat().st_size
            with mmap.mmap(stream.fileno(), size, access=mmap.ACCESS_COPY) as mapping:
                address = ctypes.addressof(ctypes.c_char.from_buffer(mapping))
                pages = (size + mmap.PAGESIZE - 1) // mmap.PAGESIZE
                vector = (ctypes.c_ubyte * pages)()
                assert libc.mincore(address, size, vector) == 0
                resident = sum(v & 1 for v in vector)
            assert resident == 0, (resident, pages)
        cursor = cursors.get(mode.rsplit('-', 1)[0], (0, 0))
        row = json.loads(subprocess.check_output(['/tmp/rustrss-effective-tag-probe', str(path), mode, *map(str, cursor)], text=True, timeout=30))
        row.update(trial=trial + 1, resident_before=resident, pages=pages)
        results.append(row)
print(json.dumps({'entries': 12000, 'effective': 6000, 'manual': 3000, 'inherited': 6000,
    'bytes': size, 'cache': 'OS page cache cold; controller cache unspecified; debug binary',
    'measurements': results}, indent=2))
