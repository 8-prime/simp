mod installer;
#[cfg(feature = "ui-preview")]
mod preview;
mod runtime;
mod wizard;

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::{AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, rgb, size};
use simp_core::Manifest;

use crate::args::LaunchMode;
use installer::InstallerView;

pub fn run(installer: Result<(LaunchMode, Manifest), String>) {
    let runtime = tokio::runtime::Handle::current();
    let title = installer
        .as_ref()
        .map(|(_, manifest)| format!("{} Setup", manifest.core_config.name))
        .unwrap_or_else(|_| "Application Setup".into());
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Light, None, cx);
            let theme = Theme::global_mut(cx);
            theme.font_family = "Segoe UI".into();
            theme.font_size = px(14.);
            theme.radius = px(4.);
            theme.colors.primary = rgb(0x087bff).into();
            theme.colors.primary_hover = rgb(0x006be6).into();
            theme.colors.primary_active = rgb(0x005bc4).into();
            theme.colors.primary_foreground = rgb(0xffffff).into();
            theme.colors.foreground = rgb(0x101539).into();
            theme.colors.border = rgb(0xced8e5).into();
            theme.colors.input = rgb(0x9cacc3).into();
            theme.colors.ring = rgb(0x087bff).into();
            theme.tokens.primary = theme.colors.primary.into();
            theme.tokens.button_primary = theme.colors.primary.into();
            theme.tokens.button_primary_hover = theme.colors.primary_hover.into();
            theme.tokens.button_primary_active = theme.colors.primary_active.into();
            theme.colors.button_primary_foreground = rgb(0xffffff).into();
            Theme::sync_base(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(520.), px(500.)), cx);
            // Open after the platform event loop has started (required on Windows).
            cx.spawn(async move |cx| {
                let result = cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        window_min_size: Some(size(px(520.), px(500.))),
                        titlebar: Some(TitlebarOptions {
                            title: Some(title.into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| {
                            let view = InstallerView::new(installer, runtime, window, cx);
                            window.focus(&view.focus, cx);
                            view
                        });
                        InstallerView::guard_window_close(&view, window, cx);
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
