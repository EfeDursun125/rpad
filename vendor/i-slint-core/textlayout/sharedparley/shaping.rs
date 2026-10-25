// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

// cSpell: ignore uncacheable unrepresentable

//! From text to shaped paragraphs.
//!
//! Everything that ends up in a cache entry is produced here, and [`shape_paragraphs`] is the
//! one function it happens through: measuring and drawing must register identical cache
//! dependencies, so neither may shape through anything narrower.

use super::*;
use crate::SharedString;
use unicode_width::UnicodeWidthChar;

/// Font size of inline `code` runs, as a fraction of the surrounding body
/// text. Matches the convention used by GitHub-style markdown renderers — the
/// glyphs sit a little smaller than body text, inside a translucent capsule
/// that visually marks them as code.
const INLINE_CODE_FONT_SCALE: f32 = 0.85;

std::thread_local! {
    static LAYOUT_CONTEXT: RefCell<parley::LayoutContext<Brush>> = Default::default();
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub(super) struct Brush {
    /// When set, this overrides the fill/stroke to use this color.
    pub(super) override_fill_color: Option<Color>,
    pub(super) stroke: Option<TextStrokeStyle>,
    pub(super) link_color: Option<Color>,
}

pub(super) struct LayoutWithoutLineBreaksBuilder {
    pub(super) font_request: Option<FontRequest>,
    pub(super) text_wrap: TextWrap,
    pub(super) stroke: Option<TextStrokeStyle>,
    pub(super) scale_factor: ScaleFactor,
    pub(super) pixel_size: LogicalLength,
    /// When false, overlong words are not broken up. Only used to measure the
    /// min-content width (the longest word), never to lay text out for display.
    pub(super) overflow_wrap_anywhere: bool,
    line_ratio: RefCell<Option<Option<f32>>>,
    tab_space: RefCell<Option<f32>>,
    syntax_lines: crate::syntax_highlight::SyntaxLines,
}

impl LayoutWithoutLineBreaksBuilder {
    pub(super) fn new(
        font_request: Option<FontRequest>,
        text_wrap: TextWrap,
        stroke: Option<TextStrokeStyle>,
        scale_factor: ScaleFactor,
    ) -> Self {
        let pixel_size = font_request
            .as_ref()
            .and_then(|font_request| font_request.pixel_size)
            .unwrap_or(DEFAULT_FONT_SIZE);

        Self {
            font_request,
            text_wrap,
            stroke,
            scale_factor,
            pixel_size,
            overflow_wrap_anywhere: true,
            syntax_lines: Default::default(),
            line_ratio: RefCell::new(None),
            tab_space: RefCell::new(None),
        }
    }

    fn ranged_builder<'a>(
        &self,
        layout_ctx: &'a mut parley::LayoutContext<Brush>,
        font_ctx: &'a mut parley::FontContext,
        text: &'a str,
    ) -> parley::RangedBuilder<'a, Brush> {
        // Use the requested font's natural line-height ratio for every run so fallback fonts,
        // such as the symbol font used for password characters, don't enlarge the line box.
        // `FontSizeRelative` scales the result with each styled span's font size.
        let line_height_ratio = *self.line_ratio.borrow_mut().get_or_insert_with(||
            self.font_request.as_ref().and_then(|font| line_height_ratio(font_ctx, font)));

        let mut builder = layout_ctx.ranged_builder(font_ctx, text, self.scale_factor.get(), false);

        if let Some(ratio) = line_height_ratio {
            builder.push_default(parley::StyleProperty::LineHeight(
                parley::style::LineHeight::FontSizeRelative(ratio),
            ));
        }

        if let Some(ref font_request) = self.font_request {
            let mut fallback_family_iter = sharedfontique::FALLBACK_FAMILIES
                .into_iter()
                .map(parley::style::FontFamilyName::Generic);

            let font_families: &[parley::style::FontFamilyName] = if let Some(family) =
                &font_request.family
            {
                let mut iter =
                    core::iter::once(parley::style::FontFamilyName::named(family.as_str()))
                        .chain(fallback_family_iter);
                &core::array::from_fn::<
                    _,
                    { sharedfontique::FALLBACK_FAMILIES.as_slice().len() + 1 },
                    _,
                >(|_| iter.next().unwrap())
            } else {
                &core::array::from_fn::<_, { sharedfontique::FALLBACK_FAMILIES.as_slice().len() }, _>(
                    |_| fallback_family_iter.next().unwrap(),
                )
            };

            builder.push_default(parley::style::FontFamily::List(std::borrow::Cow::Borrowed(
                font_families,
            )));

            if let Some(weight) = font_request.weight {
                builder.push_default(parley::StyleProperty::FontWeight(
                    parley::style::FontWeight::new(weight as f32),
                ));
            }
            if let Some(letter_spacing) = font_request.letter_spacing {
                builder.push_default(parley::StyleProperty::LetterSpacing(letter_spacing.get()));
            }
            builder.push_default(parley::StyleProperty::FontStyle(if font_request.italic {
                parley::style::FontStyle::Italic
            } else {
                parley::style::FontStyle::Normal
            }));
        }
        builder.push_default(parley::StyleProperty::FontSize(self.pixel_size.get()));
        builder.push_default(parley::StyleProperty::WordBreak(match self.text_wrap {
            TextWrap::NoWrap => parley::style::WordBreak::KeepAll,
            TextWrap::WordWrap => parley::style::WordBreak::Normal,
            TextWrap::CharWrap => parley::style::WordBreak::BreakAll,
        }));
        builder.push_default(parley::StyleProperty::OverflowWrap(
            match (self.text_wrap, self.overflow_wrap_anywhere) {
                (TextWrap::NoWrap, _) | (_, false) => parley::style::OverflowWrap::Normal,
                (TextWrap::WordWrap | TextWrap::CharWrap, true) => {
                    parley::style::OverflowWrap::Anywhere
                }
            },
        ));
        if self.text_wrap == TextWrap::NoWrap {
            // Parley 0.9 removed the width parameter from `Layout::align()` and instead
            // uses the `max_advance` set by `break_all_lines()` as the alignment container
            // width. To allow passing `max_physical_width` to `break_all_lines` for alignment
            // purposes without triggering actual line wrapping, we must set `TextWrapMode::NoWrap`.
            builder.push_default(parley::StyleProperty::TextWrapMode(
                parley::style::TextWrapMode::NoWrap,
            ));
        }

        builder.push_default(parley::StyleProperty::Brush(Brush {
            override_fill_color: None,
            stroke: self.stroke,
            link_color: None,
        }));

        builder
    }

    /// Note that the selection is deliberately absent here: it is a rendering concern, not a
    /// styling one, and baking it into the layout both makes the layout uncacheable across
    /// selection changes and makes sub-glyph selection boundaries unrepresentable. See
    /// [`SelectionSpan`].
    pub(super) fn build(
        &self,
        font_context: &mut parley::FontContext,
        text: &str,
        formatting: impl IntoIterator<Item = i_slint_common::styled_text::FormattedSpan>,
        link_color: Option<Color>,
        colors: &[crate::syntax_highlight::ColorSpan],
    ) -> parley::Layout<Brush> {
        use i_slint_common::styled_text::Style;

        LAYOUT_CONTEXT.with_borrow_mut(|layout_ctx| {
            // Tabs remain one source byte. Shape a wide space at the next four-column stop.
            let has_tabs = text.contains('\t');
            let display = if has_tabs { alloc::borrow::Cow::Owned(text.replace('\t', " ")) }
                else { alloc::borrow::Cow::Borrowed(text) };
            let space = if has_tabs { *self.tab_space.borrow_mut().get_or_insert_with(||
                self.font_request.as_ref().and_then(|font| super::char_size_for_font(font_context, font, ' '))
                .map(|size| size.width).unwrap_or(self.pixel_size.get() * 0.6)) } else { 0. };
            let mut builder = self.ranged_builder(layout_ctx, font_context, &display);
            let mut column = 0;
            for (offset, ch) in text.char_indices().filter(|_| has_tabs) {
                if ch == '\t' {
                    let cells = crate::textlayout::DEFAULT_TAB_WIDTH - column % crate::textlayout::DEFAULT_TAB_WIDTH;
                    builder.push(parley::StyleProperty::LetterSpacing(space * (cells - 1) as f32), offset..offset + 1);
                    column += cells;
                } else { column += ch.width().unwrap_or(0); }
            }
            for span in colors {
                builder.push(parley::StyleProperty::Brush(Brush {
                    override_fill_color: Some(span.color), stroke: self.stroke, link_color: None,
                }), span.range.clone());
            }

            // filter empty ranges otherwise parley will panic on assert
            for span in formatting.into_iter().filter(|s| !s.range.is_empty()) {
                match span.style {
                    Style::Emphasis => {
                        builder.push(
                            parley::StyleProperty::FontStyle(parley::style::FontStyle::Italic),
                            span.range,
                        );
                    }
                    Style::Strikethrough => {
                        builder.push(parley::StyleProperty::Strikethrough(true), span.range);
                    }
                    Style::Strong => {
                        builder.push(
                            parley::StyleProperty::FontWeight(parley::style::FontWeight::BOLD),
                            span.range,
                        );
                    }
                    Style::Code => {
                        builder.push(
                            parley::StyleProperty::FontFamily(parley::style::FontFamily::Single(
                                parley::style::FontFamilyName::Generic(
                                    parley::style::GenericFamily::Monospace,
                                ),
                            )),
                            span.range.clone(),
                        );
                        // Inline `code` reads as slightly smaller text on top of a
                        // translucent capsule (drawn separately in `TextParagraph::draw`),
                        // matching the convention used by common markdown renderers.
                        builder.push(
                            parley::StyleProperty::FontSize(
                                self.pixel_size.get() * INLINE_CODE_FONT_SCALE,
                            ),
                            span.range,
                        );
                    }
                    Style::Underline => {
                        builder.push(parley::StyleProperty::Underline(true), span.range);
                    }
                    Style::Link => {
                        builder.push(parley::StyleProperty::Underline(true), span.range.clone());
                        builder.push(
                            parley::StyleProperty::Brush(Brush {
                                override_fill_color: None,
                                stroke: self.stroke,
                                link_color,
                            }),
                            span.range,
                        );
                    }
                    Style::Color(color) => {
                        builder.push(
                            parley::StyleProperty::Brush(Brush {
                                override_fill_color: Some(crate::Color::from_argb_encoded(color)),
                                stroke: self.stroke,
                                link_color: None,
                            }),
                            span.range,
                        );
                    }
                }
            }

            builder.build(&display)
        })
    }
}

