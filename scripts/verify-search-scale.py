"""Production 10k search timings with independently cold SSD pages per query.
Leaves its printed cache DB and /tmp evidence directory for explicit cleanup.
Requires rebuilt search_scale example; never drops global caches.
"""
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('scale',Path(__file__).with_name('verify-scale-performance.py'))
scale=importlib.util.module_from_spec(spec);spec.loader.exec_module(scale)
subprocess.run(['df','-h','.'],check=True)
root=Path(tempfile.mkdtemp(prefix='rustrss-search-unigrams-'))
fd,name=tempfile.mkstemp(prefix='rustrss-search-unigrams-',suffix='.sqlite',dir=Path.home()/'.cache');os.close(fd)
path=Path(name);print('Evidence:',root,'Disk database:',path,flush=True)
subprocess.run(['target/debug/examples/search_scale','init',str(root/'seed.sqlite')],check=True,timeout=120)
src=sqlite3.connect(root/'seed.sqlite');dst=sqlite3.connect(path);src.backup(dst);src.close()
# Model the preceding production format: remove only standalone CJK tokens
# appended by v16; retain original bigrams and Latin/numeric tokens.
rows=dst.execute('SELECT id,search_tokens FROM entries').fetchall()
def cjk(c):
    n=ord(c)
    return 0x3040<=n<=0x30ff or 0x3400<=n<=0x4dbf or 0x4e00<=n<=0x9fff or 0xf900<=n<=0xfaff or 0x20000<=n<=0x2fa1f
dst.executemany('UPDATE entries SET search_tokens=? WHERE id=?',
    [(' '.join(w for w in text.split() if not(len(w)==1 and cjk(w))),id) for id,text in rows])
dst.execute('DROP INDEX IF EXISTS idx_entries_search_order')
dst.execute('PRAGMA user_version=15');dst.commit();dst.close()
def query(word):
    result=subprocess.run(['target/debug/examples/search_scale',word,str(path)],capture_output=True,text=True,check=True,timeout=120)
    return json.loads(result.stdout)
report={'migration':query('稀'),'results':[]}
with sqlite3.connect(path) as db:
    report['schema_version']=db.execute('PRAGMA user_version').fetchone()[0]
    db.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()
report['database_bytes']=path.stat().st_size
for word in ['uniqueneedle','common','中文','文','新闻','稀有词','稀','龘','absenttoken']:
    with path.open('rb') as file:os.fsync(file.fileno());os.posix_fadvise(file.fileno(),0,0,os.POSIX_FADV_DONTNEED)
    cache=scale.residency(path);assert cache['resident_pages']==0,cache
    result=query(word);result['cache_before']=cache;report['results'].append(result)
    print(json.dumps(result,ensure_ascii=False),flush=True)
(root/'results.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
