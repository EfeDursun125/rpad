# Changelog

## 0.1.3

- Avoid starting line-number animations while loading an existing document.
- Poll the WGPU device after presenting frames so completed GPU resources can be reclaimed.

## 0.1.2

- Highlight Lua function calls and logical operators.
- Preserve the last normal window dimensions when closing a maximized or fullscreen window.
- Set the Wayland application identity so window decorations use the RPad icon.
- Add syntax highlighting for Pawn, SourcePawn, GDScript, Java, C# and CMake alongside other common formats.
- Use four-column tab stops and show faint arrows for selected tabs.
- Reuse shaped paragraphs, line breaking, syntax states and search matches across edits.
- Update line numbers incrementally and avoid repeated history size scans.
- Read the Windows theme directly through the registry.

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
