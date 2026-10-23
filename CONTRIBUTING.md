# Developing RPad

Requires Linux, a current stable Rust toolchain, a C/C++ compiler, pkg-config, Fontconfig and OpenGL/EGL development libraries. After obtaining the source:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

Tests use Slint's backend with a virtual clock; no display server is required. System fonts are needed. Example Ubuntu dependencies are listed in `.github/workflows/ci.yml`. The application requires a suitable desktop portal for file dialogs. File selection runs in the background; save confirmations are drawn inside the application.

When changing the interface, also launch the application and inspect it. Menu rows must remain highlighted while the pointer rests over them, and dropdown text must remain visible when the pointer is stationary. Test rapid typing/deletion, Enter, up/down movement, selection and undo together. Changes to file handling must preserve BOMs, line endings, permissions, symbolic links and external-change conflict detection.

`vendor/i-slint-core` is intentionally included in the repository. Patch origins and scope are documented in `vendor/README.md`. Review these patches when upgrading Slint; do not modify files in the Cargo cache.

When reporting a bug, include your distribution, desktop environment, X11/Wayland session, RPad version and reproduction steps. Do not share personal data or system secrets in sample documents.

For Windows, use `cargo build --release --target x86_64-pc-windows-msvc`. To cross-compile from Linux, use `cargo xwin build --release --target x86_64-pc-windows-msvc`; this requires the Rust target, clang/lld and the Windows SDK. The icon is embedded in the EXE by `build.rs`. In the save confirmation, Cancel must preserve the document, Discard must proceed only with the requested operation, and the window must remain open if saving fails or the file chooser is canceled.