/// The line-height ratio, relative to the font size, that every shaped line gets.
pub(super) fn line_height_ratio(
    font_ctx: &mut parley::FontContext,
    font_request: &FontRequest,
) -> Option<f32> {
    let font = font_request.query_fontique(&mut font_ctx.collection, &mut font_ctx.source_cache)?;
    let face = skrifa::FontRef::from_index(font.blob.data(), font.index).ok()?;
    let location = face.axes().location(font.synthesis.variation_settings());
    let metrics = face.metrics(skrifa::instance::Size::unscaled(), &location);
    let units_per_em = metrics.units_per_em as f32;
    (units_per_em > 0.0)
        .then(|| (metrics.ascent - metrics.descent + metrics.leading) / units_per_em)
        .map(|natural_ratio| {
            font_request.line_height_for_natural_height(natural_ratio).unwrap_or(natural_ratio)
        })
}

/// Splits plain text into paragraph byte ranges at `'\n'`. The `'\n'` and any preceding `'\r'`
/// are excluded from the range: parley treats a lone CR as a mandatory line break, so a CRLF
/// left in the paragraph would render an extra empty line.
pub(super) fn paragraph_ranges(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut start = 0;
    text.split('\n').map(move |paragraph| {
        let end = start + paragraph.len();
        let range = if paragraph.ends_with('\r') { start..end - 1 } else { start..end };
        start = end + 1;
        range
    })
}

