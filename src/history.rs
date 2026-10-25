use crate::edit::{difference, same_snapshot, EditRange};
use slint::SharedString;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

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
    single_line: bool,
}
pub struct History {
    text: SharedString,
    selection: Selection,
    undo: VecDeque<Change>,
    undo_bytes: usize,
    redo: Vec<Change>,
    last_edit: Instant,
}
impl History {
    pub fn new(text: SharedString) -> Self {
        Self {
            text,
            selection: Selection::default(),
            undo: VecDeque::new(),
            undo_bytes: 0,
            redo: Vec::new(),
            last_edit: Instant::now(),
        }
    }
    pub fn selection(&mut self, text: &SharedString, selection: Selection) -> bool {
        if !same_snapshot(&self.text, text) {
            return false;
        }

        self.selection = selection;

        true
    }

    pub fn record(&mut self, text: SharedString, selection: Selection) -> Option<EditRange> {
        if same_snapshot(&self.text, &text) {
            self.selection = selection;

            return None;
        }

        let Some(edit) = difference(&self.text, &text) else {
            self.text = text;
            self.selection = selection;

            return None;
        };

        let change = Change {
            start: edit.start,
            removed: self.text[edit.start..edit.old_end].into(),
            inserted: text[edit.start..edit.new_end].into(),
            before: self.selection,
            after: selection,
            single_line: !text[edit.start..edit.new_end].contains('\n'),
        };
        let now = Instant::now();
        self.undo_bytes += change.removed.len() + change.inserted.len();
        let merge = self.undo.back_mut().filter(|last| {
            last.removed.is_empty()
                && change.removed.is_empty()
                && last.start + last.inserted.len() == change.start
                && last.single_line
                && change.single_line
                && now.duration_since(self.last_edit) < Duration::from_millis(700)
        });
        if let Some(last) = merge {
            last.inserted.push_str(&change.inserted);
            last.after = selection;
        } else {
            self.undo.push_back(change);
        }
        // Bound history by both operations and retained text.
        while self.undo.len() > 200 || (self.undo.len() > 1 && self.undo_bytes > 16 * 1024 * 1024) {
            let removed = self.undo.pop_front().unwrap();
            self.undo_bytes -= removed.removed.len() + removed.inserted.len();
        }
        self.redo.clear();
        self.text = text;
        self.selection = selection;
        self.last_edit = now;

        Some(edit)
    }
    pub fn step(&mut self, redo: bool) -> Option<(SharedString, Selection)> {
        let change = if redo {
            self.redo.pop()?
        } else {
            let change = self.undo.pop_back()?;
            self.undo_bytes -= change.removed.len() + change.inserted.len();
            change
        };
        let mut text = self.text.to_string();

        if redo {
            text.replace_range(
                change.start..change.start + change.removed.len(),
                &change.inserted,
            );
            self.selection = change.after;
            self.undo_bytes += change.removed.len() + change.inserted.len();
            self.undo.push_back(change);
        } else {
            text.replace_range(
                change.start..change.start + change.inserted.len(),
                &change.removed,
            );
            self.selection = change.before;
            self.redo.push(change);
        }
        self.text = text.into();
        self.last_edit = Instant::now() - Duration::from_secs(1);
        Some((self.text.clone(), self.selection))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_history_byte_accounting_survives_trim_undo_and_redo() {
        let mut history = History::new("".into());
        for index in 0..250 {
            history.record(format!("{index} 🦀\n").into(), Selection::default());
            let bytes: usize = history
                .undo
                .iter()
                .map(|change| change.removed.len() + change.inserted.len())
                .sum();
            assert_eq!(history.undo_bytes, bytes);
            assert!(history.undo.len() <= 200);
        }
        let expected = history.text.clone();
        while history.step(false).is_some() {}
        assert_eq!(history.undo_bytes, 0);
        while history.step(true).is_some() {}
        assert_eq!(history.text, expected);
        assert_eq!(
            history.undo_bytes,
            history
                .undo
                .iter()
                .map(|change| change.removed.len() + change.inserted.len())
                .sum::<usize>()
        );
    }

    #[test]
    fn history_shares_text_and_round_trips_unicode_replacement() {
        let old: SharedString = "prefix\nğ🦀\ntail".into();
        let mut history = History::new(old.clone());
        assert_eq!(history.text.as_ptr(), old.as_ptr());
        history.selection(
            &old,
            Selection {
                anchor: 7,
                cursor: 13,
            },
        );
        let new: SharedString = "prefix\nö\ntail".into();
        history.record(
            new.clone(),
            Selection {
                anchor: 9,
                cursor: 9,
            },
        );
        assert_eq!(history.text.as_ptr(), new.as_ptr());
        let (text, selection) = history.step(false).unwrap();
        assert_eq!(text, old);
        assert_eq!(selection.anchor, 7);
        assert_eq!(selection.cursor, 13);
        let (text, _) = history.step(true).unwrap();
        assert_eq!(text, new);
    }
}
