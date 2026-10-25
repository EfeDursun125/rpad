use crate::edit::same_snapshot;
use slint::SharedString;

// One document/query pair, shared by both directions. Keep only offsets, not copies
// of the matching text. Very frequent matches use a count-only bounded fallback.
const MAX_CACHED_MATCHES: usize = 131_072;

#[derive(Default)]
pub struct SearchCache {
    text: SharedString,
    needle: SharedString,
    offsets: Vec<usize>,
    count: usize,
    complete: bool,
}

impl SearchCache {
    pub fn clear(&mut self) {
        self.text = SharedString::default();
        self.needle = SharedString::default();
        self.offsets.clear();
        self.count = 0;
        self.complete = false;
    }

    pub fn find(
        &mut self,
        text: SharedString,
        needle: SharedString,
        start: usize,
        end: usize,
        backwards: bool,
    ) -> (Option<usize>, usize) {
        if needle.is_empty() {
            return (None, 0);
        }

        if !same_snapshot(&self.text, &text) || self.needle != needle {
            self.offsets.clear();
            self.count = 0;
            self.complete = true;

            // Include overlapping occurrences for navigation (str::find/rfind semantics).
            let mut remaining = text.as_str();
            let mut base = 0;
            let mut counted_until = 0;

            while let Some(relative) = remaining.find(needle.as_str()) {
                let offset = base + relative;

                if offset >= counted_until {
                    self.count += 1;
                    counted_until = offset + needle.len();
                }

                if self.offsets.len() < MAX_CACHED_MATCHES {
                    self.offsets.push(offset);
                } else {
                    self.complete = false;
                }

                let advance = offset + text[offset..].chars().next().unwrap().len_utf8();
                base = advance;
                remaining = &text[advance..];
            }

            if !self.complete {
                // Avoid retaining a large allocation when this query cannot use it.
                self.offsets = Vec::new();
            }

            self.text = text;
            self.needle = needle;
        }

        let found = if !self.complete {
            if backwards {
                self.text[..start].rfind(self.needle.as_str()).or_else(|| {
                    self.text[start..]
                        .rfind(self.needle.as_str())
                        .map(|offset| start + offset)
                })
            } else {
                self.text[end..]
                    .find(self.needle.as_str())
                    .map(|offset| end + offset)
                    .or_else(|| self.text[..end].find(self.needle.as_str()))
            }
        } else if backwards {
            let before = self
                .offsets
                .partition_point(|offset| *offset + self.needle.len() <= start);
            before
                .checked_sub(1)
                .and_then(|index| self.offsets.get(index))
                .copied()
                .or_else(|| {
                    self.offsets
                        .last()
                        .copied()
                        .filter(|offset| *offset >= start)
                })
        } else {
            let after = self.offsets.partition_point(|offset| *offset < end);
            self.offsets.get(after).copied().or_else(|| {
                self.offsets
                    .first()
                    .copied()
                    .filter(|offset| *offset + self.needle.len() <= end)
            })
        };

        (found, self.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excessive_matches_use_bounded_fallback() {
        let text: SharedString = "a".repeat(MAX_CACHED_MATCHES + 100).into();
        let mut cache = SearchCache::default();
        let (found, count) = cache.find(text.clone(), "a".into(), 0, 0, false);
        assert_eq!(found, Some(0));
        assert_eq!(count, text.len());
        assert!(!cache.complete && cache.offsets.capacity() == 0);
        assert_eq!(
            cache
                .find(text.clone(), "a".into(), text.len(), text.len(), false)
                .0,
            Some(0)
        );
        assert_eq!(
            cache.find(text.clone(), "a".into(), 0, 0, true).0,
            Some(text.len() - 1)
        );
    }

    #[test]
    fn cached_navigation_matches_string_search_with_overlaps_and_unicode() {
        for text in ["aaaaa", "🦀ğ🦀ğ🦀", "one two one", "nothing"] {
            for needle in ["aa", "🦀ğ", "one", "x"] {
                let mut cache = SearchCache::default();
                let shared: SharedString = text.into();
                for boundary in text
                    .char_indices()
                    .map(|(index, _)| index)
                    .chain(Some(text.len()))
                {
                    for backwards in [false, true] {
                        let expected = if backwards {
                            text[..boundary].rfind(needle).or_else(|| {
                                text[boundary..]
                                    .rfind(needle)
                                    .map(|offset| boundary + offset)
                            })
                        } else {
                            text[boundary..]
                                .find(needle)
                                .map(|offset| boundary + offset)
                                .or_else(|| text[..boundary].find(needle))
                        };
                        let (found, count) = cache.find(
                            shared.clone(),
                            needle.into(),
                            boundary,
                            boundary,
                            backwards,
                        );
                        assert_eq!(
                            found, expected,
                            "{text:?} {needle:?} {boundary} {backwards}"
                        );
                        assert_eq!(count, text.matches(needle).count());
                    }
                }
                let offsets = cache.offsets.as_ptr();
                cache.find(shared, needle.into(), 0, 0, false);
                assert_eq!(offsets, cache.offsets.as_ptr());
                let (found, count) = cache.find("changed".into(), needle.into(), 0, 0, false);
                assert_eq!(found, "changed".find(needle));
                assert_eq!(count, "changed".matches(needle).count());
            }
        }
    }
}
