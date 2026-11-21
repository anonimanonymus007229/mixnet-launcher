use crate::{app::Launcher, dialogs::DialogKind, ui::*};
use gpui::{prelude::*, *};
use mixnet_core::Client;
use mixnet_theme::{color as c, layout as l};

impl Launcher {
    pub fn home(&self, cx: &mut Context<Self>) -> AnyElement {
        let i = self.state.current();
        let source = i.is_source();
        let description = match &i.client {
            Client::Source { location, revision } => format!(
                "{} · {}",
                location,
                if mixnet_builder::repository::is_commit(revision) {
                    &revision[..8]
                } else if revision.is_empty() {
                    "local checkout"
                } else {
                    "commit not resolved yet"
                }
            ),
            Client::Binary { executable } => executable.display().to_string(),
        };
        column().gap(px(30.))
            .child(row().justify_between().items_end()
                .child(column().gap(px(7.)).child(section_label("YOUR WORKSPACE")).child(heading("Home")))
                .child(button("new-instance", "New instance", "plus", false)
                    .on_click(cx.listener(|a, _, window, cx| a.open_dialog(DialogKind::Create, window, cx)))))
            .when(self.state.instances.len() > 1, |d| d.child(row().flex_wrap().gap(px(8.)).children(self.state.instances.iter().map(|instance| {
                let id = instance.id.clone();
                button(SharedString::from(format!("switch-{id}")), instance.name.clone(), if instance.is_source() { "code" } else { "box" }, false)
                    .bg(rgb(if instance.id == self.state.selected { c::PLATE } else { c::PAGE }))
                    .on_click(cx.listener(move |a, _, _, cx| {
                        if !a.busy { a.state.selected = id.clone(); a.save(); a.reload_mods(); cx.notify(); }
                    }))
            }))))
            .child(column().gap(px(18.)).pt(px(16.)).pb(px(26.)).border_b_1().border_color(rgb(c::RULE))
                .child(row().gap(px(10.)).child(mono("01"))
                    .child(section_label(if source { "SOURCE INSTANCE" } else { "EXTERNAL CLIENT" })))
                .child(row().gap(px(10.))
                    .child(div().flex_1().min_w_0().text_size(px(27.)).font_weight(FontWeight::SEMIBOLD).text_ellipsis().child(i.name.clone()))
                    .child(button("play", if self.busy { "Working…" } else if source { "Build and play" } else { "Play" }, "play", true)
                        .opacity(if self.busy { 0.6 } else { 1. })
                        .on_click(cx.listener(|a, _, _, cx| a.build_play(cx))))
                    .child(icon_button("instance-folder", "folder")
                        .on_click(cx.listener(|a, _, _, cx| a.open_folder(a.store.instance(&a.state.selected), cx))))
                    .child(icon_button("rename", "edit")
                        .on_click(cx.listener(|a, _, window, cx| a.open_dialog(DialogKind::Rename, window, cx)))))
                .child(mono(description).text_size(px(11.))))
            .child(column().gap(px(16.))
                .child(row().justify_between()
                    .child(row().gap(px(10.)).child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child("Mixin mods"))
                        .child(mono(format!("{:02} ENABLED", self.mods.iter().filter(|m| m.enabled).count()))))
                    .when(source, |d| d.child(row().gap(px(8.))
                        .child(button("rescan-mods", "Refresh", "refresh", false).on_click(cx.listener(|a, _, _, cx| { if !a.busy { a.reload_mods(); cx.notify(); } })))
                        .child(button("mods-folder", "Open folder", "folder", false).on_click(cx.listener(|a, _, _, cx| a.open_mods_folder(cx)))))))
                .when(source, |d| d.child(mono(self.store.mods().display().to_string()).text_size(px(10.))))
                .when(!source, |d| d.child(column().py(px(30.)).gap(px(10.)).child(muted("This client runs as-is. Mixin mods require a source instance."))
                    .child(muted("Its executable and game assets stay in their original folder."))))
                .when(source && self.mods.is_empty(), |d| d.child(column().py(px(32.)).gap(px(12.)).items_center()
                    .child(icon("box").size(px(26.)))
                    .child(div().text_color(rgb(c::SOFT)).child("A clean client. Your mods go here."))
                    .child(muted("Add mixin.toml packages to the mods folder, then click Refresh."))))
                .when(source, |d| d.children(self.mods.iter().enumerate().map(|(index, m)| {
                    let id = m.id.clone(); let enabled = m.enabled;
                    row().id(SharedString::from(format!("mod-{id}"))).h(px(l::ROW)).gap(px(18.)).border_b_1().border_color(rgb(c::RULE))
                        .child(mono(format!("{:02}", index + 1)).w(px(24.)))
                        .child(column().flex_1().gap(px(5.)).child(div().font_weight(FontWeight::SEMIBOLD).child(m.name.clone()))
                            .child(mono(m.id.clone()).text_size(px(10.))))
                        .child(mono(m.version.clone()))
                        .child(row().w(px(36.)).h(px(22.)).px(px(3.)).rounded(px(11.)).cursor_pointer()
                            .bg(rgb(if enabled { c::ACCENT } else { c::PLATE }))
                            .when(enabled, |r| r.justify_end())
                            .child(div().size(px(16.)).rounded_full().bg(rgb(if enabled { c::PAGE } else { c::MUTED }))))
                        .on_click(cx.listener(move |a, _, _, cx| {
                            if !a.busy {
                                let disabled = &mut a.state.current_mut().disabled_mods;
                                if enabled { if !disabled.contains(&id) { disabled.push(id.clone()); } } else { disabled.retain(|m| m != &id); }
                                a.save(); a.reload_mods(); a.phase = "Mods changed · build required".into(); cx.notify();
                            }
                        }))
                }))))

            .into_any_element()
    }
}