pub(super) fn create_text_paragraphs(
    layout_builder: &LayoutWithoutLineBreaksBuilder,
    font_context: &mut parley::FontContext,
    text: PlainOrStyledText,
    link_color: Color,
) -> Vec<TextParagraph> {
    create_text_paragraphs_memo(layout_builder, font_context, text, link_color, None, None)
}

fn create_text_paragraphs_memo(
    layout_builder: &LayoutWithoutLineBreaksBuilder,
    font_context: &mut parley::FontContext,
    text: PlainOrStyledText,
    link_color: Color,
    memo: Option<&RefCell<super::memo::ParagraphMemo>>,
    previous: Option<Vec<TextParagraph>>,
) -> Vec<TextParagraph> {
    let plain = matches!(text, PlainOrStyledText::Plain(_));
    let style = Rc::new(super::memo::Style::new(layout_builder));
    let paragraph_from_text =
        |font_context: &mut parley::FontContext,
         text: &str,
         range: std::ops::Range<usize>,
         formatting: Vec<i_slint_common::styled_text::FormattedSpan>,
         links: Vec<(std::ops::Range<usize>, std::string::String)>,
         colors: &[crate::syntax_highlight::ColorSpan]| {
            let code_ranges: alloc::vec::Vec<Range<usize>> = formatting
                .iter()
                .filter(|s| matches!(s.style, i_slint_common::styled_text::Style::Code))
                .map(|s| s.range.clone())
                .collect();

            let key = memo
                .filter(|_| plain)
                .map(|memo| memo.borrow_mut().key(layout_builder, text, colors));
            let cached = key.and_then(|key| memo.unwrap().borrow_mut().get(key, text, colors));
            let layout = cached.unwrap_or_else(|| {
                let layout =
                    layout_builder.build(font_context, text, formatting, Some(link_color), colors);
                if let Some(key) = key {
                    memo.unwrap()
                        .borrow_mut()
                        .insert(key, text, colors, &layout);
                }
                layout
            });

            TextParagraph {
                range,
                y: PhysicalLength::default(),
                layout,
                links,
                code_ranges,
                retained: None,
                breaking: None,
                tab_markers: Vec::new(),
            }
        };

    let mut paragraphs = Vec::with_capacity(1);

    match text {
        PlainOrStyledText::Plain(ref text) => {
            let ranges: Vec<_> = paragraph_ranges(text).collect();
            paragraphs.reserve(ranges.len());
            let old = previous.unwrap_or_default();
            let matches = |paragraph: &TextParagraph, index: usize| {
                let colors = layout_builder
                    .syntax_lines
                    .get(index)
                    .map_or(&[][..], |line| line.as_slice());
                paragraph.retained.as_ref().is_some_and(|retained| {
                    *retained.style == *style
                        && retained.text.as_str() == &text[ranges[index].clone()]
                        && retained.colors.as_slice() == colors
                })
            };
            let mut prefix = 0;
            while prefix < old.len().min(ranges.len()) && matches(&old[prefix], prefix) {
                prefix += 1;
            }
            let mut suffix = 0;
            while suffix < old.len().min(ranges.len()) - prefix
                && matches(&old[old.len() - 1 - suffix], ranges.len() - 1 - suffix)
            {
                suffix += 1;
            }
            let tail_start = ranges.len() - suffix;
            let mut old = old;
            let tail = old.split_off(old.len() - suffix);
            old.truncate(prefix);
            let mut head = old.into_iter();
            let mut tail = tail.into_iter();
            for (index, range) in ranges.into_iter().enumerate() {
                let mut paragraph = if index < prefix {
                    head.next().unwrap()
                } else if index >= tail_start {
                    tail.next().unwrap()
                } else {
                    let colors = layout_builder
                        .syntax_lines
                        .get(index)
                        .cloned()
                        .unwrap_or_default();
                    let mut paragraph = paragraph_from_text(
                        font_context,
                        &text[range.clone()],
                        range.clone(),
                        Default::default(),
                        Default::default(),
                        colors.as_slice(),
                    );
                    paragraph.tab_markers = text[range.clone()].bytes().enumerate()
                        .filter(|(_, byte)| *byte == b'\t')
                        .map(|(offset, _)| TabMarker { offset, line: 0, rect: PhysicalRect::zero() })
                        .collect();
                    paragraph.retained = Some(RetainedParagraph {
                        text: text[range.clone()].into(),
                        colors,
                        style: style.clone(),
                    });
                    paragraph
                };
                paragraph.range = range;
                paragraph.y = PhysicalLength::zero();
                paragraphs.push(paragraph);
            }
        }
        PlainOrStyledText::Styled(rich_text) => {
            for paragraph in rich_text.paragraphs {
                paragraphs.push(paragraph_from_text(
                    font_context,
                    &paragraph.text,
                    0..0,
                    paragraph.formatting,
                    paragraph.links,
                    &[],
                ));
            }
        }
    };

    paragraphs
}

