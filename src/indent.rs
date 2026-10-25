use crate::history::Selection;
use i_slint_core::textlayout::DEFAULT_TAB_WIDTH;

pub fn indent(text: &str, selection: Selection, reverse: bool) -> (String, Selection) {
    let clamp = |offset: i32| {
        let mut offset = (offset.max(0) as usize).min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    };
    let anchor = clamp(selection.anchor);
    let cursor = clamp(selection.cursor);
    let start = anchor.min(cursor);
    let end = anchor.max(cursor);
    let first = text[..start].rfind('\n').map_or(0, |index| index + 1);
    let last_position = if end > start && text.as_bytes()[end - 1] == b'\n' {
        end - 1
    } else {
        end
    };
    let mut edits = Vec::new();
    let mut line = first;

    loop {
        let removed = if reverse {
            let mut columns = 0;
            let mut bytes = 0;
            for byte in text.as_bytes()[line..].iter().copied() {
                if columns >= DEFAULT_TAB_WIDTH {
                    break;
                }
                match byte {
                    b' ' => {
                        columns += 1;
                        bytes += 1;
                    }
                    b'\t' => {
                        columns += DEFAULT_TAB_WIDTH - columns % DEFAULT_TAB_WIDTH;
                        bytes += 1;
                    }
                    _ => break,
                }
            }
            bytes
        } else {
            0
        };
        let inserted = if reverse { "" } else { "\t" };
        if removed != 0 || !inserted.is_empty() {
            edits.push((line, removed, inserted));
        }
        let Some(next) = text[line..].find('\n').map(|index| line + index + 1) else {
            break;
        };
        if next > last_position {
            break;
        }
        line = next;
    }

    let mut output = String::with_capacity(text.len() + edits.len());
    let mut copied = 0;
    for &(offset, removed, inserted) in &edits {
        output.push_str(&text[copied..offset]);
        output.push_str(inserted);
        copied = offset + removed;
    }
    output.push_str(&text[copied..]);

    let map = |point: usize| {
        let mut mapped = point as isize;
        for &(offset, removed, inserted) in &edits {
            if point >= offset + removed {
                if point == start && start != end && point == offset && removed == 0 {
                    continue;
                }
                mapped += inserted.len() as isize - removed as isize;
            } else if point > offset {
                mapped -= (point - offset) as isize;
            }
        }
        mapped.max(0) as i32
    };

    (
        output,
        Selection {
            anchor: map(anchor),
            cursor: map(cursor),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_lines_indent_and_outdent_without_replacing_text() {
        let text = "local ğ = 1\nprint(ğ)\nnext";
        let end = text.find("next").unwrap() as i32;
        let (indented, selection) = indent(
            text,
            Selection {
                anchor: 0,
                cursor: end,
            },
            false,
        );
        assert_eq!(indented, "\tlocal ğ = 1\n\tprint(ğ)\nnext");
        let (restored, _) = indent(&indented, selection, true);
        assert_eq!(restored, text);
    }
    #[test]
    fn reverse_selection_and_mixed_indentation_preserve_content() {
        let text = "  \tfirst\n    second";
        let (result, selection) = indent(
            text,
            Selection {
                anchor: text.len() as i32,
                cursor: 0,
            },
            true,
        );
        assert_eq!(result, "first\nsecond");
        assert_eq!(selection.cursor, 0);
        assert_eq!(selection.anchor, result.len() as i32);
    }
}
