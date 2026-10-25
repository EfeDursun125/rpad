# Third-party sources

RPad's own source code is provided under the GPL-3.0-only terms in `LICENSE`.

## Slint

`vendor/i-slint-core` was obtained from the Slint 1.18.1 source package on crates.io. Original copyright headers and `vendor/i-slint-core/LICENSES` are preserved. RPad's local changes are listed in `vendor/README.md`.

- Source: https://github.com/slint-ui/slint/tree/v1.18.1/internal/core
- License options: GPL-3.0-only, LicenseRef-Slint-Royalty-free-2.0 or LicenseRef-Slint-Software-3.0. RPad distributions use the GPL-3.0-only option.

Exact versions of other Rust dependencies are recorded in `Cargo.lock`; license information is provided in each package. `cargo metadata --locked --format-version 1` lists dependency license declarations. Binary distributions must also comply with the applicable dependency license terms.

## Application icon

`assets/rpad.svg` was adapted from the R icon supplied at the start of the project. Speed trails and the play shape were replaced with a document and text caret.

Vendored Slint 1.18.1 backend/renderer sources and the Inter test-font fixture retain their upstream license notices; origins and local patches are listed in [vendor/README.md](vendor/README.md).

`ui/solid-scrollview.slint` adapts the Material ScrollView from `i-slint-compiler` 1.18.1 with solid themed thumbs and optional hover transitions. Its original copyright and license header are retained.

## Syntax highlighting

Syntect 5.3.0 (MIT) supplies bundled Sublime Text syntax definitions and color themes. Source: https://github.com/trishume/syntect. Its license and dependency notices are included by the release license collector.
