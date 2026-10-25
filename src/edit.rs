use slint::SharedString;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditRange {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
}

pub fn same_snapshot(left: &SharedString, right: &SharedString) -> bool {
    left.len() == right.len() && left.as_ptr() == right.as_ptr()
}

// Compare in blocks so unchanged document regions use slice equality, not a
// per-character traversal. Only the changed ranges need owned history storage.
pub fn difference(old: &str, new: &str) -> Option<EditRange> {
    let limit = old.len().min(new.len());
    let mut start = 0;

    while start + 64 <= limit
        && old.as_bytes()[start..start + 64] == new.as_bytes()[start..start + 64]
    {
        start += 64;
    }

    while start < limit && old.as_bytes()[start] == new.as_bytes()[start] {
        start += 1;
    }

    if start == old.len() && start == new.len() {
        return None;
    }

    while !old.is_char_boundary(start) || !new.is_char_boundary(start) {
        start -= 1;
    }

    let mut suffix = 0;
    let remaining = limit - start;

    while suffix + 64 <= remaining
        && old.as_bytes()[old.len() - suffix - 64..old.len() - suffix]
            == new.as_bytes()[new.len() - suffix - 64..new.len() - suffix]
    {
        suffix += 64;
    }

    while suffix < remaining
        && old.as_bytes()[old.len() - suffix - 1] == new.as_bytes()[new.len() - suffix - 1]
    {
        suffix += 1;
    }

    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }

    Some(EditRange {
        start,
        old_end: old.len() - suffix,
        new_end: new.len() - suffix,
    })
}

pub fn column(text: &str) -> usize {
    text.chars().fold(0, |column, ch| {
        if ch == '\t' {
            column + i_slint_core::textlayout::DEFAULT_TAB_WIDTH
                - column % i_slint_core::textlayout::DEFAULT_TAB_WIDTH
        } else {
            column + 1
        }
    }) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn differences_reconstruct_unicode_and_block_boundaries() {
        let old = format!("{}ğ🦀\n{}", "a".repeat(127), "tail".repeat(100));
        let boundaries: Vec<_> = old
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(old.len()))
            .collect();

        for start in boundaries.iter().step_by(17).copied() {
            for end in boundaries
                .iter()
                .copied()
                .filter(|end| *end >= start)
                .step_by(31)
            {
                for inserted in ["", "x", "ö🦀\n"] {
                    let mut new = old.clone();
                    new.replace_range(start..end, inserted);

                    if let Some(edit) = difference(&old, &new) {
                        let mut reconstructed = old.clone();
                        reconstructed.replace_range(
                            edit.start..edit.old_end,
                            &new[edit.start..edit.new_end],
                        );
                        assert_eq!(reconstructed, new);
                    } else {
                        assert_eq!(old, new);
                    }
                }
            }
        }
    }
}
