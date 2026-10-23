use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default)]
pub struct Selection {
    pub anchor: i32,
    pub cursor: i32,
}
struct Change {
    start: usize,
    removed: String,
    inserted: String,
    before: Selection,
    after: Selection,
}
pub struct History {
    text: String,
    selection: Selection,
    undo: Vec<Change>,
    redo: Vec<Change>,
    last_edit: Instant,
}
impl History {
    pub fn new(text: String) -> Self {
        Self {
            text,
            selection: Selection::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: Instant::now(),
        }
    }
    pub fn selection(&mut self, text: &str, selection: Selection) {
        if self.text == text {
            self.selection = selection;
        }
    }
    pub fn record(&mut self, text: &str, selection: Selection) {
        if text == self.text {
            self.selection = selection;
            return;
        }
        let mut start = self
            .text
            .bytes()
            .zip(text.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        while !self.text.is_char_boundary(start) || !text.is_char_boundary(start) {
            start -= 1;
        }
        let mut suffix = self.text[start..]
            .bytes()
            .rev()
            .zip(text[start..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        while !self.text.is_char_boundary(self.text.len() - suffix)
            || !text.is_char_boundary(text.len() - suffix)
        {
            suffix -= 1;
        }
        let change = Change {
            start,
            removed: self.text[start..self.text.len() - suffix].into(),
            inserted: text[start..text.len() - suffix].into(),
            before: self.selection,
            after: selection,
        };
        let now = Instant::now();
        let merge = self.undo.last_mut().filter(|last| {
            last.removed.is_empty()
                && change.removed.is_empty()
                && last.start + last.inserted.len() == change.start
                && !last.inserted.contains('\n')
                && !change.inserted.contains('\n')
                && now.duration_since(self.last_edit) < Duration::from_millis(700)
        });
        if let Some(last) = merge {
            last.inserted.push_str(&change.inserted);
            last.after = selection;
        } else {
            self.undo.push(change);
        }
        // Bound history by both operations and retained text.
        while self.undo.len() > 200
            || (self.undo.len() > 1
                && self
                    .undo
                    .iter()
                    .map(|c| c.removed.len() + c.inserted.len())
                    .sum::<usize>()
                    > 16 * 1024 * 1024)
        {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.text = text.into();
        self.selection = selection;
        self.last_edit = now;
    }
    pub fn step(&mut self, redo: bool) -> Option<(String, Selection)> {
        let change = if redo {
            self.redo.pop()?
        } else {
            self.undo.pop()?
        };
        if redo {
            self.text.replace_range(
                change.start..change.start + change.removed.len(),
                &change.inserted,
            );
            self.selection = change.after;
            self.undo.push(change);
        } else {
            self.text.replace_range(
                change.start..change.start + change.inserted.len(),
                &change.removed,
            );
            self.selection = change.before;
            self.redo.push(change);
        }
        self.last_edit = Instant::now() - Duration::from_secs(1);
        Some((self.text.clone(), self.selection))
    }
}
