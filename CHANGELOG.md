# Changelog

## 0.1.1

- Match line-number spacing to the renderer's fractional font metrics, including wrapped lines.
- Verify alignment with multiple fonts, blank lines and fractional display scaling.
- Build release packages with optimization level 3, fat LTO and one codegen unit.

## 0.1.0

- Compact Material 3 text editor built with Rust and Slint.
- System theme tracking, red accent fallback and terminal font detection.
- Smooth text caret, character fades and line-number opacity animations.
- UTF-8, BOM, LF/CRLF, atomic saving and external file change detection.
- Find/replace, undo/redo, line numbers and text size controls.
- Opening files from the terminal, sudo session support and desktop installation.
