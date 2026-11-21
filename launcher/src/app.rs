use crate::{
    dialogs::{Dialog, DialogKind},
    source_picker::HomeCommit,
    ui::*,
};
use gpui::{prelude::*, *};
use mixnet_builder::{self as builder, mods::ModInfo, tools::ToolStatus};
use mixnet_core::{State, Store};
use mixnet_theme::{color as c, layout as l};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

#[derive(Clone, Copy, PartialEq)]
pub enum Page {
    Home,
    Tools,
}
pub struct Launcher {
    pub store: Store,
    pub state: State,
    pub page: Page,
    pub mods: Vec<ModInfo>,
    pub tools: Vec<ToolStatus>,
    pub scanning: bool,
    pub dialog: Option<Dialog>,
    pub error: Option<String>,
    pub busy: bool,
    pub phase: String,
    pub logs: VecDeque<String>,
    pub show_logs: bool,
    pub log_scroll: ScrollHandle,
    events: Option<Receiver<builder::Event>>,
    cancel: Option<Arc<AtomicBool>>,
    tool_events: Option<Receiver<Vec<ToolStatus>>>,
    pub home_commit: HomeCommit,
}
impl Launcher {
    pub fn new(store: Store, state: State, cx: &mut Context<Self>) -> Self {
        let mut app = Self {
            store,
            state,
            page: Page::Home,
            mods: vec![],
            tools: vec![],
            scanning: false,
            dialog: None,
            error: None,
            busy: false,
            phase: "Ready when you are".into(),
            logs: VecDeque::new(),
            show_logs: false,
            log_scroll: ScrollHandle::new(),
            events: None,
            cancel: None,
            tool_events: None,
            home_commit: HomeCommit::new(),
        };
        app.reload_mods();
        app.scan_tools(cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this.update(cx, |app, cx| app.poll(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        app
    }
    pub fn save(&mut self) -> bool {
        match self.store.save(&self.state) {
            Ok(()) => true,
            Err(e) => {
                self.error = Some(format!("Could not save: {e:#}"));
                false
            }
        }
    }
    pub fn reload_mods(&mut self) {
        match builder::mods::discover(self.state.current(), &self.store) {
            Ok(mods) => self.mods = mods,
            Err(e) => {
                self.mods.clear();
                self.error = Some(format!("{e:#}"));
            }
        }
    }
    pub fn scan_tools(&mut self, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        self.home_commit.reset();
        let (tx, rx) = mpsc::channel();
        let state = self.state.clone();
        self.tool_events = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(builder::tools::scan(&state));
        });
        cx.notify();
    }
    fn poll(&mut self, cx: &mut Context<Self>) {
        let mut changed = self.poll_sources(cx);
        if let Some(result) = self.tool_events.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.tools = result;
            self.scanning = false;
            self.tool_events = None;
            changed = true;
        }
        let events: Vec<_> = self
            .events
            .as_ref()
            .map(|rx| rx.try_iter().take(500).collect())
            .unwrap_or_default();
        for e in events {
            changed = true;
            match e {
                builder::Event::Log(line) => {
                    if self.logs.len() >= 1500 {
                        self.logs.pop_front();
                    }
                    self.logs.push_back(line);
                    self.log_scroll.scroll_to_bottom();
                }
                builder::Event::Phase(phase) => self.phase = phase,
                builder::Event::Finished(result) => {
                    self.busy = false;
                    self.events = None;
                    self.cancel = None;
                    match result {
                        Ok(()) => self.phase = "Session finished".into(),
                        Err(e) => {
                            self.phase = "Stopped".into();
                            self.error = Some(e);
                            self.show_logs = true;
                        }
                    }
                }
            }
        }
        if changed {
            cx.notify();
        }
    }
    pub fn build_play(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.error = None;
        self.logs.clear();
        self.show_logs = true;
        self.busy = true;
        self.phase = "Starting…".into();
        let (rx, cancel) = builder::start(self.store.clone(), self.state.clone());
        self.events = Some(rx);
        self.cancel = Some(cancel);
        cx.notify();
    }
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
            self.phase = "Stopping…".into();
        }
        cx.notify();
    }
    pub fn open_dialog(&mut self, kind: DialogKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let dialog = Dialog::new(kind, &self.state, cx);
        let field = if matches!(dialog.kind, DialogKind::Tool(_)) {
            &dialog.location
        } else {
            &dialog.name
        };
        field.read(cx).handle().focus(window, cx);
        self.dialog = Some(dialog);
        self.error = None;
        cx.notify();
    }
    pub fn open_mods_folder(&mut self, cx: &mut Context<Self>) {
        let path = self.store.mods();
        if let Err(error) = std::fs::create_dir_all(&path) {
            self.error = Some(format!("Cannot create mods folder: {error}"));
            cx.notify();
            return;
        }
        self.open_folder(path, cx);
    }
    pub fn pick_field(&mut self, files: bool, cx: &mut Context<Self>) {
        let Some(dialog) = &self.dialog else {
            return;
        };
        let input = dialog.location.clone();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files,
            directories: !files,
            multiple: false,
            prompt: Some(
                if files {
                    "Choose executable"
                } else {
                    "Choose client source folder"
                }
                .into(),
            ),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(path) = paths.first()
            {
                let text = path.to_string_lossy().to_string();
                let _ = this.update(cx, |_, cx| {
                    input.update(cx, |field, cx| {
                        field.set(text);
                        cx.notify();
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }
    pub fn open_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let result = builder::tools::command("explorer.exe").arg(path).spawn();
        if let Err(e) = result {
            self.error = Some(e.to_string());
        }
        cx.notify();
    }
    fn navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        column()
            .w(px(l::SIDEBAR))
            .h_full()
            .flex_shrink_0()
            .px(px(20.))
            .py(px(28.))
            .gap(px(8.))
            .border_r_1()
            .border_color(rgb(c::RULE))
            .child(
                row()
                    .px(px(12.))
                    .gap(px(9.))
                    .mb(px(42.))
                    .child(icon("code"))
                    .child(
                        div()
                            .text_size(px(19.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("mixnet"),
                    ),
            )
            .children(
                [
                    (Page::Home, "Home", "home"),
                    (Page::Tools, "Tools", "tools"),
                ]
                .into_iter()
                .map(|(page, label, glyph)| {
                    row()
                        .id(label)
                        .h(px(42.))
                        .px(px(12.))
                        .gap(px(12.))
                        .rounded(px(l::RADIUS))
                        .cursor_pointer()
                        .bg(rgb(if self.page == page { c::PLATE } else { c::PAGE }))
                        .hover(|s| s.bg(rgb(c::HOVER)))
                        .text_color(rgb(if self.page == page { c::INK } else { c::MUTED }))
                        .child(icon(glyph))
                        .child(label)
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.page = page;
                            cx.notify();
                        }))
                }),
            )
            .child(div().flex_1())
            .child(mono("NATIVE MIXIN LAUNCHER").text_size(px(9.)).px(px(12.)))
            .child(muted("0.1.0 · Windows").px(px(12.)))
    }
    fn console(&self, cx: &mut Context<Self>) -> impl IntoElement {
        column()
            .border_t_1()
            .border_color(rgb(c::RULE))
            .bg(rgb(c::HOVER))
            .flex_shrink_0()
            .child(
                row()
                    .px(px(l::GUTTER))
                    .h(px(43.))
                    .gap(px(10.))
                    .child(icon("terminal"))
                    .child(div().flex_1().text_size(px(12.)).child(self.phase.clone()))
                    .when(self.busy, |r| {
                        r.child(
                            button("stop", "Stop", "stop", false)
                                .on_click(cx.listener(|a, _, _, cx| a.cancel(cx))),
                        )
                    })
                    .child(
                        button("log-folder", "Logs", "folder", false)
                            .h(px(28.))
                            .on_click(cx.listener(|a, _, _, cx| {
                                a.open_folder(a.store.instance(&a.state.selected).join("logs"), cx)
                            })),
                    )
                    .child(
                        button(
                            "toggle-console",
                            if self.show_logs { "Hide" } else { "Console" },
                            "terminal",
                            false,
                        )
                        .h(px(28.))
                        .on_click(cx.listener(|a, _, _, cx| {
                            a.show_logs = !a.show_logs;
                            cx.notify();
                        })),
                    ),
            )
            .when(self.show_logs, |d| {
                d.child(
                    column()
                        .id("logs")
                        .h(px(190.))
                        .overflow_y_scroll()
                        .track_scroll(&self.log_scroll)
                        .px(px(l::GUTTER))
                        .pb(px(16.))
                        .gap(px(3.))
                        .children(
                            self.logs
                                .iter()
                                .rev()
                                .take(200)
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .map(|line| {
                                    mono(line.clone())
                                        .text_size(px(11.))
                                        .text_color(rgb(c::SOFT))
                                }),
                        ),
                )
            })
    }
}
impl Drop for Launcher {
    fn drop(&mut self) {
        if let Some(stop) = &self.cancel {
            stop.store(true, Ordering::Relaxed);
        }
    }
}
impl Render for Launcher {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .font_family(mixnet_theme::FONT)
            .text_size(px(14.))
            .text_color(rgb(c::INK))
            .bg(rgb(c::PAGE))
            .child(
                row().size_full().child(self.navigation(cx)).child(
                    column()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(
                            column()
                                .id("page")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .px(px(l::GUTTER))
                                .pt(px(38.))
                                .pb(px(28.))
                                .gap(px(28.))
                                .child(match self.page {
                                    Page::Home => self.home(cx).into_any_element(),
                                    Page::Tools => self.tools_page(cx).into_any_element(),
                                })
                                .when_some(self.error.clone(), |d, error| {
                                    d.child(
                                        row()
                                            .gap(px(12.))
                                            .p(px(14.))
                                            .rounded(px(6.))
                                            .bg(rgb(c::SURFACE))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .text_size(px(12.))
                                                    .text_color(rgb(c::DANGER))
                                                    .child(error),
                                            )
                                            .child(
                                                button("dismiss", "Dismiss", "close", false)
                                                    .on_click(cx.listener(|a, _, _, cx| {
                                                        a.error = None;
                                                        cx.notify();
                                                    })),
                                            ),
                                    )
                                }),
                        )
                        .child(self.console(cx)),
                ),
            )
            .when(self.dialog.is_some(), |d| d.child(self.dialog_view(cx)))
    }
}
