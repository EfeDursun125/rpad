#!/usr/bin/env python3
"""Archive repository sources, including new unignored files, without build outputs."""
import pathlib
import subprocess
import tarfile

root = pathlib.Path(__file__).resolve().parent.parent
paths = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=root).split(b'\0')
output = root / 'dist' / 'RPad-0.1.0-source.tar.gz'
output.parent.mkdir(exist_ok=True)
with tarfile.open(output, 'w:gz') as archive:
    for item in sorted(set(paths)):
        if not item:
            continue
        relative = pathlib.Path(item.decode())
        if (root / relative).is_file():
            archive.add(root / relative, arcname=str(pathlib.Path('RPad-0.1.0') / relative), recursive=False)
print(output)
