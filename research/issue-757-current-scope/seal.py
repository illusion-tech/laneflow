"""Hash the exact reconstructed source; build products never enter this tree."""
import hashlib
import json
from pathlib import Path
import sys

root = Path(sys.argv[1]).resolve()
items = {}
for path in sorted(root.rglob('*')):
    if path.is_file() and not any(part in {'.git', 'target', '__pycache__'} for part in path.relative_to(root).parts):
        items[path.relative_to(root).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
encoded = json.dumps(items, sort_keys=True, separators=(',', ':')).encode()
print(hashlib.sha256(encoded).hexdigest())
if len(sys.argv) > 2:
    Path(sys.argv[2]).write_text(json.dumps(items, indent=2) + '\n', encoding='utf-8')
