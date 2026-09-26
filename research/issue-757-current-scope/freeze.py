"""Record build/source/input identity before measured runs."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

root = Path.cwd()
here = Path(__file__).resolve().parent
baseline = '46fdfaf47ae0c000ddc63420c8a71443baa8fb04'
frozen = Path('E:/projects/laneflow-evidence/issue-707/4de40e04')

def run(*args):
    return subprocess.check_output(args, text=True, encoding='utf-8').strip()

def identity(path):
    return {'path': str(path.resolve()), 'bytes': path.stat().st_size,
            'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}

evidence = here / 'evidence'
evidence.mkdir(exist_ok=True)
result = {'baseline_commit':baseline, 'baseline_tree':run('git','rev-parse',baseline+'^{tree}'),
          'rustc':run('rustc','+1.98.0','-Vv'), 'cargo':run('cargo','+1.98.0','-V'),
          'build':'release; locked; offline; CARGO_INCREMENTAL=0; default features; diagnostic adds laneflow-runtime/scope-counts',
          'source_hash':run(sys.executable,str(here/'seal.py'),'target/review-source'),
          'source_origin':'git archive baseline, then prepare.py; exported research tree, not a clean production checkout',
          'archive':identity(root/'target/base-source.tar'), 'lock':identity(root/'target/review-source/Cargo.lock'),
          'binaries':{p.name:identity(p) for p in (root/'target/binaries').glob('*-review.exe')},
          'inputs':{scale:{p.name:identity(p) for p in (frozen/'inputs'/('urban-'+scale)).iterdir() if p.is_file()}
                    for scale in ['10k','100k']},
          'plans':{scale:identity(frozen/'plans'/(scale+'-performance.toml')) for scale in ['10k','100k']},
          'admission':{}}
for scale in ['10k','100k']:
    path = root/'target'/('current-admission-'+scale)
    admission = json.loads((path/'result.json').read_text())
    diagnostics = json.loads((path/'diagnostics.json').read_text())
    result['admission'][scale] = {'status':admission['status'],'completed_ticks':admission['completed_ticks'],
        'result':identity(path/'result.json'),'diagnostics':diagnostics}
(evidence/'identity.json').write_text(json.dumps(result,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
print(result['source_hash'])
