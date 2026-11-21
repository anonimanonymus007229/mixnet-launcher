use crate::{app::Launcher, dialogs::DialogKind, input::Input, ui::*};
use gpui::{prelude::*, *};
use mixnet_builder::repository::{self, Repository};
use mixnet_core::{Client, State};
use mixnet_theme::color as c;
use std::{
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

type Query = Receiver<Result<Repository, String>>;
pub struct SourcePicker {
    pub input: String,
    pub repository: Option<Repository>,
    pub error: Option<String>,
    pub loading: bool,
    pub open: bool,
    pub selected: Option<String>,
    pub filter: Entity<Input>,
    changed: Instant,
    query: Option<Query>,
    view_signature: (String, String),
}
impl SourcePicker {
    pub fn new(cx: &mut Context<Launcher>) -> Self {
        Self {
            input: "\0".into(),
            repository: None,
            error: None,
            loading: false,
            open: false,
            selected: None,
            filter: cx.new(|cx| Input::new("", "Search tags", cx)),
            changed: Instant::now(),
            query: None,
            view_signature: (String::new(), String::new()),
        }
    }
    fn poll(
        &mut self,
        input: String,
        revision: &Entity<Input>,
        state: &State,
        cx: &mut Context<Launcher>,
    ) -> bool {
        if self.input != input {
            self.input = input;
            self.changed = Instant::now();
            self.query = None;
            self.repository = None;
            self.error = None;
            self.loading = false;
            self.open = false;
            self.selected = None;
            revision.update(cx, |i, cx| {
                i.set(String::new());
                cx.notify();
            });
            return true;
        }
        if let Some(result) = self.query.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.query = None;
            self.loading = false;
            match result {
                Ok(repo) => {
                    if revision.read(cx).text.is_empty() {
                        revision.update(cx, |i, cx| {
                            i.set(repo.head.clone().unwrap_or_default());
                            cx.notify();
                        });
                    }
                    self.repository = Some(repo);
                }
                Err(e) => self.error = Some(e),
            }
            return true;
        }
        if !self.loading
            && self.repository.is_none()
            && self.error.is_none()
            && self.changed.elapsed() >= Duration::from_millis(500)
        {
            self.loading = true;
            let (tx, rx) = mpsc::channel();
            self.query = Some(rx);
            let input = self.input.clone();
            let state = state.clone();
            std::thread::spawn(move || {
                let _ = tx.send(repository::inspect(&input, &state).map_err(|e| format!("{e:#}")));
            });
            return true;
        }
        let signature = (
            self.filter.read(cx).text.clone(),
            revision.read(cx).text.clone(),
        );
        let changed = signature != self.view_signature;
        self.view_signature = signature;
        changed
    }
    pub fn retry(&mut self) {
        self.query = None;
        self.repository = None;
        self.error = None;
        self.loading = false;
        self.changed = Instant::now() - Duration::from_secs(1);
    }
}

