#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
prefix="${PREFIX:-/usr/local}"
case "${1:-}" in
    --user) prefix="$HOME/.local" ;;
    --help|-h) printf 'Usage: ./install.sh [--user]\nDefault prefix: /usr/local (override with PREFIX).\n'; exit 0 ;;
    '') ;;
    *) printf 'Unknown option: %s\n' "$1" >&2; exit 2 ;;
esac
cargo build --release --locked
dirs=("$prefix/bin" "$prefix/share/applications" "$prefix/share/icons/hicolor/scalable/apps")
installer=(install)
if ! mkdir -p -- "${dirs[@]}" 2>/dev/null || [[ ! -w "${dirs[0]}" || ! -w "${dirs[1]}" || ! -w "${dirs[2]}" ]]; then
    if [[ $EUID -eq 0 ]]; then printf 'Could not create installation directories.\n' >&2; exit 1; fi
    installer=(sudo install)
fi
"${installer[@]}" -Dm755 target/release/rpad "$prefix/bin/rpad"
"${installer[@]}" -Dm644 packaging/rpad.desktop "$prefix/share/applications/rpad.desktop"
"${installer[@]}" -Dm644 assets/rpad.svg "$prefix/share/icons/hicolor/scalable/apps/rpad.svg"
for size in 16 32 48 64 128 256; do
    "${installer[@]}" -Dm644 "assets/icons/rpad-$size.png" "$prefix/share/icons/hicolor/${size}x${size}/apps/rpad.png"
done
if command -v update-desktop-database >/dev/null; then update-desktop-database "$prefix/share/applications" 2>/dev/null || true; fi
if command -v gtk-update-icon-cache >/dev/null && [[ -w "$prefix/share/icons/hicolor" ]]; then gtk-update-icon-cache -f -t "$prefix/share/icons/hicolor" 2>/dev/null || true; fi
printf 'Installed RPad in %s/bin\n' "$prefix"
