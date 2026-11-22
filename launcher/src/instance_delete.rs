use crate::{app::Launcher, ui::*};
use gpui::{prelude::*, *};
use mixnet_theme::color as c;

impl Launcher {
    pub fn confirm_delete(&mut self, id: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.dialog = None;
        self.error = None;
        self.phase = "Deleting instance…".into();
        let store = self.store.clone();
        let mut state = self.state.clone();
        // Removing a source checkout or build tree can take time; never block GPUI.
        let deletion = cx.background_executor().spawn(async move {
            let result = store.delete_instance(&mut state, &id);
            (state, result)
        });
        cx.spawn(async move |this, cx| {
            let (state, result) = deletion.await;
            let _ = this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(warnings) => {
                        app.state = state;
                        app.phase = "Instance deleted".into();
                        app.error = if warnings.is_empty() {
                            None
                        } else {
                            Some(format!(
                                "Instance removed, but some files could not be deleted:\n{}",
                                warnings.join("\n")
                            ))
                        };
                        app.log_view.update(cx, |view, cx| view.clear(cx));
                        app.show_logs = false;
                        app.reload_mods();
                    }
                    Err(error) => {
                        app.phase = "Deletion failed".into();
                        app.error = Some(format!("Could not delete instance: {error:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn delete_dialog_view(&self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let name = self
            .state
            .instances
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.name.as_str())
            .unwrap_or("this instance");
        let focus = self.dialog.as_ref().unwrap().name.read(cx).handle();
        let key_id = id.to_owned();
        let button_id = id.to_owned();
        div().id("delete-confirmation").track_focus(&focus)
            .absolute().inset_0().bg(rgba(0x000000b8)).flex().items_center().justify_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(move |a, e: &KeyDownEvent, _, cx| {
                match e.keystroke.key.as_str() {
                    "escape" => { a.dialog = None; a.error = None; cx.notify(); }
                    "enter" => a.confirm_delete(key_id.clone(), cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(column().w(px(500.)).p(px(30.)).gap(px(20.))
                .bg(rgb(c::SURFACE)).border_1().border_color(rgb(c::RULE)).rounded(px(12.))
                .child(div().text_size(px(24.)).font_weight(FontWeight::SEMIBOLD).child("Delete instance?"))
                .child(div().text_color(rgb(c::SOFT)).child(format!("“{name}” and its local data will be permanently deleted.")))
                .child(muted("This includes its client files, user settings, logs, downloaded source and build workspace. This cannot be undone."))
                .child(muted("Shared mods, original external source folders and prebuilt client executables will not be deleted."))
                .child(row().justify_end().gap(px(10.))
                    .child(button("cancel-delete", "Cancel", "close", false).on_click(cx.listener(|a, _, _, cx| {
                        a.dialog = None; a.error = None; cx.notify();
                    })))
                    .child(button("confirm-delete", "Delete instance", "trash", true)
                        .bg(rgb(c::DANGER)).hover(|s| s.bg(rgb(0xff9585)))
                        .on_click(cx.listener(move |a, _, _, cx| a.confirm_delete(button_id.clone(), cx))))))
            .into_any_element()
    }
}
