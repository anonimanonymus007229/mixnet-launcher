use crate::{app::Launcher, dialogs::DialogKind, ui::*};
use gpui::{prelude::*, *};
use mixnet_builder::tools::TOOLS;
use mixnet_theme::{color as c, layout as l};

impl Launcher {
    pub fn tools_page(&self, cx: &mut Context<Self>) -> AnyElement {
        column().gap(px(26.))
            .child(row().justify_between().items_end()
                .child(column().gap(px(7.)).child(section_label("ENVIRONMENT")).child(heading("Tools")))
                .child(button("scan-tools", if self.scanning { "Checking…" } else { "Check again" }, "refresh", false)
                    .on_click(cx.listener(|a, _, _, cx| a.scan_tools(cx)))))
            .child(muted("Manual paths take priority over *_EXE environment variables and PATH. No automatic installers."))
            .children([(false, "LAUNCHER & MIXINS"), (true, "CLIENT BUILD TOOLCHAIN")].into_iter().map(|(build, label)| {
                column().gap(px(12.)).child(section_label(label)).children(TOOLS.iter().filter(move |s| s.build == build).map(|spec| {
                    let status = self.tools.iter().find(|t| t.id == spec.id);
                    let available = status.is_some_and(|t| t.available);
                    let id = spec.id.to_string();
                    let path = status.and_then(|t| t.path.as_ref()).map(|p| p.display().to_string()).unwrap_or_else(|| "Not configured".into());
                    let detail = status.map(|s| format!("{} · {}", s.origin, s.version)).unwrap_or_else(|| "Checking environment…".into());
                    let state_label = if self.scanning && status.is_none() { "CHECKING" } else if available { "AVAILABLE" } else if spec.optional { "OPTIONAL" } else { "MISSING" };
                    row().min_h(px(l::ROW + 18.)).py(px(12.)).gap(px(14.)).border_b_1().border_color(rgb(c::RULE))
                        .child(div().size(px(6.)).rounded_full().bg(rgb(if available { c::SUCCESS } else if spec.optional { c::MUTED } else { c::DANGER })))
                        .child(column().w(px(146.)).gap(px(5.)).child(div().font_weight(FontWeight::SEMIBOLD).child(spec.name))
                            .child(mono(state_label).text_size(px(9.)).text_color(rgb(if available { c::SUCCESS } else { c::MUTED }))))
                        .child(column().flex_1().min_w_0().gap(px(4.))
                            .child(mono(path).text_size(px(10.)).text_ellipsis())
                            .child(muted(spec.purpose).text_size(px(11.)))
                            .child(muted(detail).text_size(px(10.)).text_ellipsis()))
                        .child(button(SharedString::from(format!("tool-{id}")), "Set path", "edit", false)
                            .on_click(cx.listener(move |a, _, window, cx| a.open_dialog(DialogKind::Tool(id.clone()), window, cx))))
                }))
            }))
            .child(rule())
            .child(row().gap(px(12.)).child(column().flex_1().gap(px(6.)).child(section_label("APPLICATION DATA"))
                .child(mono(self.store.root.display().to_string()).text_size(px(11.))))
                .child(button("data-folder", "Open folder", "folder", false).on_click(cx.listener(|a, _, _, cx| a.open_folder(a.store.root.clone(), cx)))))
            .child(muted("Binary instances do not need ddmix or a compiler. CMake can discover installed Visual Studio Build Tools without cl.exe in PATH."))
            .into_any_element()
    }
}
