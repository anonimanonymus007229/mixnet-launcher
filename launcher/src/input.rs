//! Compact single-line editor with native text/IME input, UTF-16 ranges and clipboard.
use gpui::{prelude::*, *};
use mixnet_theme::color as c;
use std::ops::Range;

pub struct Input {
    pub text: String,
    focus: FocusHandle,
    selection: Range<usize>, // UTF-8 bytes; platform ranges are UTF-16.
    marked: Option<Range<usize>>,
    placeholder: String,
}
impl Input {
    pub fn new(text: impl Into<String>, placeholder: &str, cx: &mut Context<Self>) -> Self {
        let text = text.into();
        let end = text.len();
        Self {
            text,
            focus: cx.focus_handle(),
            selection: end..end,
            marked: None,
            placeholder: placeholder.into(),
        }
    }
    pub fn handle(&self) -> FocusHandle {
        self.focus.clone()
    }
    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }
    pub fn set(&mut self, text: String) {
        let end = text.len();
        self.text = text;
        self.selection = end..end;
        self.marked = None;
    }
    fn byte(&self, utf16: usize) -> usize {
        let mut count = 0;
        for (i, ch) in self.text.char_indices() {
            if count >= utf16 {
                return i;
            }
            count += ch.len_utf16();
        }
        self.text.len()
    }
    fn utf16(&self, byte: usize) -> usize {
        self.text[..byte].encode_utf16().count()
    }
    fn range(&self, range: Range<usize>) -> Range<usize> {
        self.byte(range.start)..self.byte(range.end)
    }
    fn replace(&mut self, range: Range<usize>, text: &str) {
        let text = text.replace(['\r', '\n', '\0'], "");
        self.text.replace_range(range.clone(), &text);
        let end = range.start + text.len();
        self.selection = end..end;
    }
    fn key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = e.keystroke.key.as_str();
        let ctrl = e.keystroke.modifiers.control || e.keystroke.modifiers.platform;
        match (ctrl, key) {
            (true, "a") => self.selection = 0..self.text.len(),
            (true, "c" | "x") => {
                if !self.selection.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.text[self.selection.clone()].into(),
                    ));
                    if key == "x" {
                        self.replace(self.selection.clone(), "");
                    }
                }
            }
            (true, "v") => {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    self.replace(self.selection.clone(), &text);
                }
            }
            (_, "backspace") => {
                let range = if self.selection.is_empty() {
                    let end = self.selection.end;
                    let start = self.text[..end].char_indices().last().map_or(0, |(i, _)| i);
                    start..end
                } else {
                    self.selection.clone()
                };
                self.replace(range, "");
            }
            (_, "delete") => {
                let start = self.selection.start;
                let end = if self.selection.is_empty() {
                    start + self.text[start..].chars().next().map_or(0, char::len_utf8)
                } else {
                    self.selection.end
                };
                self.replace(start..end, "");
            }
            (_, "left") => {
                let pos = if !self.selection.is_empty() {
                    self.selection.start
                } else {
                    self.text[..self.selection.start]
                        .char_indices()
                        .last()
                        .map_or(0, |(i, _)| i)
                };
                self.selection = pos..pos;
            }
            (_, "right") => {
                let end = self.selection.end;
                let pos = if !self.selection.is_empty() {
                    end
                } else {
                    end + self.text[end..].chars().next().map_or(0, char::len_utf8)
                };
                self.selection = pos..pos;
            }
            (_, "home") => self.selection = 0..0,
            (_, "end") => {
                let end = self.text.len();
                self.selection = end..end;
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}
impl Render for Input {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let before = self.text[..self.selection.start].to_owned();
        let selected = self.text[self.selection.clone()].to_owned();
        let after = self.text[self.selection.end..].to_owned();
        let view = cx.entity();
        let focus = self.focus.clone();
        div()
            .id("input")
            .track_focus(&self.focus)
            .relative()
            .w_full()
            .h(px(40.))
            .px(px(12.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .bg(rgb(c::HOVER))
            .border_1()
            .border_color(rgb(if focused { c::ACCENT } else { c::RULE }))
            .text_size(px(13.))
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                focus.focus(window, cx)
            })
            .on_key_down(cx.listener(Self::key))
            .child(
                div()
                    .flex()
                    .items_center()
                    .whitespace_nowrap()
                    .when(self.text.is_empty() && !focused, |d| {
                        d.child(
                            div()
                                .text_color(rgb(c::MUTED))
                                .child(self.placeholder.clone()),
                        )
                    })
                    .child(before)
                    .when(!selected.is_empty(), |d| {
                        d.child(
                            div()
                                .bg(rgb(c::ACCENT))
                                .text_color(rgb(c::PAGE))
                                .child(selected),
                        )
                    })
                    .when(focused, |d| {
                        d.child(div().w(px(1.)).h(px(16.)).bg(rgb(c::ACCENT)))
                    })
                    .child(after),
            )
            .child(
                canvas(
                    |bounds, _, _| bounds,
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus_for(&view, cx),
                            ElementInputHandler::new(bounds, view),
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}
fn focus_for(view: &Entity<Input>, cx: &App) -> FocusHandle {
    view.read(cx).focus.clone()
}
impl EntityInputHandler for Input {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let r = self.range(range);
        *adjusted = Some(self.utf16(r.start)..self.utf16(r.end));
        Some(self.text[r].into())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.utf16(self.selection.start)..self.utf16(self.selection.end),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|r| self.utf16(r.start)..self.utf16(r.end))
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.range(r))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        self.replace(range, text);
        self.marked = None;
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.range(r))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        let start = range.start;
        self.replace(range, text);
        let end = self.selection.end;
        self.marked = Some(start..end);
        if let Some(r) = selected {
            let base = self.utf16(start);
            self.selection = self.range(base + r.start..base + r.end);
        }
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(bounds)
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.utf16(self.selection.end))
    }
    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selection = self.range(range);
        cx.notify();
    }
    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.text.encode_utf16().count())
    }
}
