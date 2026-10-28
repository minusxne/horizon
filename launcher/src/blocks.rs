//! The blocky look's parts (`docs/design-system.md`, "Blocks"): pixel type, pixel icons, panels
//! with a dark outline, a light bevel on top and a darker lip below, sunk wells, and the slots
//! that hold pictures.
use crate::theme::{self, Theme};
use gpui::{
    div, img, prelude::*, px, svg, AnyElement, Div, FontWeight, Hsla, ImageSource, ObjectFit,
    SharedString, Stateful, Svg,
};
use std::borrow::Cow;

/// The icons, served to `svg()` as `icons/<name>.svg`.
pub struct Assets;

impl gpui::AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let name = path
            .strip_prefix("icons/")
            .and_then(|p| p.strip_suffix(".svg"));
        Ok(name.and_then(horizon_theme::icons::get).map(Cow::Borrowed))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(horizon_theme::icons::ALL
            .iter()
            .map(|(name, _)| format!("icons/{name}.svg").into())
            .collect())
    }
}

/// A pixel icon (`horizon_theme::icons`) in `color`.
pub fn icon(name: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .flex_none()
        .size(px(size))
        .text_color(color)
}

/// Text in the pixel face: headings, navigation, buttons, badges.
pub fn pixel(size: f32, weight: FontWeight) -> Div {
    div()
        .font_family(theme::PIXEL)
        .font_weight(weight)
        .text_size(px(size))
        .line_height(px((size * 1.2).round()))
}

/// The light edge along the top of a raised block and the darker lip along its bottom; the
/// block must be `relative`.
pub fn edges(top: Hsla, bottom: Hsla, lip: f32) -> [AnyElement; 2] {
    [
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(theme::BEVEL))
            .bg(top)
            .into_any_element(),
        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .h(px(lip))
            .bg(bottom)
            .into_any_element(),
    ]
}

/// A panel: rows, cards, notices, dialogs.
pub fn panel(t: &Theme) -> Div {
    div()
        .relative()
        .bg(t.panel)
        .border_2()
        .border_color(t.outline)
        .children(edges(t.bevel, t.lip, theme::BEVEL))
}

/// A square slot for an icon: the picture at `source` when there is one, else the first letter of
/// `name` on a raised block.
pub fn slot(source: Option<ImageSource>, name: &str, size: f32, t: &Theme) -> Div {
    let frame = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .overflow_hidden()
        .border_2()
        .border_color(t.outline)
        .bg(t.well);
    match source {
        Some(source) => frame.child(img(source).size(px(size - 4.)).object_fit(ObjectFit::Cover)),
        None => frame.bg(t.raised).child(
            pixel((size * 0.42).round(), FontWeight::BOLD)
                .text_color(t.muted)
                .child(
                    name.chars()
                        .find(|c| c.is_alphanumeric())
                        .unwrap_or('?')
                        .to_uppercase()
                        .to_string(),
                ),
        ),
    }
}

/// Lays the stone texture over an area of `w` by `h` logical pixels.
pub fn stone(image: std::sync::Arc<gpui::RenderImage>, w: f32, h: f32) -> Div {
    let step = crate::art::STONE;
    let (cols, rows) = ((w / step).ceil() as usize, (h / step).ceil() as usize);
    let mut layer = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .overflow_hidden();
    for y in 0..rows {
        for x in 0..cols {
            layer = layer.child(
                img(image.clone())
                    .absolute()
                    .left(px(x as f32 * step))
                    .top(px(y as f32 * step))
                    .size(px(step)),
            );
        }
    }
    layer
}

/// A clickable block in the sidebar: an instance or a server.
pub fn side_item(id: SharedString, selected: bool, t: &Theme) -> Stateful<Div> {
    let t = *t;
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .gap(px(10.))
        .p(px(6.))
        .cursor_pointer()
        .when(selected, |d| {
            d.bg(t.raised)
                .children(edges(t.bevel, t.lip, theme::BEVEL))
                .child(
                    div()
                        .absolute()
                        .left(px(-10.))
                        .top(px(8.))
                        .bottom(px(8.))
                        .w(px(4.))
                        .bg(t.accent),
                )
        })
        .when(!selected, |d| d.hover(move |s| s.bg(t.raised.opacity(0.6))))
}
