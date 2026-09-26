"""Exercise evidence rejection against real files without changing the saved run."""
import copy
import json
from pathlib import Path
import sys
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analyze

meta = Path('target/review-screen/10k-1-all.process.json')
original = Path.read_text
source = json.loads(original(meta))
run = meta.parent / source['label']
summary_path = run / 'summary.json'
quality_path = run / 'extended-quality.json'

def rejected(path, change, reason):
    edited = copy.deepcopy(json.loads(original(path)))
    change(edited)
    def read(candidate, *args, **kwargs):
        return json.dumps(edited) if candidate == path else original(candidate, *args, **kwargs)
    try:
        with patch.object(Path, 'read_text', read):
            analyze.analyze(meta)
    except AssertionError as error:
        assert str(error) == reason, (str(error), reason)
        return
    raise RuntimeError('invalid run accepted')

result = analyze.analyze(meta)
assert result['quality']['run_identity']['quality_schema'] == 'active-to-active-v1'
for change, reason in [
    (lambda m: m.update(exit_code=1), 'failed process'),
    (lambda m: m.update(source_unchanged=False), 'source_unchanged'),
    (lambda m: m.update(bundle_head_after='wrong'), 'bundle HEAD changed'),
    (lambda m: m.update(bundle_status=' M source.rs'), 'uncommitted bundle'),
    (lambda m: m.update(binary_sha256='0'*64), 'binary hash'),
    (lambda m: m.update(workers=2), 'mode/worker mismatch'),
    (lambda m: m.update(diagnostic=True), 'counting mismatch'),
    (lambda m: m.update(ticks=511), 'summary run identity'),
]:
    rejected(meta, change, reason)

rejected(quality_path, lambda q: q.update(ticks=16_384), 'quality tick count')
rejected(quality_path, lambda q: q['run_identity'].update(run_id='other-run'), 'quality run identity')
rejected(quality_path, lambda q: q['run_identity'].update(mode='both'), 'quality run identity')
other_quality = json.loads(original(meta.parent / '10k-2-all' / 'extended-quality.json'))
rejected(quality_path, lambda q: (q.clear(), q.update(other_quality)), 'quality run identity')
rejected(summary_path, lambda s: s.update(input_manifest_sha256='0'*64), 'unfrozen input manifest')
rejected(summary_path, lambda s: s.update(quality_sha256='0'*64), 'quality hash')
rejected(summary_path, lambda s: s.update(trips_sha256='0'*64), 'trip hash')

streams = analyze.captured_streams(meta, source['label'])
is_file = Path.is_file
for missing in streams:
    try:
        with patch.object(Path, 'is_file', lambda p: False if p == missing else is_file(p)):
            analyze.captured_streams(meta, source['label'])
    except AssertionError as error:
        assert str(error) == 'missing captured stream'
    else:
        raise RuntimeError('missing stream accepted')
print('valid run accepted; 17 invalid evidence variants rejected')
