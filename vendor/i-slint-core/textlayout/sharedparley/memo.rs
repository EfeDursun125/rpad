// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Bounded paragraph shaping memo, owned by the renderer (not a global font cache).
//! Line breaking mutates the returned layout, so entries always hold an unbroken copy.
use super::{
    shaping::{Brush, LayoutWithoutLineBreaksBuilder},
    *,
};
use crate::syntax_highlight::ColorSpan;
use std::string::String;
use std::{
    collections::{HashMap, VecDeque},
    hash::{Hash, Hasher},
};

const MAX_ENTRIES: usize = 4096;
const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(PartialEq)]
pub(super) struct Style {
    font: Option<FontRequest>,
    wrap: TextWrap,
    stroke: Option<TextStrokeStyle>,
    scale: ScaleFactor,
    overflow: bool,
}

impl Style {
    pub(super) fn new(builder: &LayoutWithoutLineBreaksBuilder) -> Self {
        Self {
            font: builder.font_request.clone(),
            wrap: builder.text_wrap,
            stroke: builder.stroke,
            scale: builder.scale_factor,
            overflow: builder.overflow_wrap_anywhere,
        }
    }
}

type Key = (u64, usize);

struct Entry {
    text: String,
    colors: Vec<ColorSpan>,
    layout: parley::Layout<Brush>,
    cost: usize,
    used: u64,
}

#[derive(Default)]
pub(super) struct ParagraphMemo {
    styles: Vec<Style>,
    entries: HashMap<Key, Entry>,
    order: VecDeque<(Key, u64)>,
    bytes: usize,
    clock: u64,
    #[cfg(feature = "testing")]
    hits: u64,
}

impl ParagraphMemo {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn key(
        &mut self,
        builder: &LayoutWithoutLineBreaksBuilder,
        text: &str,
        colors: &[ColorSpan],
    ) -> Key {
        let style = Style::new(builder);
        let index = if let Some(index) = self.styles.iter().position(|cached| *cached == style) {
            index
        } else {
            // Font/theme/scale changes cannot accumulate unbounded style or font data.
            if self.styles.len() == 32 {
                self.clear();
            }
            self.styles.push(style);
            self.styles.len() - 1
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        index.hash(&mut hasher);
        text.hash(&mut hasher);
        for span in colors {
            span.range.hash(&mut hasher);
            span.color.as_argb_encoded().hash(&mut hasher);
        }
        (hasher.finish(), index)
    }

    pub(super) fn get(
        &mut self,
        key: Key,
        text: &str,
        colors: &[ColorSpan],
    ) -> Option<parley::Layout<Brush>> {
        let entry = self.entries.get_mut(&key)?;
        // Validate the complete key as well as its hash; collisions must never change text.
        if entry.text != text || entry.colors != colors {
            return None;
        }
        self.clock = self.clock.wrapping_add(1);
        entry.used = self.clock;
        self.order.push_back((key, self.clock));
        #[cfg(feature = "testing")]
        {
            self.hits += 1;
        }
        let layout = entry.layout.clone();
        self.compact_order();
        Some(layout)
    }

    pub(super) fn insert(
        &mut self,
        key: Key,
        text: &str,
        colors: &[ColorSpan],
        layout: &parley::Layout<Brush>,
    ) {
        // Conservative accounting for glyph/cluster/run arrays; giant paragraphs bypass the
        // memo instead of keeping another copy of a long unbroken file in memory.
        let cost = 1024 + text.len().saturating_mul(256) + std::mem::size_of_val(colors);
        if cost > MAX_BYTES / 8 {
            return;
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes -= previous.cost;
        }
        while self.bytes + cost > MAX_BYTES || self.entries.len() >= MAX_ENTRIES {
            let Some((old_key, used)) = self.order.pop_front() else {
                break;
            };
            if self
                .entries
                .get(&old_key)
                .is_some_and(|entry| entry.used == used)
            {
                self.bytes -= self.entries.remove(&old_key).unwrap().cost;
            }
        }
        self.clock = self.clock.wrapping_add(1);
        self.entries.insert(
            key,
            Entry {
                text: text.into(),
                colors: colors.into(),
                layout: layout.clone(),
                cost,
                used: self.clock,
            },
        );
        self.order.push_back((key, self.clock));
        self.bytes += cost;
        self.compact_order();
    }

    #[cfg(feature = "testing")]
    pub(super) fn usage(&self) -> (usize, usize, usize, u64) {
        (self.entries.len(), self.bytes, self.order.len(), self.hits)
    }

    fn compact_order(&mut self) {
        if self.order.len() > self.entries.len() * 4 + 64 {
            self.order.retain(|(key, used)| {
                self.entries
                    .get(key)
                    .is_some_and(|entry| entry.used == *used)
            });
        }
    }
}