/// The builder the shaped paragraphs of `text` must be produced with. Measuring and drawing share
/// cache entries, so they have to agree on every input baked into the shaping -- which is why this
/// lives in one place rather than at each call site.
pub(super) fn shaping_builder(
    text: Pin<&dyn crate::item_rendering::RenderString>,
    item_rc: Option<&crate::item_tree::ItemRc>,
    text_wrap: TextWrap,
    scale_factor: ScaleFactor,
) -> LayoutWithoutLineBreaksBuilder {
    let (stroke_brush, _, stroke_style) = text.stroke();
    LayoutWithoutLineBreaksBuilder::new(
        item_rc.map(|irc| text.font_request(irc)),
        text_wrap,
        (!stroke_brush.is_transparent()).then_some(stroke_style),
        scale_factor,
    )
}

/// The builder for measuring content widths, without an item to derive one from.
///
/// `WordWrap` gives `WordBreak::Normal`, so the min-content width becomes the longest word.
/// Content widths are intrinsic to the text, so they don't depend on the item's actual wrap mode.
/// `overflow_wrap_anywhere` is off because parley may otherwise break anywhere to keep overlong
/// words from overflowing, which would make the min-content width a single character instead of
/// the longest word.
pub(super) fn content_widths_builder(
    font_request: FontRequest,
    scale_factor: ScaleFactor,
) -> LayoutWithoutLineBreaksBuilder {
    let mut builder = LayoutWithoutLineBreaksBuilder::new(
        Some(font_request),
        TextWrap::WordWrap,
        None,
        scale_factor,
    );
    builder.overflow_wrap_anywhere = false;
    builder
}