type HomeResult = (String, String, String, Result<String, String>);
pub struct HomeCommit {
    attempted: Option<(String, String, String)>,
    query: Option<Receiver<HomeResult>>,
}
impl HomeCommit {
    pub fn new() -> Self {
        Self {
            attempted: None,
            query: None,
        }
    }
    pub fn reset(&mut self) {
        self.attempted = None;
    }
    pub fn poll(&mut self, state: &mut State, busy: bool) -> bool {
        if busy {
            return false;
        }
        if let Some((id, location, revision, result)) =
            self.query.as_ref().and_then(|rx| rx.try_recv().ok())
        {
            self.query = None;
            if let Ok(commit) = result
                && let Some(i) = state.instances.iter_mut().find(|i| i.id == id)
                && let Client::Source {
                    location: old_location,
                    revision: old_revision,
                } = &mut i.client
                && *old_location == location
                && *old_revision == revision
            {
                *old_revision = commit;
                return true;
            }
        }
        let i = state.current();
        if let Client::Source { location, revision } = &i.client
            && !repository::is_commit(revision)
            && !(revision.is_empty() && std::path::Path::new(location).is_dir())
        {
            let key = (i.id.clone(), location.clone(), revision.clone());
            if self.attempted.as_ref() != Some(&key) && self.query.is_none() {
                self.attempted = Some(key.clone());
                let (tx, rx) = mpsc::channel();
                self.query = Some(rx);
                let state = state.clone();
                std::thread::spawn(move || {
                    let (id, location, revision) = key;
                    let result = repository::inspect(&location, &state)
                        .and_then(|r| r.resolve(&revision))
                        .map_err(|e| format!("{e:#}"));
                    let _ = tx.send((id, location, revision, result));
                });
            }
        }
        false
    }
}
impl Launcher {
    pub fn poll_sources(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        if let Some(d) = &mut self.dialog
            && matches!(d.kind, DialogKind::Create)
            && !d.binary
        {
            let input = d.location.read(cx).text.trim().trim_matches('"').to_owned();
            changed |= d.source.poll(input, &d.revision, &self.state, cx);
        }
        if self.home_commit.poll(&mut self.state, self.busy) {
            self.save();
            changed = true;
        }
        changed
    }
    pub fn source_fields(&self, cx: &mut Context<Self>) -> AnyElement {
        let d = self.dialog.as_ref().unwrap();
        let picker = &d.source;
        let local = picker.repository.as_ref().is_some_and(|r| r.local);
        let current = d.revision.read(cx).text.trim();
        let selected = picker.selected.as_ref().filter(|name| {
            picker.repository.as_ref().is_some_and(|r| {
                r.tags
                    .iter()
                    .any(|t| &t.name == *name && t.commit == current)
            })
        });
        let label = if local {
            "Local checkout · current files".into()
        } else if picker.loading {
            "Loading repository tags…".into()
        } else if let Some(tag) = selected {
            format!("Tag · {tag}")
        } else {
            "Select a tag or use a custom commit".into()
        };
        let query = picker.filter.read(cx).text.trim().to_ascii_lowercase();
        column()
            .gap(px(12.))
            .flex_shrink_0()
            .child(
                column()
                    .gap(px(8.))
                    .child(section_label("REPOSITORY TAG"))
                    .child(
                        button("tag-dropdown", label, "chevron", false)
                            .w_full()
                            .justify_between()
                            .opacity(if picker.repository.is_some() { 1. } else { 0.6 })
                            .on_click(cx.listener(|a, _, _, cx| {
                                if let Some(d) = &mut a.dialog
                                    && d.source.repository.as_ref().is_some_and(|r| !r.local)
                                {
                                    d.source.open = !d.source.open;
                                    cx.notify();
                                }
                            })),
                    )
                    .when(picker.open && !local, |col| {
                        col.child(
                            column()
                                .gap(px(6.))
                                .p(px(8.))
                                .rounded(px(6.))
                                .bg(rgb(c::HOVER))
                                .child(picker.filter.clone())
                                .child(
                                    column()
                                        .id("tag-list")
                                        .max_h(px(160.))
                                        .overflow_y_scroll()
                                        // Keep wheel events here, even at either scroll limit.
                                        .on_scroll(|_, _, cx| cx.stop_propagation())
                                        .child(
                                            row()
                                                .id("custom-commit")
                                                .h(px(34.))
                                                .px(px(8.))
                                                .cursor_pointer()
                                                .hover(|s| s.bg(rgb(c::PLATE)))
                                                .child(muted("Custom commit"))
                                                .on_click(cx.listener(|a, _, window, cx| {
                                                    if let Some(d) = &mut a.dialog {
                                                        d.source.selected = None;
                                                        d.source.open = false;
                                                        d.revision
                                                            .read(cx)
                                                            .handle()
                                                            .focus(window, cx);
                                                    }
                                                    cx.notify();
                                                })),
                                        )
                                        .when_some(picker.repository.as_ref(), |col, repo| {
                                            col.children(
                                                repo.tags
                                                    .iter()
                                                    .filter(|t| {
                                                        t.name.to_ascii_lowercase().contains(&query)
                                                    })
                                                    .map(|tag| {
                                                        let name = tag.name.clone();
                                                        let commit = tag.commit.clone();
                                                        row()
                                                            .id(SharedString::from(format!(
                                                                "tag-{name}"
                                                            )))
                                                            .h(px(34.))
                                                            .flex_shrink_0()
                                                            .px(px(8.))
                                                            .gap(px(12.))
                                                            .cursor_pointer()
                                                            .hover(|s| s.bg(rgb(c::PLATE)))
                                                            .child(
                                                                div()
                                                                    .flex_1()
                                                                    .text_size(px(13.))
                                                                    .child(name.clone()),
                                                            )
                                                            .child(mono(commit[..8].to_owned()))
                                                            .on_click(cx.listener(
                                                                move |a, _, _, cx| {
                                                                    if let Some(d) = &mut a.dialog {
                                                                        d.revision.update(
                                                                            cx,
                                                                            |i, cx| {
                                                                                i.set(
                                                                                    commit.clone(),
                                                                                );
                                                                                cx.notify();
                                                                            },
                                                                        );
                                                                        d.source.selected =
                                                                            Some(name.clone());
                                                                        d.source.open = false;
                                                                        a.error = None;
                                                                    }
                                                                    cx.notify();
                                                                },
                                                            ))
                                                    }),
                                            )
                                        })
                                        .when(
                                            picker.repository.as_ref().is_some_and(|r| {
                                                r.tags.iter().all(|t| {
                                                    !t.name.to_ascii_lowercase().contains(&query)
                                                })
                                            }),
                                            |col| col.child(muted("No matching tags").p(px(8.))),
                                        ),
                                ),
                        )
                    }),
            )
            .when_some(picker.error.clone(), |col, error| {
                col.child(
                    column()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(c::DANGER))
                                .child(error),
                        )
                        .child(
                            button("retry-source", "Retry repository check", "refresh", false)
                                .on_click(cx.listener(|a, _, _, cx| {
                                    if let Some(d) = &mut a.dialog {
                                        d.source.retry();
                                    }
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                column()
                    .gap(px(8.))
                    .child(section_label(if local {
                        "CURRENT CHECKOUT COMMIT"
                    } else {
                        "COMMIT"
                    }))
                    .when(local, |col| {
                        col.child(mono(if current.is_empty() {
                            "Not a Git checkout".into()
                        } else {
                            current.to_owned()
                        }))
                    })
                    .when(!local, |col| col.child(d.revision.clone())),
            )
            .into_any_element()
    }
}
