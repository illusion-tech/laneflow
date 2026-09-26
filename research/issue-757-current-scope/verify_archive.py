"""Regress copied diagnostics, exact inventory and relocated package reading."""
import io
import json
import shutil
import tempfile
from pathlib import Path, PureWindowsPath
from unittest.mock import patch
from analyze import require
from analyze import analyze
from inventory import validate_inventory

paths = [p for name in ['review-screen', 'review-counts', 'review-long']
         for p in sorted((Path('target') / name).glob('*.process.json'))]
validate_inventory(paths)

def rejects(action, reason):
    try:
        action()
    except AssertionError as error:
        require(str(error) == reason, (str(error), reason))
    else:
        raise AssertionError('invalid archive accepted')

rejects(lambda: validate_inventory(paths[:-1]), 'experiment matrix mismatch')
rejects(lambda: validate_inventory(paths[:-1] + paths[:1]), 'duplicate experiment')
read_text = Path.read_text
second = json.loads(paths[1].read_text())
second['run_id'] = json.loads(paths[0].read_text())['run_id']
with patch.object(Path, 'read_text', lambda p, *a, **kw:
                  json.dumps(second) if p == paths[1] else read_text(p, *a, **kw)):
    rejects(lambda: validate_inventory(paths), 'duplicate run identity')

meta = Path('target/review-counts/10k-both.process.json')
work = meta.parent / '10k-both/work.csv'
other = (meta.parent / '10k-all/work.csv').read_bytes()
original_open = Path.open
def swapped(p, *args, **kwargs):
    if p == work:
        mode = args[0] if args else kwargs.get('mode', 'r')
        return io.BytesIO(other) if 'b' in mode else io.StringIO(other.decode())
    return original_open(p, *args, **kwargs)
with patch.object(Path, 'open', swapped):
    rejects(lambda: analyze(meta), 'frozen evidence hash: work.csv')

meta = Path('target/review-screen/10k-1-all.process.json')
saved = json.loads(meta.read_text())
with tempfile.TemporaryDirectory(prefix='lf757-relocated-') as name:
    root = Path(name)
    group = root / meta.parent.name
    group.mkdir()
    shutil.copytree(meta.parent / saved['label'], group / saved['label'])
    for suffix in ['.process.json', '.stdout', '.stderr']:
        shutil.copy2(meta.parent / (saved['label'] + suffix), group)
    for kind, field in [('binaries', 'binary'), ('plans', 'plan')]:
        (root / kind).mkdir()
        asset_name = PureWindowsPath(saved[field]).name
        source_asset = meta.parent.parent / kind / asset_name
        if not source_asset.is_file():
            source_asset = Path(saved[field])
        shutil.copy2(source_asset, root / kind / asset_name)
    require(analyze(group / meta.name) == analyze(meta), 'test expectation failed')
# A changed excluded file must fail before either tracked artifact is replaced.
import runpy
import analyze as analyzer
before = {name: (Path(__file__).parent / 'evidence' / name).read_bytes()
          for name in ['files.json', 'results.json']}
real_digest = analyzer.digest
def damaged_excluded(path):
    return '0' * 64 if 'scope-counts' in path.parts else real_digest(path)
with patch.object(analyzer, 'digest', damaged_excluded):
    rejects(lambda: runpy.run_path(str(Path(__file__).parent / 'aggregate.py')),
            'raw evidence index changed')
for name, data in before.items():
    require((Path(__file__).parent / 'evidence' / name).read_bytes() == data,
            'rejected aggregation changed published files')
print('exact inventory and relocated package accepted; 5 archive corruption variants rejected')
