use crate::edit::{difference, same_snapshot, EditRange};
use crate::LineRow;
use slint::{Model, ModelTracker, SharedString, VecModel};
use std::cell::{Cell, RefCell};

#[derive(Default)]
pub struct LineModel {
    rows: VecModel<LineRow>,
    text: RefCell<SharedString>,
    starts: RefCell<Vec<usize>>,
    rows_current: Cell<bool>,
    position_cache: Cell<Option<(usize, usize, usize)>>,
}

impl LineModel {
    pub fn sync(&self, text: SharedString, edit: Option<EditRange>, visible: bool) -> usize {
        let mut previous = self.text.borrow_mut();
        let mut starts = self.starts.borrow_mut();
        let initial = starts.is_empty();
        let edit = if initial || same_snapshot(&previous, &text) {
            None
        } else {
            edit.or_else(|| difference(&previous, &text))
        };

        let mut changed_rows = None;

        if initial {
            self.position_cache.set(None);
            *starts = line_starts(&text, 0);
        } else if let Some(edit) = edit {
            if self
                .position_cache
                .get()
                .is_some_and(|(_, offset, _)| offset > edit.start)
            {
                self.position_cache.set(None);
            }

            let first = starts.partition_point(|start| *start <= edit.start) - 1;
            let last = starts.partition_point(|start| *start <= edit.old_end) - 1;
            let old_end = starts.get(last + 1).copied().unwrap_or(previous.len());
            let new_end = old_end - edit.old_end + edit.new_end;
            let region_start = starts[first];
            let mut replacement = line_starts(&text[region_start..new_end], region_start);

            // A final newline belongs to the unchanged next line, except at EOF.
            if last + 1 < starts.len() {
                replacement.pop();
            }

            let old_count = last + 1 - first;
            let new_count = replacement.len();
            let at_end = last + 1 == starts.len();

            if edit.old_end != edit.new_end {
                for start in &mut starts[last + 1..] {
                    *start = *start - edit.old_end + edit.new_end;
                }
            }

            starts.splice(first..last + 1, replacement);
            changed_rows = Some((first, old_count, new_count, at_end));
        }

        *previous = text;
        let count = starts.len();

        if !visible {
            if self.rows_current.replace(false) {
                self.rows.set_vec(Vec::new());
            }

            return count;
        }

        let row = |index, animate_on_create| {
            let start = starts[index];
            let end = starts
                .get(index + 1)
                .map_or(previous.len(), |next| next - 1);

            LineRow {
                content: previous[start..end].into(),
                shown: true,
                animate_on_create,
            }
        };

        if !self.rows_current.replace(true) {
            self.rows.set_vec(
                (0..count)
                    .map(|index| row(index, false))
                    .collect::<Vec<_>>(),
            );
        } else if let Some((first, old_count, new_count, at_end)) = changed_rows {
            let previous_total = self.rows.row_count();

            if at_end {
                // Reuse fading trailing rows during rapid delete/insert sequences.
                for index in first..count {
                    self.set_row(index, row(index, true));
                }
            } else {
                for index in first..first + old_count.min(new_count) {
                    self.set_row(index, row(index, true));
                }

                if new_count > old_count {
                    for index in first + old_count..first + new_count {
                        self.rows.insert(index, row(index, true));
                    }
                } else {
                    for _ in new_count..old_count {
                        self.rows.remove(first + new_count);
                    }
                }
            }

            let retained_count = previous_total.max(count);

            while self.rows.row_count() > retained_count {
                self.rows.remove(self.rows.row_count() - 1);
            }

            while self.rows.row_count() < retained_count {
                self.rows.push(LineRow {
                    content: "".into(),
                    shown: false,
                    animate_on_create: false,
                });
            }

            for index in count..self.rows.row_count() {
                if let Some(mut row) = self.rows.row_data(index) {
                    if row.shown {
                        row.shown = false;
                        self.rows.set_row_data(index, row);
                    }
                }
            }
        }

        count
    }

    fn set_row(&self, index: usize, row: LineRow) {
        if let Some(previous) = self.rows.row_data(index) {
            if previous != row {
                self.rows.set_row_data(index, row);
            }
        } else {
            self.rows.push(row);
        }
    }

    pub fn position(&self, text: &SharedString, offset: usize) -> Option<(usize, usize)> {
        if !same_snapshot(&self.text.borrow(), text) {
            return None;
        }

        let starts = self.starts.borrow();
        let line = starts
            .partition_point(|start| *start <= offset)
            .checked_sub(1)?;
        let (base, column) = self
            .position_cache
            .get()
            .filter(|(cached_line, cached_offset, _)| {
                *cached_line == line && *cached_offset <= offset
            })
            .map(|(_, cached_offset, column)| (cached_offset, column))
            .unwrap_or((starts[line], 1));
        let column = text[base..offset].chars().fold(column - 1, |column, ch| {
            if ch == '\t' {
                let width = i_slint_core::textlayout::DEFAULT_TAB_WIDTH;
                column + width - column % width
            } else {
                column + 1
            }
        }) + 1;
        self.position_cache.set(Some((line, offset, column)));

        Some((line + 1, column))
    }

