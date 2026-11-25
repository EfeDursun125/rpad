// RPad: bounded piece table for native multiline edits.
// Slint still requires a contiguous SharedString snapshot for layout, IME and clipboard.
use crate::SharedString;
use alloc::vec::Vec;
use core::ops::Range;

const MAX_PIECES: usize = 256;
const MAX_UNUSED_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
struct Piece {
    source: usize,
    range: Range<usize>,
}

#[derive(Default)]
pub(super) struct PieceTable {
    sources: Vec<SharedString>,
    pieces: Vec<Piece>,
    snapshot: SharedString,
    retained: usize,
}

impl PieceTable {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    fn reset(&mut self, text: &SharedString) {
        self.clear();
        self.snapshot = text.clone();
        self.retained = text.len();
        if !text.is_empty() {
            self.sources.push(text.clone());
            self.pieces.push(Piece { source: 0, range: 0..text.len() });
        }
    }

    fn push(pieces: &mut Vec<Piece>, piece: Piece) {
        if piece.range.is_empty() {
            return;
        }
        if let Some(last) = pieces.last_mut() {
            if last.source == piece.source && last.range.end == piece.range.start {
                last.range.end = piece.range.end;
                return;
            }
        }
        pieces.push(piece);
    }

    pub fn replace(
        &mut self,
        text: &SharedString,
        range: Range<usize>,
        replacement: &str,
    ) -> SharedString {
        assert!(range.start <= range.end);
        assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
        if self.snapshot.len() != text.len()
            || self.snapshot.as_str().as_ptr() != text.as_str().as_ptr()
        {
            self.reset(text);
        }

        let mut before = Vec::new();
        let mut after = Vec::new();
        let mut offset = 0;
        for piece in &self.pieces {
            let end = offset + piece.range.len();
            if offset < range.start {
                let keep = (range.start - offset).min(piece.range.len());
                Self::push(&mut before, Piece {
                    source: piece.source,
                    range: piece.range.start..piece.range.start + keep,
                });
            }
            if end > range.end {
                let skip = range.end.saturating_sub(offset);
                Self::push(&mut after, Piece {
                    source: piece.source,
                    range: piece.range.start + skip..piece.range.end,
                });
            }
            offset = end;
        }

        if !replacement.is_empty() {
            let source = self.sources.len();
            self.sources.push(replacement.into());
            self.retained += replacement.len();
            Self::push(&mut before, Piece { source, range: 0..replacement.len() });
        }
        for piece in after {
            Self::push(&mut before, piece);
        }
        self.pieces = before;
        let length = text.len() - range.len() + replacement.len();
        let snapshot = SharedString::from_fragments(
            self.pieces.iter().map(|piece| &self.sources[piece.source][piece.range.clone()]),
            length,
        );

        // Periodically compact obsolete source buffers. The renderer snapshot is
        // already available and can become the new original without another copy.
        if self.pieces.len() > MAX_PIECES
            || self.sources.len() > MAX_PIECES
            || self.retained.saturating_sub(length) > MAX_UNUSED_BYTES
        {
            self.reset(&snapshot);
        } else {
            self.snapshot = snapshot.clone();
        }
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piece_edits_match_utf8_string_and_compact() {
        let mut table = PieceTable::default();
        let mut text: SharedString = "ağ🦀z".into();
        let mut expected = text.as_str().to_owned();
        for round in 0..600 {
            let boundaries: Vec<_> = expected.char_indices().map(|(i, _)| i)
                .chain(core::iter::once(expected.len())).collect();
            let start = boundaries[round % boundaries.len()];
            let end = boundaries[(round % boundaries.len() + 1).min(boundaries.len() - 1)];
            let replacement = if round % 3 == 0 { "ö" } else { "🦀x" };
            expected.replace_range(start..end, replacement);
            text = table.replace(&text, start..end, replacement);
            assert_eq!(text.as_str(), expected);
            assert!(table.sources.len() <= MAX_PIECES);
        }
        let other: SharedString = "new".into();
        assert_eq!(table.replace(&other, 1..2, "Ö"), "nÖw");
    }
}
