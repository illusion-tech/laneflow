"""Bind the completed experiments and explicitly preserve the interrupted attempt."""
import json
from pathlib import Path
from analyze import analyze, captured_streams, digest, quantiles
import csv

here = Path(__file__).resolve().parent
target = Path('target')
roots = ['review-screen', 'review-counts', 'review-long', 'scope-counts']
complete, excluded, index = [], [], []
for name in roots:
    for meta_path in sorted((target/name).glob('*.process.json')):
        meta = json.loads(meta_path.read_text())
        run = meta_path.parent/meta['label']
        if name == 'scope-counts' and meta['label'] != '100k-p3':
            continue
        if name == 'scope-counts':
            assert 'exit_code' not in meta and not (run/'summary.json').exists(), 'exclusion changed'
            excluded.append({'label':str(run),'status':'interrupted-incomplete','reason':'no exit status or summary; separately rerun in scope-counts-remaining'})
            files = [meta_path, *run.glob('*')]
        else:
            result = analyze(meta_path)
            with (run/'individual-quality.csv').open() as f:
                trips = list(csv.DictReader(f))
            result['trips'] = {
                'segments':len(trips), 'right_censored':sum(r['right_censored']=='true' for r in trips),
                'complete_new_trips':sum(r['left_censored']=='false' and r['right_censored']=='false' and r['end_kind']=='completed' for r in trips),
                'stopped_at_least_60s':sum(int(r['max_continuous_stop_ms'])>=60000 for r in trips),
                'max_stop_ms':quantiles([int(r['max_continuous_stop_ms']) for r in trips]),
            }
            result['source_identity'] = meta['source_identity_before']
            result['binary_sha256'] = meta['binary_sha256'].lower()
            result['bundle_head'] = meta['bundle_head']
            result['directory'] = str(run.resolve())
            result['summary'].pop('evidence')
            complete.append(result)
            streams = captured_streams(meta_path, meta['label'])
            files = [meta_path, *run.glob('*'), *streams]
        for path in sorted(files):
            if path.is_file():
                index.append({'path':str(path.resolve()),'bytes':path.stat().st_size,'sha256':digest(path)})
assert len(complete)==28 and len(excluded)==1, 'experiment inventory changed'
output = here/'evidence'
(output/'results.json').write_text(json.dumps({'complete':complete,'excluded':excluded},indent=2)+'\n',encoding='utf-8')
(output/'files.json').write_text(json.dumps(index,indent=2)+'\n',encoding='utf-8')
print('28 complete runs; 1 interrupted attempt excluded and indexed')
for r in complete:
    if r['diagnostic']:
        print(r['label'],r['work_totals'])
for scale in ['10k','100k']:
    for mode in ['all','p3','waiting','both']:
        rows = [r for r in complete if 'review-screen' in r['directory'] and r['scale']==scale and r['mode']==mode]
        values = [r['windows']['screen']['step_ms']['mean'] for r in rows]
        p95 = [r['windows']['screen']['step_ms']['p95'] for r in rows]
        iteration = [r['windows']['screen']['iteration_ms']['mean'] for r in rows]
        print(scale,mode,'mean',sum(values)/len(values),'p95 range',min(p95),max(p95),'iteration',sum(iteration)/len(iteration))
for r in complete:
    if 'review-long' in r['directory']:
        print(r['label'],r['trips'])
