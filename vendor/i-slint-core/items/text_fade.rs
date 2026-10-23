// rpad rendering-only edit effects. Byte offsets always refer to the live text;
// ghosts retain the old shaping input and never participate in editing or saving.
use crate::SharedString;
use alloc::{rc::Rc, vec::Vec};
use core::ops::Range;

pub const DURATION_MS: u64 = 180;
#[derive(Clone)]
pub struct Insertion {
    pub range: Range<usize>,
    pub started: u64,
}
impl Insertion {
    pub fn opacity(&self, now: u64) -> f32 {
        let t = (now.saturating_sub(self.started) as f32 / DURATION_MS as f32).clamp(0., 1.);
        t * t
    }
}
#[derive(Clone)]
pub struct Deletion {
    pub text: SharedString,
    pub range: Range<usize>,
    pub started: u64,
    pub insertions: Vec<Insertion>,
    #[cfg(feature = "shared-parley")]
    pub cache: Rc<crate::textlayout::sharedparley::TextLayoutCache>,
}
impl Deletion {
    pub fn opacity(&self, now: u64) -> f32 {
        let t = (now.saturating_sub(self.started) as f32 / DURATION_MS as f32).clamp(0., 1.);
        (1. - t) * (1. - t)
    }
}
#[derive(Default, Clone)]
pub struct TextFadeState {
    pub insertions: Vec<Insertion>,
    pub deletions: Vec<Deletion>,
}
impl TextFadeState {
    pub fn clear(&mut self) {
        self.insertions.clear();
        self.deletions.clear();
    }
    pub fn prune(&mut self, now: u64) {
        self.insertions
            .retain(|i| now.saturating_sub(i.started) < DURATION_MS);
        self.deletions
            .retain(|d| now.saturating_sub(d.started) < DURATION_MS);
    }
    pub fn record(&mut self, old: &str, new: &str, now: u64, hint: Option<Range<usize>>) {
        self.prune(now);
        if old == new {
            return;
        }
        let mut start = old
            .bytes()
            .zip(new.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        while !old.is_char_boundary(start) || !new.is_char_boundary(start) {
            start -= 1;
        }
        let mut suffix = old[start..]
            .bytes()
            .rev()
            .zip(new[start..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix)
        {
            suffix -= 1;
        }
        let mut old_end = old.len() - suffix;
        let mut new_end = new.len() - suffix;
        // Equal adjacent characters have an ambiguous textual diff. The native
        // selection identifies where this actual edit happened.
        if let Some(range) = hint {
            if range.start <= range.end
                && range.end <= old.len()
                && old.is_char_boundary(range.start)
                && old.is_char_boundary(range.end)
            {
                let inserted = new.len().checked_sub(old.len() - range.len());
                if let Some(end) = inserted.and_then(|len| range.start.checked_add(len)) {
                    if end <= new.len()
                        && new.is_char_boundary(range.start)
                        && new.is_char_boundary(end)
                        && old[..range.start] == new[..range.start]
                        && old[range.end..] == new[end..]
                    {
                        start = range.start;
                        old_end = range.end;
                        new_end = end;
                    }
                }
            }
        }
        // Retain at most eight short-lived shaping snapshots and 64 insertion ranges.
        // Very large deletions commit immediately rather than retaining huge documents.
        if old_end > start && old.len() <= 1024 * 1024 && old_end - start <= 4096 {
            self.deletions.push(Deletion {
                text: old.into(),
                range: start..old_end,
                started: now,
                insertions: self.insertions.clone(),
                #[cfg(feature = "shared-parley")]
                cache: Rc::new(Default::default()),
            });
            if self.deletions.len() > 8 {
                self.deletions.remove(0);
            }
        }
        let mut rebased = Vec::new();
        for insertion in self.insertions.drain(..) {
            let before = insertion.range.start..insertion.range.end.min(start);
            if before.start < before.end {
                rebased.push(Insertion {
                    range: before,
                    started: insertion.started,
                });
            }
            let after = insertion.range.start.max(old_end)..insertion.range.end;
            if after.start < after.end {
                rebased.push(Insertion {
                    range: (after.start - old_end + new_end)..(after.end - old_end + new_end),
                    started: insertion.started,
                });
            }
        }
        if new_end > start {
            rebased.push(Insertion {
                range: start..new_end,
                started: now,
            });
        }
        if rebased.len() > 64 {
            rebased.drain(..rebased.len() - 64);
        }
        self.insertions = rebased;
    }
}