    pub fn trim(&self, count: usize) {
        while self.rows.row_count() > count {
            self.rows.remove(self.rows.row_count() - 1);
        }
    }
}

fn line_starts(text: &str, base: usize) -> Vec<usize> {
    std::iter::once(base)
        .chain(
            text.bytes()
                .enumerate()
                .filter_map(|(index, byte)| (byte == b'\n').then_some(base + index + 1)),
        )
        .collect()
}

impl Model for LineModel {
    type Data = LineRow;

    fn row_count(&self) -> usize {
        self.rows.row_count()
    }
    fn row_data(&self, row: usize) -> Option<LineRow> {
        self.rows.row_data(row)
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        self.rows.model_tracker()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_rows_skips_animations_but_inserted_rows_animate() {
        let model = LineModel::default();
        model.sync("first\nsecond".into(), None, true);
        assert!(!model.row_data(0).unwrap().animate_on_create);
        assert!(!model.row_data(1).unwrap().animate_on_create);

        model.sync("first\nsecond\nthird".into(), None, true);
        assert!(model.row_data(2).unwrap().animate_on_create);
    }

    fn assert_lines(model: &LineModel, text: &str) {
        let expected: Vec<_> = text.split('\n').collect();
        for (index, expected) in expected.iter().enumerate() {
            let row = model.row_data(index).unwrap();
            assert!(row.shown);
            assert_eq!(row.content.as_str(), *expected);
        }
        assert_eq!(model.starts.borrow().len(), expected.len());
    }

    #[test]
    fn incremental_edits_match_full_split_including_hidden_rows() {
        let model = LineModel::default();
        let mut text = "first\nğ🦀\n\nlast\n".to_owned();
        model.sync(text.clone().into(), None, true);
        let mut seed = 3u64;

        for step in 0..500 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let boundaries: Vec<_> = text
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(text.len()))
                .collect();
            let a = boundaries[seed as usize % boundaries.len()];
            let b = boundaries[(seed >> 32) as usize % boundaries.len()];
            let inserted = ["", "a", "ğ🦀", "\n", "\n\n", "text\nend"][step % 6];
            let mut new = text.clone();
            new.replace_range(a.min(b)..a.max(b), inserted);
            let edit = difference(&text, &new);
            let visible = step % 7 != 0;
            model.sync(new.clone().into(), edit, visible);

            for offset in new
                .char_indices()
                .map(|(index, _)| index)
                .chain(Some(new.len()))
            {
                let prefix = &new[..offset];
                let expected = (
                    prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
                    crate::edit::column(prefix.rsplit('\n').next().unwrap()),
                );
                let snapshot = model.text.borrow().clone();
                assert_eq!(model.position(&snapshot, offset), Some(expected));
            }

            if visible {
                assert_lines(&model, &new);
            } else {
                assert_eq!(model.row_count(), 0);
                model.sync(new.clone().into(), None, true);
                assert_lines(&model, &new);
            }

            text = new;
        }
    }

    #[test]
    fn large_document_edit_reuses_unchanged_rows_and_snapshot() {
        let model = LineModel::default();
        let old: SharedString = "same line\n".repeat(100_000).into();
        model.sync(old.clone(), None, true);
        let first = model.row_data(0).unwrap().content;
        let middle = model.row_data(50_000).unwrap().content;
        let new: SharedString = format!("{old}last").into();
        model.sync(new.clone(), difference(&old, &new), true);

        assert_eq!(model.row_data(0).unwrap().content.as_ptr(), first.as_ptr());
        assert_eq!(
            model.row_data(50_000).unwrap().content.as_ptr(),
            middle.as_ptr()
        );
        assert_eq!(model.text.borrow().as_ptr(), new.as_ptr());
        assert_eq!(model.position(&new, new.len()), Some((100_001, 5)));
    }
}

#[cfg(test)]
mod benchmarks {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    #[ignore = "manual comparison of line-update cost, not full editor latency"]
    fn compare_large_document_line_update_cost() {
        let old: SharedString = "same line\n".repeat(100_000).into();
        let legacy = VecModel::from(
            old.split('\n')
                .map(|content| LineRow {
                    content: content.into(),
                    shown: true,
                    animate_on_create: false,
                })
                .collect::<Vec<_>>(),
        );
        let model = LineModel::default();
        model.sync(old.clone(), None, true);
        let mut previous = old.clone();
        let mut full = Duration::ZERO;
        let mut incremental = Duration::ZERO;

        for index in 0..20 {
            let text: SharedString = format!("{old}{index}").into();
            let start = Instant::now();

            for (index, content) in text.split('\n').enumerate() {
                let row = LineRow {
                    content: content.into(),
                    shown: true,
                    animate_on_create: false,
                };
                if legacy.row_data(index).as_ref() != Some(&row) {
                    legacy.set_row_data(index, row);
                }
            }

            full += start.elapsed();
            let start = Instant::now();
            let edit = difference(&previous, &text);
            model.sync(text.clone(), edit, true);
            incremental += start.elapsed();
            previous = text;
        }

        eprintln!("100,001 rows; mean legacy full-row update {:?}; incremental update including diff {:?}",
            full / 20, incremental / 20);
    }
}
