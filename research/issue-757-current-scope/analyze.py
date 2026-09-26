"""Validate each run before summarizing; no implicit pass for missing evidence."""
import csv
import hashlib
import json
import math
from pathlib import Path
import statistics
import sys

def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()

def quantiles(values):
    values = sorted(values)
    return {"mean": statistics.mean(values), **{
        name: values[math.ceil(q * len(values)) - 1]
        for name, q in [('p50', .5), ('p95', .95), ('p99', .99), ('max', 1)]}}

def captured_streams(meta_path, label):
    streams = [meta_path.parent / (label + suffix) for suffix in ['.stdout', '.stderr']]
    assert all(p.is_file() for p in streams), 'missing captured stream'
    return streams

def validate_frozen_run(meta_path, run, frozen):
    index = json.loads((Path(__file__).resolve().parent / 'evidence/files.json').read_text(encoding='utf-8'))
    canonical = json.dumps(index, sort_keys=True, separators=(',', ':')).encode('utf-8')
    assert hashlib.sha256(canonical).hexdigest() == frozen['raw_evidence_index']['canonical_sha256'], 'raw evidence index changed'
    expected = {entry['path']: entry for entry in index}
    paths = [meta_path, *captured_streams(meta_path, run.name), *[run / name for name in [
        'summary.json', 'timing.csv', 'work.csv', 'initial.jsonl', 'ticks.jsonl',
        'commands.jsonl', 'events.jsonl', 'extended-quality.json', 'individual-quality.csv']]]
    for path in paths:
        record = expected.get(str(path.resolve()))
        assert record is not None, 'run absent from frozen evidence'
        assert path.stat().st_size == record['bytes'] and digest(path) == record['sha256'], 'frozen evidence hash: ' + path.name

