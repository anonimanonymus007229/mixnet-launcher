use crate::{app::Launcher, input::Input, source_picker::SourcePicker, ui::*};
use gpui::{prelude::*, *};
use mixnet_core::{Client, Instance, State};
use mixnet_theme::color as c;
use std::path::PathBuf;

#[derive(Clone)]
pub enum DialogKind {
    Create,
    Edit(String),
    Delete(String),
    Tool(String),
}
pub struct Dialog {
    pub kind: DialogKind,
    pub binary: bool,
    pub name: Entity<Input>,
    pub location: Entity<Input>,
    pub revision: Entity<Input>,
    pub source: SourcePicker,
    original_client: Option<Client>,
}
impl Dialog {
    pub fn new(kind: DialogKind, state: &State, cx: &mut Context<Launcher>) -> Self {
        let original = if let DialogKind::Edit(id) = &kind {
            state.instances.iter().find(|i| &i.id == id)
        } else {
            None
        };
        let name = original
            .map(|i| i.name.clone())
            .unwrap_or_else(|| "DDNet".into());
        let (location, revision, binary) = match original.map(|i| &i.client) {
            Some(Client::Source { location, revision }) => {
                (location.clone(), revision.clone(), false)
            }
            Some(Client::Binary { executable }) => (
                executable.to_string_lossy().into_owned(),
                String::new(),
                true,
            ),
            None => (
                "https://github.com/ddnet/ddnet.git".into(),
                String::new(),
                false,
            ),
        };
        let location = if let DialogKind::Tool(id) = &kind {
            state
                .tools
                .get(id)
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        } else {
            location
        };
        let mut source = SourcePicker::new(cx);
        if original.is_some() && !binary {
            source.seed_location(&location);
        }
        Self {
            kind,
            binary,
            name: cx.new(|cx| Input::new(name, "Instance name", cx)),
            location: cx.new(|cx| Input::new(location, "Path to executable", cx)),
            revision: cx.new(|cx| {
                Input::new(
                    revision,
                    "Full commit hash (filled when you select a tag)",
                    cx,
                )
            }),
            source,
            original_client: original.map(|i| i.client.clone()),
        }
    }
    fn client(&self, cx: &App) -> anyhow::Result<Client> {
        let location = self
            .location
            .read(cx)
            .text
            .trim()
            .trim_matches('"')
            .to_owned();
        let revision = self.revision.read(cx).text.trim().to_owned();
        // A name-only edit must also work offline, without resetting a pinned commit.
        if let Some(original) = &self.original_client {
            let unchanged = match original {
                Client::Source {
                    location: old_location,
                    revision: old_revision,
                } => !self.binary && *old_location == location && *old_revision == revision,
                Client::Binary { executable } => {
                    self.binary && executable.to_string_lossy() == location
                }
            };
            if unchanged {
                return Ok(original.clone());
            }
        }
        if self.binary {
            let executable = PathBuf::from(&location)
                .canonicalize()
                .map_err(|e| anyhow::anyhow!("Cannot open executable: {e}"))?;
            anyhow::ensure!(executable.is_file(), "Choose a client executable.");
            return Ok(Client::Binary { executable });
        }
        anyhow::ensure!(
            self.source.input == location,
            "Wait for the source validation to finish."
        );
        if let Some(error) = &self.source.error {
            anyhow::bail!("{error}");
        }
        let repo = self.source.repository.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Wait for the repository check and tag list to finish loading.")
        })?;
        let (location, _) = mixnet_builder::repository::validate_location(&repo.location)?;
        let revision = repo.resolve(&revision)?;
        Ok(Client::Source { location, revision })
    }
}
impl Launcher {
    pub fn commit_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(d) = &self.dialog else {
            return;
        };
        let kind = d.kind.clone();
        let name = d.name.read(cx).text.trim().to_string();
        let mut next = self.state.clone();
        let mut client_changed = false;
        let location = d
            .location
            .read(cx)
            .text
            .trim()
            .trim_matches('"')
            .to_string();
        let result = (|| -> anyhow::Result<()> {
            match kind {
                DialogKind::Delete(_) => anyhow::bail!("Use the deletion confirmation."),
                DialogKind::Edit(id) => {
                    client_changed = next.edit_instance(&id, name, d.client(cx)?)?;
                }
                DialogKind::Tool(id) => {
                    if location.is_empty() {
                        next.tools.remove(&id);
                    } else {
                        let path = PathBuf::from(&location)
                            .canonicalize()
                            .map_err(|e| anyhow::anyhow!("Cannot open executable: {e}"))?;
                        anyhow::ensure!(
                            path.is_file(),
                            "Choose an executable file, not a directory."
                        );
                        next.tools.insert(id, path);
                    }
                }
                DialogKind::Create => {
                    anyhow::ensure!(
                        !name.is_empty() && name.chars().count() <= 80,
                        "Use an instance name between 1 and 80 characters."
                    );
                    let instance = Instance::new(name, d.client(cx)?);
                    self.store.prepare(&instance)?;
                    next.selected = instance.id.clone();
                    next.instances.push(instance);
                }
            }
            self.store.save(&next)?;
            Ok(())
        })();
        match result {
            Err(e) => self.error = Some(format!("{e:#}")),
            Ok(()) => {
                self.state = next;
                self.dialog = None;
                self.error = None;
                if client_changed {
                    self.phase = "Instance settings changed · build required".into();
                }
                self.reload_mods();
                self.scanning = false;
                self.scan_tools(cx);
            }
        }
        cx.notify();
    }
    pub fn dialog_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let d = self.dialog.as_ref().unwrap();
        if let DialogKind::Delete(id) = &d.kind {
            return self.delete_dialog_view(id, cx);
        }
        let create = matches!(d.kind, DialogKind::Create);
        let instance_form = create || matches!(d.kind, DialogKind::Edit(_));
        let tool = matches!(d.kind, DialogKind::Tool(_));
        let title = match &d.kind {
            DialogKind::Create => "New instance".to_string(),
            DialogKind::Edit(_) => "Edit instance".to_string(),
            DialogKind::Delete(_) => "Delete instance".to_string(),
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
                        fields.push(focus_location.clone());
                        if instance_form && !binary {
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
                .when(instance_form, |col| col.child(row().gap(px(10.))
                    .child(button("source-kind", "Open source", "code", !binary).flex_1()
                        .on_click(cx.listener(|a, _, _, cx| {
                            if let Some(d) = &mut a.dialog && d.binary { d.binary = false; d.location.update(cx, |i, cx| { i.set("https://github.com/ddnet/ddnet.git".into()); cx.notify(); }); } cx.notify();
                        })))
                    .child(button("binary-kind", "Prebuilt / closed source", "box", binary).flex_1()
                        .on_click(cx.listener(|a, _, _, cx| {
                            if let Some(d) = &mut a.dialog && !d.binary { d.binary = true; d.location.update(cx, |i, cx| { i.set(String::new()); cx.notify(); }); } cx.notify();
                        })))))
                .when(!tool, |col| col.child(column().gap(px(8.)).child(section_label("NAME")).child(d.name.clone())))
                .child(column().gap(px(8.))
                    .child(section_label(if tool { "EXECUTABLE PATH · EMPTY = AUTOMATIC" } else if binary { "CLIENT EXECUTABLE" } else { "GIT URL OR LOCAL SOURCE FOLDER" }))
                    .child(row().gap(px(8.)).child(div().flex_1().min_w_0().child(d.location.clone()))
                        .child(button("browse-field", "Browse", "folder", false)
                            .on_click(cx.listener(move |a, _, _, cx| a.pick_field(tool || binary, cx))))))
                .when(instance_form && !binary, |col| col.child(self.source_fields(cx)))
                .when(instance_form, |col| col.child(muted(if binary {
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