/// A builder for tests, which have no item to derive one from. Everything else obtains its
/// builder through [`shaping_builder`] or [`content_widths_builder`], so that it cannot disagree
/// with what the item's cache entry was shaped with.
#[cfg(test)]
pub(super) fn plain_builder_for_tests() -> LayoutWithoutLineBreaksBuilder {
    LayoutWithoutLineBreaksBuilder::new(None, TextWrap::NoWrap, None, ScaleFactor::new(1.0))
}

#[cfg(test)]
pub(super) fn wrap_builder_for_tests() -> LayoutWithoutLineBreaksBuilder {
    LayoutWithoutLineBreaksBuilder::new(None, TextWrap::WordWrap, None, ScaleFactor::new(1.0))
}

/// Shapes `text` the way both the drawing and the measuring paths need it, so that they can share
/// one cache entry. `text_wrap` is passed separately because `text_size` measures the unwrapped
/// width of items that are otherwise wrapped.
pub(super) fn shape_paragraphs(
    text: Pin<&dyn crate::item_rendering::RenderString>,
    item_rc: Option<&crate::item_tree::ItemRc>,
    text_wrap: TextWrap,
    scale_factor: ScaleFactor,
    font_context: &mut parley::FontContext,
    memo: Option<&RefCell<super::memo::ParagraphMemo>>,
    previous: Option<Vec<TextParagraph>>,
) -> Vec<TextParagraph> {
    let mut builder = shaping_builder(text, item_rc, text_wrap, scale_factor);
    let content = text.text();
    if let PlainOrStyledText::Plain(ref plain) = content {
        builder.syntax_lines = text.syntax_spans(plain);
    }
    create_text_paragraphs_memo(&builder, font_context, content, text.link_color(), memo, previous)
}

struct RetainedParagraph {
    text: SharedString,
    colors: Rc<Vec<crate::syntax_highlight::ColorSpan>>,
    style: Rc<super::memo::Style>,
}

pub(super) struct TabMarker {
    pub(super) offset: usize,
    pub(super) line: usize,
    pub(super) rect: PhysicalRect,
}

pub(super) struct TextParagraph {
    pub(super) tab_markers: Vec<TabMarker>,
    retained: Option<RetainedParagraph>,
    pub(super) breaking: Option<(Option<PhysicalLength>, parley::Alignment)>,
    pub(super) range: Range<usize>,
    pub(super) y: PhysicalLength,
    pub(super) layout: parley::Layout<Brush>,
    pub(super) links: std::vec::Vec<(Range<usize>, std::string::String)>,
    /// Byte ranges within the paragraph's text that carry `Style::Code`. Drawn with a
    /// translucent rounded background by `draw` for visual parity with common markdown
    /// renderers.
    pub(super) code_ranges: std::vec::Vec<Range<usize>>,
}
