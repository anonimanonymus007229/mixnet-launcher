use crate::{app::Launcher, input::Input, source_picker::SourcePicker, ui::*};
use gpui::{prelude::*, *};
use mixnet_core::{Client, Instance, State};
use mixnet_theme::color as c;
use std::path::PathBuf;

#[derive(Clone)]
pub enum DialogKind {
    Create,
    Rename,
    Tool(String),
}
pub struct Dialog {
    pub kind: DialogKind,
    pub binary: bool,
    pub name: Entity<Input>,
    pub location: Entity<Input>,
    pub revision: Entity<Input>,
    pub source: SourcePicker,
}
impl Dialog {
    pub fn new(kind: DialogKind, state: &State, cx: &mut Context<Launcher>) -> Self {
        let name = match &kind {
            DialogKind::Rename => state.current().name.clone(),
            _ => "DDNet".into(),
        };
        let location = match &kind {
            DialogKind::Tool(id) => state
                .tools
                .get(id)
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            _ => "https://github.com/ddnet/ddnet.git".into(),
        };
        Self {
            kind,
            binary: false,
            name: cx.new(|cx| Input::new(name, "Instance name", cx)),
            location: cx.new(|cx| Input::new(location, "Path to executable", cx)),
            revision: cx
                .new(|cx| Input::new("", "Full commit hash (filled when you select a tag)", cx)),
            source: SourcePicker::new(cx),
        }
    }
}
impl Launcher {
    pub fn commit_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(d) = &self.dialog else {
            return;
        };
        let kind = d.kind.clone();
        let binary = d.binary;
        let name = d.name.read(cx).text.trim().to_string();
        let location = d
            .location
            .read(cx)
            .text
            .trim()
            .trim_matches('"')
            .to_string();
        let revision = d.revision.read(cx).text.trim().to_string();
        let result = (|| -> anyhow::Result<()> {
            match kind {
                DialogKind::Rename => {
                    anyhow::ensure!(
                        !name.is_empty() && name.chars().count() <= 80,
                        "Use an instance name between 1 and 80 characters."
                    );
                    self.state.current_mut().name = name;
                }
                DialogKind::Tool(id) => {
                    if location.is_empty() {
                        self.state.tools.remove(&id);
                    } else {
                        let path = PathBuf::from(&location)
                            .canonicalize()
                            .map_err(|e| anyhow::anyhow!("Cannot open executable: {e}"))?;
                        anyhow::ensure!(
                            path.is_file(),
                            "Choose an executable file, not a directory."
                        );
                        self.state.tools.insert(id, path);
                    }
                }
                DialogKind::Create => {
                    anyhow::ensure!(
                        !name.is_empty() && name.chars().count() <= 80,
                        "Use an instance name between 1 and 80 characters."
                    );
                    let client = if binary {
                        let executable = PathBuf::from(&location)
                            .canonicalize()
                            .map_err(|e| anyhow::anyhow!("Cannot open executable: {e}"))?;
                        anyhow::ensure!(executable.is_file(), "Choose a client executable.");
                        Client::Binary { executable }
                    } else {
                        let picker = &d.source;
                        anyhow::ensure!(
                            picker.input == location,
                            "Wait for the source validation to finish."
                        );
                        if let Some(error) = &picker.error {
                            anyhow::bail!("{error}");
                        }
                        let repo = picker.repository.as_ref().ok_or_else(|| {
                            anyhow::anyhow!(
                                "Wait for the repository check and tag list to finish loading."
                            )
                        })?;
                        // Recheck local files at submission: the folder may have changed since the probe.
                        let (location, _) =
                            mixnet_builder::repository::validate_location(&repo.location)?;
                        let revision = repo.resolve(&revision)?;
                        Client::Source { location, revision }
                    };
                    let instance = Instance::new(name, client);
                    self.store.prepare(&instance)?;
                    self.state.selected = instance.id.clone();
                    self.state.instances.push(instance);
                }
            }
            Ok(())
        })();
        match result {
            Err(e) => self.error = Some(format!("{e:#}")),
            Ok(()) => {
                if self.save() {
                    self.dialog = None;
                    self.error = None;
                    self.reload_mods();
                    self.scanning = false;
                    self.scan_tools(cx);
                }
            }
        }
        cx.notify();
    }
    pub fn dialog_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let d = self.dialog.as_ref().unwrap();
        let create = matches!(d.kind, DialogKind::Create);
        let rename = matches!(d.kind, DialogKind::Rename);
        let tool = matches!(d.kind, DialogKind::Tool(_));
        let title = match &d.kind {
            DialogKind::Create => "New instance".to_string(),
            DialogKind::Rename => "Rename instance".to_string(),
            DialogKind::Tool(id) => format!("{} executable", id),
        };
        let binary = d.binary;
        let focus_name = d.name.clone();
        let focus_location = d.location.clone();
        let focus_revision = d.revision.clone();
        div().absolute().inset_0().bg(rgba(0x000000b8)).flex().items_center().justify_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(move |a, e: &KeyDownEvent, window, cx| {
                match e.keystroke.key.as_str() {
                    "escape" => { a.dialog = None; a.error = None; cx.notify(); }
                    "enter" => a.commit_dialog(cx),
                    "tab" => {
                        // Cycle the actual form fields, including reverse traversal.
                        let mut fields = vec![];
                        if !tool { fields.push(focus_name.clone()); }
                        if !rename { fields.push(focus_location.clone()); }
                        if create && !binary {
                            if let Some(d) = &a.dialog && d.source.open { fields.push(d.source.filter.clone()); }
                            if !a.dialog.as_ref().and_then(|d| d.source.repository.as_ref()).is_some_and(|r| r.local) { fields.push(focus_revision.clone()); }
                        }
                        let index = fields.iter().position(|f| f.read(cx).is_focused(window)).unwrap_or(0);
                        let next = if e.keystroke.modifiers.shift { (index + fields.len() - 1) % fields.len() } else { (index + 1) % fields.len() };
                        fields[next].read(cx).handle().focus(window, cx);
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(column().w(px(570.)).max_h(relative(0.94)).id("dialog-scroll").overflow_y_scroll().p(px(30.)).gap(px(22.)).bg(rgb(c::SURFACE)).border_1().border_color(rgb(c::RULE)).rounded(px(12.))
                .child(row().justify_between().child(div().text_size(px(24.)).font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(button("close-dialog", "Close", "close", false).on_click(cx.listener(|a, _, _, cx| { a.dialog = None; a.error = None; cx.notify(); }))))
                .when(create, |col| col.child(row().gap(px(10.))
                    .child(button("source-kind", "Open source", "code", !binary).flex_1()
                        .on_click(cx.listener(|a, _, _, cx| {
                            if let Some(d) = &mut a.dialog { d.binary = false; d.location.update(cx, |i, cx| { i.set("https://github.com/ddnet/ddnet.git".into()); cx.notify(); }); } cx.notify();
                        })))
                    .child(button("binary-kind", "Prebuilt / closed source", "box", binary).flex_1()
                        .on_click(cx.listener(|a, _, _, cx| {
                            if let Some(d) = &mut a.dialog { d.binary = true; d.location.update(cx, |i, cx| { i.set(String::new()); cx.notify(); }); } cx.notify();
                        })))))
                .when(!tool, |col| col.child(column().gap(px(8.)).child(section_label("NAME")).child(d.name.clone())))
                .when(!rename, |col| col.child(column().gap(px(8.))
                    .child(section_label(if tool { "EXECUTABLE PATH · EMPTY = AUTOMATIC" } else if binary { "CLIENT EXECUTABLE" } else { "GIT URL OR LOCAL SOURCE FOLDER" }))
                    .child(row().gap(px(8.)).child(div().flex_1().min_w_0().child(d.location.clone()))
                        .child(button("browse-field", "Browse", "folder", false)
                            .on_click(cx.listener(move |a, _, _, cx| a.pick_field(tool || binary, cx)))))))
                .when(create && !binary, |col| col.child(self.source_fields(cx)))
                .when(create, |col| col.child(muted(if binary {
                    "The original game folder stays intact. MixNet provides a separate user directory via DDNet storage.cfg. Forks must support standard DDNet storage behavior."
                } else {
                    "Only build sources and mods you trust. Tags are fetched for selection; source code is downloaded only when you build."
                })))
                .when(tool, |col| col.child(muted("Automatic detection: manual path → <TOOL>_EXE → PATH. MSVC is also discovered through Visual Studio Installer. Tools are external; the launcher UI assets are embedded.")))
                .when_some(self.error.clone(), |col, error| col.child(div().text_size(px(12.)).text_color(rgb(c::DANGER)).child(error)))
                .child(row().justify_end().gap(px(10.))
                    .when(tool, |r| r.child(button("reset-tool", "Use automatic", "refresh", false).on_click(cx.listener(|a, _, _, cx| {
                        if let Some(d) = &a.dialog { d.location.update(cx, |i, cx| { i.set(String::new()); cx.notify(); }); }
                        a.commit_dialog(cx);
                    }))))
                    .child(button("save-dialog", if create { "Create instance" } else { "Save" }, "check", true).on_click(cx.listener(|a, _, _, cx| a.commit_dialog(cx))))))
            .into_any_element()
    }
}
