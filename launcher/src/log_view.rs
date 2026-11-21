//! Read-only console text: cross-line mouse selection, clipboard and bounded history.
use gpui::{prelude::*, *};
use mixnet_theme::{color as c, layout as l};
use std::{cell::Cell, collections::VecDeque, ops::Range, rc::Rc};

const LINE_HEIGHT: f32 = 18.;
const HISTORY: usize = 1500;

#[derive(Default)]
struct LogText {
    lines: VecDeque<String>,
    text: String,
    anchor: usize,
    caret: usize,
}
impl LogText {
    fn range(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }
    fn selected(&self) -> &str {
        &self.text[self.range()]
    }
    fn append(&mut self, line: String) {
        for line in line.replace('\r', "").split('\n') {
            if !self.lines.is_empty() {
                self.text.push('\n');
            }
            self.text.push_str(line);
            self.lines.push_back(line.into());
            if self.lines.len() > HISTORY {
                let removed = self.lines.pop_front().unwrap().len() + 1;
                self.text.drain(..removed);
                self.anchor = self.anchor.saturating_sub(removed);
                self.caret = self.caret.saturating_sub(removed);
            }
        }
    }
}
struct VisualLine {
    start: usize,
    shaped: ShapedLine,
}

pub struct LogView {
    text: LogText,
    focus: FocusHandle,
    scroll: ScrollHandle,
    shaped: Option<Rc<Vec<VisualLine>>>,
    area: Rc<Cell<Bounds<Pixels>>>,
    dragging: bool,
}
impl LogView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            text: LogText::default(),
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            shaped: None,
            area: Rc::new(Cell::new(Bounds::default())),
            dragging: false,
        }
    }
    pub fn append(&mut self, line: String, cx: &mut Context<Self>) {
        let at_bottom = (self.scroll.max_offset().y + self.scroll.offset().y).abs() <= px(2.);
        let follow = at_bottom && !self.dragging && self.text.range().is_empty();
        self.text.append(line);
        self.shaped = None;
        if follow {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.text = LogText::default();
        self.shaped = None;
        self.dragging = false;
        self.scroll.set_offset(Point::default());
        cx.notify();
    }
    pub fn copy_text(&self) -> String {
        if self.text.range().is_empty() {
            self.text.text.clone()
        } else {
            self.text.selected().into()
        }
    }
    fn index_at(&self, position: Point<Pixels>) -> usize {
        let Some(lines) = &self.shaped else {
            return 0;
        };
        let origin = self.area.get().origin;
        let row = (((position.y - origin.y) / px(LINE_HEIGHT)).floor().max(0.) as usize)
            .min(lines.len() - 1);
        let line = &lines[row];
        let index = line
            .shaped
            .closest_index_for_x(position.x - origin.x)
            .min(line.shaped.text.len());
        // Shaping normally returns scalar boundaries; guard against malformed glyph clusters.
        let mut index = index;
        while !line.shaped.text.is_char_boundary(index) {
            index -= 1;
        }
        line.start + index
    }
    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let index = self.index_at(e.position);
        if !e.modifiers.shift {
            self.text.anchor = index;
        }
        self.text.caret = index;
        self.dragging = true;
        if e.click_count >= 3 {
            let start = self.text.text[..index].rfind('\n').map_or(0, |i| i + 1);
            let end = self.text.text[index..]
                .find('\n')
                .map_or(self.text.text.len(), |i| index + i);
            self.text.anchor = start;
            self.text.caret = end;
        } else if e.click_count == 2 {
            self.text.anchor = self.text.text[..index]
                .char_indices()
                .rev()
                .find(|(_, ch)| ch.is_whitespace())
                .map_or(0, |(i, ch)| i + ch.len_utf8());
            self.text.caret = self.text.text[index..]
                .char_indices()
                .find(|(_, ch)| ch.is_whitespace())
                .map_or(self.text.text.len(), |(i, _)| index + i);
        }
        cx.stop_propagation();
        cx.notify();
    }
    fn key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ctrl = e.keystroke.modifiers.control || e.keystroke.modifiers.platform;
        match (ctrl, e.keystroke.key.as_str()) {
            (true, "c" | "insert") => {
                if !self.text.range().is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(self.text.selected().into()));
                }
            }
            (true, "a") => {
                self.text.anchor = 0;
                self.text.caret = self.text.text.len();
            }
            (_, "escape") => {
                self.text.anchor = self.text.caret;
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
    fn shape(&mut self, window: &Window) -> Rc<Vec<VisualLine>> {
        self.shaped
            .get_or_insert_with(|| {
                let mut start = 0;
                Rc::new(
                    self.text
                        .text
                        .split('\n')
                        .map(|text| {
                            let shaped = window.text_system().shape_line(
                                text.to_owned().into(),
                                px(11.),
                                &[TextRun {
                                    len: text.len(),
                                    font: font(mixnet_theme::MONO),
                                    color: rgb(c::SOFT).into(),
                                    background_color: None,
                                    underline: None,
                                    strikethrough: None,
                                }],
                                None,
                            );
                            let line = VisualLine { start, shaped };
                            start += text.len() + 1;
                            line
                        })
                        .collect(),
                )
            })
            .clone()
    }
}
impl Render for LogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = self.shape(window);
        let width = lines
            .iter()
            .map(|line| line.shaped.width)
            .fold(px(1.), |a, b| a.max(b))
            + px(8.);
        let height = px(lines.len() as f32 * LINE_HEIGHT);
        let selection = self.text.range();
        let area = self.area.clone();
        let view = cx.entity();
        div()
            .id("log-viewport")
            .h(px(190.))
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .px(px(l::GUTTER))
            .pb(px(16.))
            .cursor_text()
            .track_focus(&self.focus)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|v, _, _, _| v.dragging = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|v, _, _, _| v.dragging = false),
            )
            .on_key_down(cx.listener(Self::key))
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .child(
                canvas(
                    |bounds, _, _| bounds,
                    move |bounds, _, window, cx| {
                        area.set(bounds);
                        let clip = window.content_mask().bounds;
                        let first = (((clip.top() - bounds.top()) / px(LINE_HEIGHT))
                            .floor()
                            .max(0.) as usize)
                            .min(lines.len());
                        let end = ((((clip.bottom() - bounds.top()) / px(LINE_HEIGHT))
                            .ceil()
                            .max(0.) as usize)
                            + 1)
                        .min(lines.len());
                        for (row, line) in lines.iter().enumerate().take(end).skip(first) {
                            let y = bounds.top() + px(row as f32 * LINE_HEIGHT);
                            let start = selection
                                .start
                                .saturating_sub(line.start)
                                .min(line.shaped.text.len());
                            let end = selection
                                .end
                                .saturating_sub(line.start)
                                .min(line.shaped.text.len());
                            let newline = selection.start <= line.start + line.shaped.text.len()
                                && selection.end > line.start + line.shaped.text.len();
                            if start < end || newline {
                                let left = line.shaped.x_for_index(start);
                                let right = line.shaped.x_for_index(end)
                                    + if newline { px(6.) } else { px(0.) };
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(bounds.left() + left, y),
                                        size(right - left, px(LINE_HEIGHT)),
                                    ),
                                    rgba(0x9a86ff60),
                                ));
                            }
                            let _ = line.shaped.paint(
                                point(bounds.left(), y),
                                px(LINE_HEIGHT),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                        // Keep extending a drag outside the text hitbox, not just over individual rows.
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble && event.dragging() {
                                view.update(cx, |v, cx| {
                                    if v.dragging {
                                        let viewport = v.scroll.bounds();
                                        let mut offset = v.scroll.offset();
                                        if event.position.y < viewport.top() {
                                            offset.y += px(LINE_HEIGHT);
                                        }
                                        if event.position.y > viewport.bottom() {
                                            offset.y -= px(LINE_HEIGHT);
                                        }
                                        v.scroll.set_offset(offset);
                                        v.text.caret = v.index_at(event.position);
                                        cx.notify();
                                        window.refresh();
                                    }
                                });
                            }
                        });
                    },
                )
                .w(width)
                .min_w(relative(1.))
                .h(height),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{HISTORY, LogText};
    use core::prelude::v1::test;
    #[test]
    fn copies_multiline_unicode_in_both_directions() {
        let mut text = LogText::default();
        text.append("first Привет".into());
        text.append("second 🦀 line".into());
        text.anchor = "first ".len();
        text.caret = text.text.find(" line").unwrap();
        assert_eq!(text.selected(), "Привет\nsecond 🦀");
        std::mem::swap(&mut text.anchor, &mut text.caret);
        assert_eq!(text.selected(), "Привет\nsecond 🦀");
        text.append("new output".into());
        assert_eq!(text.selected(), "Привет\nsecond 🦀");
    }
    #[test]
    fn selection_survives_history_trimming() {
        let mut text = LogText::default();
        text.append("old".into());
        text.append("keep 🦀".into());
        text.anchor = "old\n".len();
        text.caret = text.text.len();
        for _ in 0..HISTORY - 1 {
            text.append("new".into());
        }
        assert_eq!(text.lines.len(), HISTORY);
        assert_eq!(text.selected(), "keep 🦀");
        text.append("trim selected line".into());
        assert!(text.selected().is_empty());
    }
    #[test]
    fn preserves_empty_lines() {
        let mut text = LogText::default();
        text.append(String::new());
        text.append("line".into());
        text.append(String::new());
        assert_eq!(text.text, "\nline\n");
    }
}
