"""Rebind synthetic transport hashes after an adversarial fixture edit.

The approved inventory/rubric are never rewritten. Tests use a new out-of-band
manifest pin to distinguish semantic integrity checks from ordinary byte tampering.
"""
import hashlib
import json
import sys
from pathlib import Path
root=Path(sys.argv[1])
def read(path): return json.loads((root/path).read_text())
def write(path,value): (root/path).write_text(json.dumps(value,indent=2,ensure_ascii=False)+'\n')
def sha(path): return hashlib.sha256((root/path).read_bytes()).hexdigest()
index=read('evaluator-only/unit-index.json')
index['approved_scope_sha256']=sha(index['approved_scope_file'])
for unit in index['units']:
    unit['evaluator_oracle_sha256']=sha(unit['evaluator_oracle'])
    for group in ['reader_allowlist','grader_static_allowlist']:
        for entry in unit[group]:
            path=entry['package_path']
            if (root/path).is_file(): entry['sha256']=sha(path)
write('evaluator-only/unit-index.json',index)
manifest=read('reader-inputs/reader-view-manifest.json')
manifest['units']=[{key:value for key,value in unit.items() if key in ['unit_id','question_id','case','cutoff','reader_allowlist']} for unit in index['units']]
write('reader-inputs/reader-view-manifest.json',manifest)
version=read('version.json')
for key,path in [('unit_index_sha256','evaluator-only/unit-index.json'),('reader_view_manifest_sha256','reader-inputs/reader-view-manifest.json')]: version[key]=sha(path)
write('version.json',version)
files=sorted(p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file() and p.name!='package.sha256')
(root/'package.sha256').write_text(''.join(f'{sha(path)}  {path}\n' for path in files))
