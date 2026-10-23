#!/usr/bin/env python3
"""Bundle the Windows binary, installer, notices and corresponding source."""
import os
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

root = Path(__file__).resolve().parent.parent
binary = Path(os.environ.get('RPAD_WINDOWS_BINARY', root / 'target/release/rpad.exe'))
output = root / 'dist/RPad-0.1.1-windows-x86_64.zip'
output.parent.mkdir(exist_ok=True)
files = {
    binary: 'RPad.exe',
    root / 'packaging/install-windows.ps1': 'install.ps1',
}
for name in ('README.md', 'LICENSE', 'THIRD_PARTY.md'):
    files[root / name] = name

for name in ('RPad-0.1.1-source.tar.gz', 'third-party-sources.tar.gz'):
    files[root / 'dist' / name] = name

for path in (root / 'dist/licenses').rglob('*'):
    if path.is_file():
        files[path] = str(path.relative_to(root / 'dist'))

# Cargo packages can carry Unix epoch timestamps; ZIP starts at 1980.
with ZipFile(output, 'w', ZIP_DEFLATED, strict_timestamps=False) as archive:
    for path, name in files.items():
        archive.write(path, 'RPad/' + name)

print(output)
