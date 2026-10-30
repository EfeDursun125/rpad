# RPad

A small, calm text editor, made for personal use. Built with Rust and Slint, RPad combines Leafpad-style simplicity with Material 3 surfaces, system colors and smooth text animations.

## Installation

Linux builds require a current stable Rust toolchain, a C/C++ compiler, pkg-config, Fontconfig and OpenGL/EGL development libraries. File dialogs require `xdg-desktop-portal` and a working desktop-specific portal backend. Save confirmations, errors and About are drawn inside RPad; file selection and disk operations run on background threads. Portal theme detection uses `gdbus`; KDE colors are read directly.

Run the installer as your normal user:

```sh
./install.sh
rpad ~/notes.txt
```

The installer builds a release binary first, then requests sudo only for installation when necessary. The default prefix is `/usr/local`. It also installs the application icon and desktop entry. Override the destination with `PREFIX=/your/prefix ./install.sh`.

For a per-user installation:

```sh
./install.sh --user
```

This installs to `~/.local/bin`. Since sudo normally does not search that directory, use the full path with a per-user installation:

```sh
sudo "$HOME/.local/bin/rpad" /etc/systemd/zram-generator.conf
```

RPad edits one file per window. A nonexistent filename opens a new document at that path. Quote paths containing spaces. For filenames starting with `-`, use `rpad -- -notes.txt`.

## Controls

| Shortcut | Action |
| --- | --- |
| Ctrl + N / Ctrl + O | New document / open file |
| Ctrl + S / Ctrl + Shift + S | Save / save as |
| Ctrl + Z / Ctrl + Shift + Z or Ctrl + Y | Undo / redo |
| Ctrl + X / Ctrl + C / Ctrl + V / Ctrl + A | Cut / copy / paste / select all |
| Ctrl + F or Ctrl + H | Find and replace |
| Ctrl + Plus / Ctrl + Minus / Ctrl + 0 | Increase / decrease / reset text size |
| Ctrl + Q | Quit |
| Escape | Close the menu or search panel |

The Find toolbar button toggles the search panel. Keyboard shortcuts open it and focus the search field. Tab inserts a real tab character in the editor. Search is case-sensitive plain-text search and wraps around the document. Replacements can be undone. Unsaved changes are checked before creating a new document, opening another file or closing the window.

The View menu controls word wrap, line numbers and smooth animations. Word wrap is enabled by default; saved preferences take precedence. View preferences, text size and window dimensions are saved after changes and on exit. The last window size is restored on launch in logical pixels, including across display scale changes:

- Linux: `$XDG_CONFIG_HOME/rpad/settings.conf`, or `~/.config/rpad/settings.conf`.
- Windows: `%APPDATA%/rpad/settings.conf`.
- Linux sudo sessions: `/root/.config/rpad/settings.conf`, separate from normal-user preferences.

## Syntax highlighting

Known file extensions select syntax colors automatically, including Lua, C/C++, headers,
Pawn (`.sma`, `.pwn`), SourcePawn (`.sp`), GDScript (`.gd`), Java (`.java`), C# (`.cs`,
`.csx`), CMake (`CMakeLists.txt`, `.cmake`), Rust, Python, JavaScript, HTML/CSS, JSON, YAML, TOML, shell scripts and Markdown.
Colors follow the light/dark theme and preserve selection colors and text animations.

## Animations

The text caret is a vertical line, separate from the mouse pointer. It follows Slint's actual text layout using the same time-based exponential interpolation on both axes. A motion timer runs every 8 ms and stops when the caret reaches its target. Idle blinking fades over 360 ms. The caret is hidden while text is selected or the editor is unfocused.

New line numbers and typed characters appear with independent 180 ms fades. Removed line numbers and deleted characters fade out. Existing text does not restart its animation on every keystroke. Deleted characters are rendering-only ghosts: they leave the real document immediately. File changes and programmatic text replacements clear old effects. Memory limits for large deletions are documented in [vendor/README.md](vendor/README.md).

Selection highlights follow the selected range with smooth geometry interpolation. Actual selection offsets, clipboard contents and saved text update immediately. The highlight and selected glyph clipping use the same animated geometry.

