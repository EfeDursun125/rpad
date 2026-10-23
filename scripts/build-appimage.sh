#!/usr/bin/env bash
set -euo pipefail

# Pin both executable inputs; verify them before running the packaging tool.
tool_version=1.9.1
tool_sha256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
runtime_version=20251108
runtime_sha256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT

curl --fail --location --proto '=https' --tlsv1.2 \
    "https://github.com/AppImage/appimagetool/releases/download/$tool_version/appimagetool-x86_64.AppImage" \
    --output "$work_dir/appimagetool"
curl --fail --location --proto '=https' --tlsv1.2 \
    "https://github.com/AppImage/type2-runtime/releases/download/$runtime_version/runtime-x86_64" \
    --output "$work_dir/runtime"

printf '%s  %s\n' "$tool_sha256" "$work_dir/appimagetool" \
    "$runtime_sha256" "$work_dir/runtime" | sha256sum --check --strict
chmod +x "$work_dir/appimagetool"

APPIMAGELAUNCHER_DISABLE=1 APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 VERSION=0.1.1 \
    "$work_dir/appimagetool" --no-appstream --runtime-file "$work_dir/runtime" \
    target/RPad.AppDir dist/RPad-0.1.1-x86_64.AppImage
