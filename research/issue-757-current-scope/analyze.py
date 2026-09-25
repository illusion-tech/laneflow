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

def analyze(meta_path):
    meta = json.loads(meta_path.read_text(encoding='utf-8-sig'))
    assert meta['exit_code'] == 0, 'failed process'
    for field in ['source_unchanged', 'binary_unchanged', 'plan_unchanged', 'bundle_head_unchanged']:
        assert meta[field] is True, field
    assert meta['source_identity_before'] == meta['source_identity_after'], 'source changed'
    assert meta['bundle_head'] == meta['bundle_head_after'], 'bundle HEAD changed'
    assert meta['bundle_status'] == '', 'uncommitted bundle'
    assert digest(Path(meta['binary'])) == meta['binary_sha256'].lower(), 'binary hash'
    assert digest(Path(meta['plan'])) == meta['plan_sha256'].lower(), 'plan hash'
    run = meta_path.parent / meta['label']
    summary = json.loads((run / 'summary.json').read_text())
    assert summary['status'] == 'research-prefix-complete', 'incomplete run'
    assert summary['error'] is None, 'run error'
    assert summary['mode'] == meta['mode'] and summary['workers'] == meta['workers'], 'mode/worker mismatch'
    assert summary['counting'] == meta['diagnostic'], 'counting mismatch'
    assert summary['plan_sha256'] == meta['plan_sha256'].lower(), 'summary plan hash'
    rows = [{k: int(v) for k, v in r.items()} for r in csv.DictReader((run / 'timing.csv').open())]
    assert [r['tick'] for r in rows] == list(range(1, meta['ticks'] + 1)), 'missing/repeated ticks'
    assert summary['completed_ticks'] == meta['ticks'], 'completed tick count'
    quality = json.loads((run / 'extended-quality.json').read_text())
    work = [{k: int(v) for k, v in r.items()} for r in csv.DictReader((run / 'work.csv').open())]
    assert [r['tick'] for r in work] == [r['tick'] for r in rows], 'work tick identity'
    if meta['diagnostic']:
        assert meta['workers'] == 1, 'TLS counters omit helper threads'
        assert sum(r['waiting_entries'] for r in work) > 0, 'missing diagnostic counts'
    else:
        assert all(v == 0 for r in work for k, v in r.items() if k != 'tick'), 'unexpected instrumentation'
    windows = {}
    cycle = 7656 if meta['scale'] == '10k' else 3712
    for name, low, high in [('entry', 1, 64), ('screen', 65, 512), ('running', 513, 2 * cycle), ('reflow', 2 * cycle + 1, meta['ticks'])]:
        selected = [r for r in rows if low <= r['tick'] <= high]
        if selected:
            windows[name] = {'first': selected[0]['tick'], 'last': selected[-1]['tick'], 'samples': len(selected),
                **{field: quantiles([r[field] / 1e6 for r in selected]) for field in
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
        print(run['label'], 'Core mean/p95', round(s['step_ns']['mean'], 3), round(s['step_ns']['p95'], 3),
            'iteration', round(s['iteration_ns']['mean'], 3), 'replacements', run['summary']['replacements'])
