// rpad: opacity is applied only while drawing. Shaping, hit testing, IME,
// selection offsets and the actual document remain owned by TextInput.
use super::*;
use crate::SharedString;
use crate::item_rendering::{HasFont, RenderString};
use crate::items::text_fade::{Deletion, Insertion};
use alloc::vec;

pub(super) struct GhostText<'a> {
    pub input: Pin<&'a crate::items::TextInput>,
    pub text: SharedString,
}
impl HasFont for GhostText<'_> {
    fn font_request(self: Pin<&Self>, item: &crate::items::ItemRc) -> FontRequest {
        self.input.font_request(item)
    }
}
impl RenderString for GhostText<'_> {
    fn text(self: Pin<&Self>) -> PlainOrStyledText {
        PlainOrStyledText::Plain(self.text.clone())
    }
}
fn split(
    rect: PhysicalRect,
    cut: PhysicalRect,
    opacity: f32,
    alpha: f32,
    output: &mut Vec<(PhysicalRect, f32)>,
) {
    let Some(hit) = rect.intersection(&cut) else {
        output.push((rect, opacity));
        return;
    };
    let mut add = |x: f32, y: f32, w: f32, h: f32, a: f32| {
        if w > 0. && h > 0. {
            output.push((
                PhysicalRect::new(PhysicalPoint::new(x, y), PhysicalSize::new(w, h)),
                a,
            ));
        }
    };
    add(
        rect.min_x(),
        rect.min_y(),
        hit.min_x() - rect.min_x(),
        rect.height(),
        opacity,
    );
    add(
        hit.max_x(),
        rect.min_y(),
        rect.max_x() - hit.max_x(),
        rect.height(),
        opacity,
    );
    add(
        hit.min_x(),
        rect.min_y(),
        hit.width(),
        hit.min_y() - rect.min_y(),
        opacity,
    );
    add(
        hit.min_x(),
        hit.max_y(),
        hit.width(),
        rect.max_y() - hit.max_y(),
        opacity,
    );
    add(
        hit.min_x(),
        hit.min_y(),
        hit.width(),
        hit.height(),
        opacity * alpha,
    );
}
// Partition, rather than overdrawing a translucent glyph on top of its opaque
// version. Adjacent animated characters keep independent ages during fast typing.
fn tiles(
    layout: &Layout,
    region: PhysicalRect,
    insertions: &[Insertion],
    now: u64,
) -> Vec<(PhysicalRect, f32)> {
    let band = region.origin.y_length()..(region.origin.y_length() + region.height_length());
    let mut tiles = vec![(region, 1.)];
    for insertion in insertions {
        for rect in layout
            .selection_geometry(insertion.range.clone(), &band)
            .backgrounds()
        {
            let mut next = Vec::new();
            for (tile, alpha) in tiles {
                split(tile, rect, alpha, insertion.opacity(now), &mut next);
            }
            tiles = next;
        }
    }
    tiles
}
#[allow(clippy::too_many_arguments)]
pub(super) fn draw<R: GlyphRenderer>(
    layout: &Layout,
    renderer: &mut R,
    size: LogicalSize,
    color: Color,
    selection_spans: &SelectionSpans,
    selection_color: Color,
    insertions: &[Insertion],
    now: u64,
) {
    let scale = renderer.scale_factor();
    let region = renderer.get_current_clip().cast::<f32>() * scale;
    for (rect, alpha) in tiles(layout, region, insertions, now) {
        if alpha <= 0. {
            continue;
        }
        renderer.save_state();
        if renderer.combine_clip(rect / scale, LogicalBorderRadius::zero()) {
            if let Some(brush) = renderer.platform_text_fill_brush(
                color.with_alpha(color.alpha() as f32 / 255. * alpha).into(),
                size,
            ) {
                let selection = renderer
                    .platform_brush_for_color(
                        &selection_color.with_alpha(selection_color.alpha() as f32 / 255. * alpha),
                    )
                    .filter(|_| !selection_spans.is_empty())
                    .map(|foreground| SelectionRendering {
                        spans: selection_spans,
                        foreground,
                    });
                layout.draw(renderer, brush, None, color, selection.as_ref());
            }
        }
        renderer.restore_state();
    }
}
pub(super) fn draw_ghost<R: GlyphRenderer>(
    layout: &Layout,
    renderer: &mut R,
    size: LogicalSize,
    color: Color,
    deletion: &Deletion,
    now: u64,
) {
    let scale = renderer.scale_factor();
    let band = draw::visible_band(renderer);
    let spans = layout.selection_geometry(deletion.range.clone(), &band);
    for region in spans.backgrounds() {
        // Freeze the deleted characters at their opacity at deletion time, then
        // fade them out. Deleting a half-visible character must never brighten it.
        for (rect, alpha) in tiles(layout, region, &deletion.insertions, deletion.started) {
            let alpha = alpha * deletion.opacity(now);
            if alpha <= 0. {
                continue;
            }
            renderer.save_state();
            if renderer.combine_clip(rect / scale, LogicalBorderRadius::zero()) {
                if let Some(brush) = renderer.platform_text_fill_brush(
                    color.with_alpha(color.alpha() as f32 / 255. * alpha).into(),
                    size,
                ) {
                    layout.draw(renderer, brush, None, color, None);
                }
            }
            renderer.restore_state();
        }
    }
}
