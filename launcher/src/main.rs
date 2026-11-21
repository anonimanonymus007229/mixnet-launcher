#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod app;
mod dialogs;
mod home;
mod input;
mod log_view;
mod source_picker;
mod tools;
mod ui;

use gpui::{prelude::*, *};
use mixnet_core::Store;
use std::{borrow::Cow, fs};

fn main() {
    if let Err(error) = launch() {
        // A GUI-subsystem executable has no stderr console. Preserve startup failures.
        let message = format!("MixNet could not start:\n{error:#}");
        let path = std::env::temp_dir().join("mixnet-launcher-startup-error.txt");
        let _ = fs::write(&path, &message);
        let _ = mixnet_builder::tools::command("notepad.exe")
            .arg(&path)
            .spawn();
    }
}
fn launch() -> anyhow::Result<()> {
    let store = Store::roaming()?;
    let state = store.initialize()?;
    gpui_platform::application()
        .with_assets(ui::Assets)
        .run(move |cx: &mut App| {
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(include_bytes!("../../assets/fonts/Geist-Regular.ttf")),
                    Cow::Borrowed(include_bytes!("../../assets/fonts/Geist-SemiBold.ttf")),
                    Cow::Borrowed(include_bytes!("../../assets/fonts/GeistMono-Regular.ttf")),
                ])
                .expect("embedded fonts");
            let bounds = Bounds::centered(None, size(px(1180.), px(800.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(1000.), px(680.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("MixNet".into()),
                        ..Default::default()
                    }),
                    app_id: Some("mixnet-launcher".into()),
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| app::Launcher::new(store, state, cx)),
            )
            .expect("MixNet window");
            cx.on_window_closed(|cx, _| {
                cx.quit();
            })
            .detach();
            cx.activate(true);
        });
    Ok(())
}
