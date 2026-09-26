"""Exact archived matrix: labels alone or a total count cannot prove coverage."""
import json

def validate_inventory(paths):
    expected = set()
    order = ['all', 'all', 'p3', 'waiting', 'both', 'both', 'waiting', 'p3', 'all']
    for scale in ['10k', '100k']:
        for number, mode in enumerate(order, 1):
            expected.add(('review-screen', f'{scale}-{number}-{mode}', scale, mode, 4, 512, False))
        for mode in ['all', 'p3', 'waiting', 'both']:
            expected.add(('review-counts', f'{scale}-{mode}', scale, mode, 1, 256, True))
    for mode in ['all', 'both']:
        expected.add(('review-long', f'10k-{mode}', '10k', mode, 4, 16384, False))
    actual, ids, directories = set(), set(), set()
    for path in paths:
        meta = json.loads(path.read_text(encoding='utf-8-sig'))
        key = (path.parent.name, meta['label'], meta['scale'], meta['mode'],
               meta['workers'], meta['ticks'], meta['diagnostic'])
        directory = (path.parent / meta['label']).resolve()
        assert key not in actual, 'duplicate experiment'
        assert meta['run_id'] not in ids, 'duplicate run identity'
        assert directory not in directories, 'duplicate run directory'
        assert path.name == meta['label'] + '.process.json', 'metadata filename mismatch'
        actual.add(key)
        ids.add(meta['run_id'])
        directories.add(directory)
    assert actual == expected, 'experiment matrix mismatch'