def analyze(meta_path):
    meta = json.loads(meta_path.read_text(encoding='utf-8-sig'))
    frozen = json.loads((Path(__file__).resolve().parent / 'evidence/identity.json').read_text(encoding='utf-8'))
    assert meta['scale'] in frozen['inputs'], 'unknown scale'
    assert meta['exit_code'] == 0, 'failed process'
    for field in ['source_unchanged', 'binary_unchanged', 'plan_unchanged', 'bundle_head_unchanged']:
        assert meta[field] is True, field
    assert meta['source_identity_before'] == meta['source_identity_after'], 'source changed'
    assert meta['source_identity_before'] == frozen['source_hash'], 'unfrozen source'
    assert meta['bundle_head'] == meta['bundle_head_after'], 'bundle HEAD changed'
    assert meta['bundle_status'] == '', 'uncommitted bundle'
    assert digest(Path(meta['binary'])) == meta['binary_sha256'].lower(), 'binary hash'
    assert digest(Path(meta['plan'])) == meta['plan_sha256'].lower(), 'plan hash'
    assert meta['plan_sha256'].lower() == frozen['plans'][meta['scale']]['sha256'], 'unfrozen plan'
    binary_name = Path(meta['binary']).name
    assert binary_name in frozen['binaries'] and meta['binary_sha256'].lower() == frozen['binaries'][binary_name]['sha256'], 'unfrozen binary'
    run = meta_path.parent / meta['label']
    summary = json.loads((run / 'summary.json').read_text())
    assert summary['status'] == 'research-prefix-complete', 'incomplete run'
    assert summary['error'] is None, 'run error'
    assert summary['mode'] == meta['mode'] and summary['workers'] == meta['workers'], 'mode/worker mismatch'
    assert summary['counting'] == meta['diagnostic'], 'counting mismatch'
    assert summary['plan_sha256'] == meta['plan_sha256'].lower(), 'summary plan hash'
    assert summary['input_manifest_sha256'] == frozen['inputs'][meta['scale']]['manifest.toml']['sha256'], 'unfrozen input manifest'
    expected_identity = {'run_id':meta['run_id'], 'mode':meta['mode'], 'scale':meta['scale'],
        'workers':meta['workers'], 'counting':meta['diagnostic'], 'limit':meta['ticks'],
        'plan_sha256':summary['plan_sha256'], 'input_manifest_sha256':summary['input_manifest_sha256'],
        'quality_schema':'active-to-active-v1'}
    assert summary['run_identity'] == expected_identity, 'summary run identity'
    rows = [{k: int(v) for k, v in r.items()} for r in csv.DictReader((run / 'timing.csv').open())]
    assert [r['tick'] for r in rows] == list(range(1, meta['ticks'] + 1)), 'missing/repeated ticks'
    assert summary['completed_ticks'] == meta['ticks'], 'completed tick count'
    quality = json.loads((run / 'extended-quality.json').read_text())
    assert quality['ticks'] == summary['completed_ticks'], 'quality tick count'
    assert quality['run_identity'] == expected_identity, 'quality run identity'
    assert digest(run / 'extended-quality.json') == summary['quality_sha256'], 'quality hash'
    assert digest(run / 'individual-quality.csv') == summary['trips_sha256'], 'trip hash'
    work = [{k: int(v) for k, v in r.items()} for r in csv.DictReader((run / 'work.csv').open())]
    assert [r['tick'] for r in work] == [r['tick'] for r in rows], 'work tick identity'
    if meta['diagnostic']:
        assert meta['workers'] == 1, 'TLS counters omit helper threads'
        assert sum(r['waiting_entries'] for r in work) > 0, 'missing diagnostic counts'
    else:
        assert all(v == 0 for r in work for k, v in r.items() if k != 'tick'), 'unexpected instrumentation'
    validate_frozen_run(meta_path, run, frozen)
    windows = {}
    cycle = 7656 if meta['scale'] == '10k' else 3712
    for name, low, high in [('entry', 1, 64), ('screen', 65, 512), ('running', 513, 2 * cycle), ('reflow', 2 * cycle + 1, meta['ticks'])]:
        selected = [r for r in rows if low <= r['tick'] <= high]
        if selected:
            windows[name] = {'first': selected[0]['tick'], 'last': selected[-1]['tick'], 'samples': len(selected),
                **{field.removesuffix('_ns') + '_ms': quantiles([r[field] / 1e6 for r in selected]) for field in
                    ['command_ns', 'step_ns', 'base_observation_ns', 'quality_ns', 'iteration_ns']},
                'active': quantiles([r['active'] for r in selected])}
    command_results = {}
    with (run / 'commands.jsonl').open(encoding='utf-8') as f:
        for line in f:
            row = json.loads(line)
            result = 'committed' if row['committed'] else row.get('details', {}).get('reason', 'uncommitted')
            key = str(row['command']) + ':' + str(result)
            command_results[key] = command_results.get(key, 0) + 1
    return {'label': meta['label'], 'mode': meta['mode'], 'scale': meta['scale'],
        'workers': meta['workers'], 'diagnostic':meta['diagnostic'], 'windows': windows,
        'quality': quality, 'summary': summary, 'command_results':command_results,
        'initial_sha256': digest(run / 'initial.jsonl'),
        'work_totals': {k: sum(r[k] for r in work) for k in work[0] if k != 'tick'},
        'peak_working_set_bytes':summary['peak_resident_bytes']}

if __name__ == '__main__':
    root = Path(sys.argv[1])
    runs = [analyze(path) for path in sorted(root.glob('*.process.json'))]
    assert runs, 'no runs'
    Path(sys.argv[2]).write_text(json.dumps(runs, indent=2) + '\n', encoding='utf-8')
    for run in runs:
        s = run['windows']['screen']
        print(run['label'], 'Core mean/p95 ms', round(s['step_ms']['mean'], 3), round(s['step_ms']['p95'], 3),
            'iteration ms', round(s['iteration_ms']['mean'], 3), 'replacements', run['summary']['replacements'])
