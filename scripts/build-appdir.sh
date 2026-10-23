#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
binary="${RPAD_BINARY:-target/release/rpad}"
appdir="${RPAD_APPDIR:-target/RPad.AppDir}"
if [[ ! -x "$binary" ]]; then printf 'Missing binary: %s\n' "$binary" >&2; exit 1; fi
mkdir -p "$appdir/usr/bin" "$appdir/usr/lib" "$appdir/usr/share/applications" "$appdir/usr/share/icons/hicolor/256x256/apps" "$appdir/usr/share/icons/hicolor/scalable/apps" "$appdir/usr/share/doc/rpad"
install -m755 "$binary" "$appdir/usr/bin/rpad"
install -m644 packaging/rpad.desktop "$appdir/rpad.desktop"
install -m644 packaging/rpad.desktop "$appdir/usr/share/applications/rpad.desktop"
install -m644 assets/icons/rpad-256.png "$appdir/rpad.png"
install -m644 assets/icons/rpad-256.png "$appdir/usr/share/icons/hicolor/256x256/apps/rpad.png"
install -m644 assets/rpad.svg "$appdir/usr/share/icons/hicolor/scalable/apps/rpad.svg"
ln -sfn rpad.png "$appdir/.DirIcon"
install -m644 LICENSE README.md THIRD_PARTY.md "$appdir/usr/share/doc/rpad/"
# Keep distribution notices for the shared libraries included below.
for notice in /usr/share/doc/lib*/copyright /usr/share/doc/zlib1g/copyright; do
    if [[ -f "$notice" ]]; then
        package="$(basename "$(dirname "$notice")")"
        install -Dm644 "$notice" "$appdir/usr/share/doc/rpad/system-licenses/$package.txt"
    fi
done
# Run this on the oldest supported distribution (the release uses Ubuntu 24.04).
# Never bundle the host's libc or graphics drivers.
copy_dependencies() {
    while IFS= read -r lib; do
        case "$(basename "$lib")" in
            libc.so.*|libm.so.*|libdl.so.*|libpthread.so.*|librt.so.*|ld-linux*|libresolv.so.*|libutil.so.*|libwayland*|libX*|libxcb*|libxkbcommon*|libdrm*|libvulkan*|libGL*|libEGL*) continue ;;
        esac
        install -m755 "$lib" "$appdir/usr/lib/$(basename "$lib")"
    done < <(ldd "$1" | awk '/=> \// {print $3}')
}
# This staging directory is generated; discard libraries from previous builds.
for staged in "$appdir"/usr/lib/*.so*; do
    if [[ -f "$staged" || -L "$staged" ]]; then rm -- "$staged"; fi
done
copy_dependencies "$binary"
# Host display libraries must match the host's graphics drivers.
patchelf --set-rpath '$ORIGIN/../lib' "$appdir/usr/bin/rpad"
for lib in "$appdir"/usr/lib/*.so*; do patchelf --set-rpath '$ORIGIN' "$lib"; done
cat > "$appdir/AppRun" <<'RUN'
#!/usr/bin/env sh
appdir="${APPDIR:-$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)}"
export LD_LIBRARY_PATH="$appdir/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$appdir/usr/bin/rpad" "$@"
RUN
chmod +x "$appdir/AppRun"
printf 'AppDir ready: %s\n' "$appdir"