Turning off **Smooth animations** disables caret movement interpolation, character fades, line-number fades, selection interpolation and menu hover transitions. Active effects finish immediately; subsequent edits appear without animation.

## Rendering and frame limits

RPad prefers the FemtoVG/WGPU renderer. It selects Mailbox presentation when supported by the surface and FIFO otherwise. Rendering follows the current monitor's refresh rate with a hard upper limit of 180 FPS; missing monitor information falls back to 60 Hz. The limit applies to input, animation and resize redraws, including the optional OpenGL path (`SLINT_BACKEND=winit-femtovg`). Idle windows redraw only when needed.

## System integration

On Linux, KDE/Matugen colors from `kdeglobals` take priority. If unavailable, RPad reads light/dark appearance and accent color through `org.freedesktop.portal.Settings`. The fallback accent is red. A background thread checks theme changes every two seconds and applies them to the open window.

Menus and descriptions use the desktop UI font (KDE, GTK/GNOME, or Windows system font), falling back to Noto Sans when detection fails. The editor font is detected from the launching terminal's Foot, Kitty or Alacritty configuration, then from Konsole's default profile and its inherited KDE fixed-width font. Konsole font weight is preserved. If no terminal setting is found, Fontconfig's `monospace` preference is used.

When started with sudo on Linux, RPad reads the invoking user's KDE theme and uses `SUDO_UID` to restore missing Wayland/X11 and session-path variables. It never elevates itself. File access is limited by the process's permissions, and the graphical session's access policy still applies.

## File behavior

- Supports UTF-8, optional UTF-8 BOM and LF/CRLF line endings. Mixed line endings are preserved. Binary files and other encodings produce an error.
- Saving uses a temporary file in the destination directory, `fsync` and atomic replacement. On Linux, existing ownership and Unix permission bits are preserved; symbolic links are resolved to their target.
- If a file changes externally or is deleted after opening/saving, RPad refuses to overwrite it. Use Save as or reopen the file.
- Files with hard links require Save as to avoid breaking links. ACLs and extended attributes are not copied in this version.
- Write errors leave the document open. Undo history resets when switching documents and is limited to 200 operations / 16 MiB of retained text, with a single large operation retained.

## Windows and AppImage

The Windows x86-64 ZIP contains `RPad.exe`, licenses, corresponding source archives and an optional `install.ps1`. Icons are embedded in the executable and window. The installer does not require administrator access: it installs to `%LOCALAPPDATA%\Programs\RPad` and creates a Start menu shortcut. Pass `-DesktopShortcut` to add a desktop shortcut. Windows appearance and accent colors are read from the system; the editor uses Cascadia Mono. Windows Terminal profile font detection is not implemented.

The Linux x86-64 AppImage is built on Ubuntu 24.04 and requires glibc 2.39 or newer. Graphics drivers and the desktop portal come from the host system. Normal execution requires FUSE. To extract and run instead:

```sh
APPIMAGE_EXTRACT_AND_RUN=1 ./RPad-0.1.3-x86_64.AppImage
```

Package generation is defined in [.github/workflows/packages.yml](.github/workflows/packages.yml). Pushing a version tag or running the workflow manually produces Windows ZIP and AppImage artifacts. Wine checks on Linux are separate from native Windows testing and do not establish compatibility on real Windows hardware.

## Development and validation

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

Tests cover file preservation, external-change conflicts, Unicode editing, find/replace undo, history resets, terminal font selection, settings persistence and animation intermediate values. GitHub Actions checks formatting, tests, Clippy, release builds and installation to a temporary directory. Appearance, file dialogs and sudo launch behavior also need interactive desktop checks. See [CONTRIBUTING.md](CONTRIBUTING.md) for the test environment.

RPad uses Slint 1.18.1's Material style. The editor and themed controls are defined in `ui/app.slint`. Native `TextInput` handles layout, selection, clipboard and input methods. The supplied R SVG was adapted into a text-editor icon by replacing its speed trails and play symbol with a document and caret.

The vendored `i-slint-core` patches suppress the native caret and implement rendering-only character and selection effects. Their scope and provenance are documented in [vendor/README.md](vendor/README.md). Other dependency provenance is recorded in [THIRD_PARTY.md](THIRD_PARTY.md).
