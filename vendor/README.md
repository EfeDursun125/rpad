# rpad's Slint patches

`i-slint-core` comes from the crates.io source of Slint 1.18.1. The following
local patches are applied; original license notices and `LICENSES` are retained.

- `items/text.rs::TextInput::visual_representation`: width zero suppresses the
  native caret, including composition, while cursor geometry/IME positioning
  retain their existing paths.
- `items/text.rs`, `items/text_fade.rs`: native edits in the multiline editor with
  a custom caret retain short-lived insertion ranges and deletion snapshots.
  The native selection disambiguates repeated characters; UTF-8 offsets follow
  subsequent edits. Programmatic text changes clear the effects, so switching
  documents and undo do not display stale characters.
- `textlayout/sharedparley.rs`, `textlayout/sharedparley/edit_fade.rs`: opacity is
  applied at drawing time using native selection geometry. Inserted glyphs fade
  in independently; deleted glyphs fade out at their old positions, starting at
  their opacity at deletion time. Existing text is never faded as a whole.
  Cached ghost layouts avoid reshaping on every opacity frame. Selection, hit
  testing, IME, clipboard and saved content still use the actual current text.

Effects last 180 ms. Retention is bounded to 64 insertion ranges and eight
snapshots; deletions over 4096 bytes or from documents over 1 MiB commit without
retaining a deletion snapshot. Active composition uses normal native rendering.

Upstream: https://github.com/slint-ui/slint/tree/v1.18.1/internal/core
`Cargo.toml` applies this source through `[patch.crates-io]`. Cargo cache files
are never edited.

RPad uses a negative TextInput cursor width to disable edit fades as well as the native cursor; zero width keeps edit fades enabled. The renderer clears active effects immediately when the width becomes negative. This convention is local to this patched core.

Selection highlights and selected-glyph clipping share interpolated per-visible-span rectangles (28 ms exponential time constant). Logical selection stays immediate. Negative cursor width disables interpolation; document replacement clears retained geometry.

## Frame pacing and WGPU presentation

`i-slint-backend-winit` and `i-slint-renderer-femtovg` are vendored from the Slint 1.18.1 crates.io packages. Original copyright notices and LICENSES are retained. RPad enables FemtoVG/WGPU by default; the OpenGL renderer remains selectable with `SLINT_BACKEND=winit-femtovg`.

The backend limits render starts to the current monitor refresh rate, capped at 180 Hz, using monotonic time and coalesced nanosecond event-loop deadlines. Missing/invalid monitor data falls back to 60 Hz. Fractional intervals are rounded upward to nanoseconds, without millisecond timer quantization. WGPU chooses Mailbox only when advertised by the surface, otherwise FIFO, and preserves the choice on resize. Maximum queued frame latency is one.

Selection spans are matched by paragraph, line and within-line ordinal, so adding an earlier selected row does not restart existing rows. New backward selections grow from the right edge.

`common/sharedfontique/Inter-VariableFont.ttf` and its license are copied from the pinned `i-slint-common` 1.18.1 package for the upstream core test fixture. Run the selection regression tests with `cargo test --manifest-path vendor/i-slint-core/Cargo.toml --features shared-parley --lib motion_tests`.

RPad also shapes literal tabs at four-column stops without changing source byte offsets. TextInput syntax colors use Syntect grammars with cached per-line parser states; color-only spans preserve font settings, selection foreground and insertion opacity.

Unchanged plain-text paragraphs retain their shaped glyphs and width/alignment-specific
line breaking across edits. Reuse checks text, syntax spans, font, wrap mode, stroke,
and display scale. A renderer-owned paragraph memo additionally reuses repeated lines;
it is limited to 4096 entries and an estimated 32 MiB, with an independently bounded
LRU queue. Giant paragraphs bypass that additional memo. Renderer teardown frees the
memo; font/style changes invalidate the applicable entries. Styled text retains its
original shaping path. Syntax caches share unchanged parser states and color vectors.

Selected literal tabs display a faint, font-independent right arrow. Tab rectangles
are cached with line breaking and sorted by visual line/x position. Painting walks
visible selection spans and binary-searches their markers; it does not rescan text,
reshape arrows, or traverse tabs outside the viewport on selection frames. Marker
clipping shares the existing animated selection edges. Unselected tabs remain blank.
