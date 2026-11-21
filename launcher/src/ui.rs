//! Small shared primitives. Views only describe layout and behavior.
use gpui::{prelude::*, *};
use mixnet_theme::{color as c, layout as l};
use std::borrow::Cow;

pub fn column() -> Div {
    div().flex().flex_col()
}
pub fn row() -> Div {
    div().flex().items_center()
}
pub fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(c::MUTED))
        .child(text.into())
}
pub fn mono(text: impl Into<SharedString>) -> Div {
    muted(text).font_family(mixnet_theme::MONO)
}
pub fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(34.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}
pub fn icon(name: &'static str) -> Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(17.))
        .text_color(rgb(c::SOFT))
        .flex_shrink_0()
}
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    glyph: &'static str,
    primary: bool,
) -> Stateful<Div> {
    row()
        .id(id)
        .h(px(l::BUTTON))
        .px(px(14.))
        .gap(px(9.))
        .rounded(px(l::RADIUS))
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .bg(rgb(if primary { c::ACCENT } else { c::PLATE }))
        .text_color(rgb(if primary { c::PAGE } else { c::INK }))
        .hover(move |s| s.bg(rgb(if primary { 0xb0a0ff } else { 0x2a2d39 })))
        .child(
            svg()
                .path(format!("icons/{glyph}.svg"))
                .size(px(16.))
                .text_color(rgb(if primary { c::PAGE } else { c::SOFT })),
        )
        .child(label.into())
}
pub fn icon_button(id: impl Into<ElementId>, glyph: &'static str) -> Stateful<Div> {
    button(id, "", glyph, false)
        .w(px(l::BUTTON))
        .px(px(0.))
        .gap(px(0.))
        .justify_center()
        .flex_shrink_0()
}
pub fn section_label(text: impl Into<SharedString>) -> Div {
    mono(text).text_size(px(11.))
}
pub fn rule() -> Div {
    div().h(px(1.)).w_full().bg(rgb(c::RULE)).flex_shrink_0()
}

pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let name = path
            .strip_prefix("icons/")
            .and_then(|p| p.strip_suffix(".svg"));
        let shape = match name {
            Some("home") => "<path d='m3 10 9-7 9 7v10a1 1 0 0 1-1 1h-5v-7H9v7H4a1 1 0 0 1-1-1z'/>",
            Some("tools") => {
                "<path d='M14.7 6.3a5 5 0 0 0-6.4 6.4L3 18a2 2 0 0 0 3 3l5.3-5.3a5 5 0 0 0 6.4-6.4l-3 3-3-3z'/>"
            }
            Some("folder") => {
                "<path d='M3 7V5a1 1 0 0 1 1-1h5l2 3h9a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z'/>"
            }
            Some("edit") => "<path d='m16 3 5 5-12 12H4v-5zM13 6l5 5'/>",
            Some("play") => "<path d='m8 4 12 8-12 8z'/>",
            Some("plus") => "<path d='M12 5v14M5 12h14'/>",
            Some("check") => "<path d='m5 12 4 4L19 6'/>",
            Some("refresh") => {
                "<path d='M20 7v5h-5M4 17v-5h5'/><path d='M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1'/>"
            }
            Some("close") => "<path d='m6 6 12 12M6 18 18 6'/>",
            Some("code") => "<path d='m8 6-6 6 6 6m8-12 6 6-6 6m-3-15-2 18'/>",
            Some("box") => "<path d='m3 7 9-5 9 5v10l-9 5-9-5zM3 7l9 5 9-5M12 12v10'/>",
            Some("chevron") => "<path d='m9 5 7 7-7 7'/>",
            Some("terminal") => "<path d='m4 6 6 6-6 6m8 0h8'/>",
            Some("stop") => "<rect x='5' y='5' width='14' height='14' rx='1'/>",
            _ => return Ok(None),
        };
        Ok(Some(Cow::Owned(format!("<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='1.7' stroke-linecap='round' stroke-linejoin='round'>{shape}</svg>").into_bytes())))
    }
    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(vec![])
    }
}
