#!/usr/bin/env python3
"""Collect dependency notices and GPL source packages alongside release artifacts."""
import json
import pathlib
import shutil
import subprocess
import tarfile

root = pathlib.Path(__file__).resolve().parent.parent
output = root / 'dist' / 'licenses'
output.mkdir(parents=True, exist_ok=True)
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version', '1'], cwd=root))
lines = ['RPad dependency notices. Exact versions are pinned in Cargo.lock.', '']
with tarfile.open(root / 'dist' / 'third-party-sources.tar.gz', 'w:gz') as archive:
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        name = f"{package['name']}-{package['version']}"
        source = pathlib.Path(package['manifest_path']).parent
        lines.append(f"{name}: {package['license'] or 'see package license files'} — {package.get('repository') or ''}")
        for candidate in source.iterdir():
            if candidate.name.upper().startswith(('LICENSE', 'COPYING', 'NOTICE')):
                destination = output / name / candidate.name
                destination.parent.mkdir(parents=True, exist_ok=True)
                if candidate.is_dir():
                    shutil.copytree(candidate, destination, dirs_exist_ok=True)
                else:
                    shutil.copy2(candidate, destination)
        if package['name'] != 'rpad' and 'GPL' in (package['license'] or ''):
            archive.add(source, arcname=name, filter=lambda info: None if '/target/' in info.name or '/.git/' in info.name else info)
(output / 'DEPENDENCIES.txt').write_text('\n'.join(lines) + '\n')
print(f'Collected {len(metadata["packages"])} package notices')
