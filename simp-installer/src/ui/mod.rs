mod installer;
#[cfg(feature = "ui-preview")]
mod preview;

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::{AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use simp_core::Manifest;

use crate::args::LaunchMode;
use installer::InstallerView;

pub fn run(installer: Result<(LaunchMode, Manifest), String>) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Light, None, cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(600.), px(440.)), cx);
            // Open after the platform event loop has started (required on Windows).
            cx.spawn(async move |cx| {
                let result = cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        window_min_size: Some(size(px(540.), px(420.))),
                        titlebar: Some(TitlebarOptions {
                            title: Some("SIMP Installer".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| {
                            let view = InstallerView::new(installer, cx);
                            window.focus(&view.focus, cx);
                            view
                        });
                        cx.new(|cx| Root::new(view, window, cx).bordered(false))
                    },
                );
                if let Err(error) = result {
                    eprintln!("Could not open the installer window: {error:#}");
                    cx.update(|cx| cx.quit());
                }
            })
            .detach();
        });
}
