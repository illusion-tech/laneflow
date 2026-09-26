import copy
import json
from pathlib import Path
import sys
from unittest.mock import patch

sys.path.insert(0, 'research/issue-757-current-scope')
import analyze

meta = Path('target/scope-screen/10k-1-all.process.json')
original = Path.read_text
source = json.loads(original(meta))

def rejected(change):
    edited = copy.deepcopy(source)
    change(edited)
    def read(path, *args, **kwargs):
        return json.dumps(edited) if path == meta else original(path, *args, **kwargs)
    try:
        with patch.object(Path, 'read_text', read):
            analyze.analyze(meta)
    except AssertionError:
        return
    raise RuntimeError('invalid run accepted')

analyze.analyze(meta)
for change in [
    lambda m: m.update(exit_code=1),
    lambda m: m.update(source_unchanged=False),
    lambda m: m.update(bundle_head_after='wrong'),
    lambda m: m.update(bundle_status=' M source.rs'),
    lambda m: m.update(binary_sha256='0'*64),
    lambda m: m.update(workers=2),
    lambda m: m.update(diagnostic=True),
    lambda m: m.update(ticks=511),
]:
    rejected(change)
print('valid run accepted; 8 invalid evidence variants rejected')
