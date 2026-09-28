use std::path::PathBuf;

use gpui_kit::component::{
    Disableable,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    progress::Progress,
};
use gpui_kit::{
    AnyElement, Context, FocusHandle, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, Role, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use simp_core::{InstallTarget, Manifest};

use crate::args::LaunchMode;

// These states will also be driven by installation events when extraction is added.
#[cfg_attr(not(feature = "ui-preview"), allow(dead_code))]
pub(super) enum InstallerState {
    Ready,
    Installing { percent: u8, action: &'static str },
    Finished,
    Failed { message: String },
}

pub(super) struct AppDetails {
    pub name: String,
    pub version: String,
    pub description: String,
    pub destination: Option<PathBuf>,
}

impl AppDetails {
    fn from_manifest(manifest: &Manifest) -> Self {
        let config = &manifest.core_config;
        Self {
            name: config.name.clone(),
            version: config.version.clone(),
            description: config.description.clone().unwrap_or_default(),
            destination: match &config.target {
                InstallTarget::Custom(path) => Some(PathBuf::from(path)),
                _ => None,
            },
        }
    }
}

pub(super) struct InstallerView {
    pub focus: FocusHandle,
    pub state: InstallerState,
    pub details: Option<AppDetails>,
    #[cfg(feature = "ui-preview")]
    pub preview_failure: Option<bool>,
    // Dropping the view cancels the simulation. No detached task keeps it alive.
    #[cfg(feature = "ui-preview")]
    pub preview_task: Option<gpui_kit::Task<()>>,
}

impl InstallerView {
    pub fn new(installer: Result<(LaunchMode, Manifest), String>, cx: &mut Context<Self>) -> Self {
        let (details, message) = match installer {
            #[cfg(feature = "ui-preview")]
            Ok((LaunchMode::Preview { fail }, manifest)) => {
                return Self::preview(AppDetails::from_manifest(&manifest), fail, cx);
            }
            Ok((LaunchMode::Runtime, manifest)) => (
                Some(AppDetails::from_manifest(&manifest)),
                "Installing application files is still under development.".into(),
            ),
            Err(message) => (None, message),
        };
        Self {
            focus: cx.focus_handle(),
            state: InstallerState::Failed { message },
            details,
            #[cfg(feature = "ui-preview")]
            preview_failure: None,
            #[cfg(feature = "ui-preview")]
            preview_task: None,
        }
    }

    fn is_preview(&self) -> bool {
        #[cfg(feature = "ui-preview")]
        return self.preview_failure.is_some();
        #[cfg(not(feature = "ui-preview"))]
        false
    }

    fn heading(&self) -> &'static str {
        match &self.state {
            InstallerState::Ready => "Ready to install",
            InstallerState::Installing { .. } => "Installing your application",
            InstallerState::Finished => "You're all set",
            InstallerState::Failed { .. } if self.details.is_none() => "Installation unavailable",
            InstallerState::Failed { .. } => "Installation couldn't finish",
        }
    }

    fn body(&self) -> AnyElement {
        let content = div().id("state-message").flex().flex_col().gap_4();
        match &self.state {
            InstallerState::Ready => content
                .role(Role::Label)
                .aria_value("Choose Install to get started.")
                .child("Choose Install to get started.")
                .into_any_element(),
            InstallerState::Installing { percent, action } => content
                .child(
                    div()
                        .id("current-action")
                        .flex()
                        .justify_between()
                        .role(Role::Status)
                        .aria_label(format!("{action}. {percent}%"))
                        .child(*action)
                        .child(format!("{percent}%")),
                )
                .child(
                    Progress::new("installation-progress")
                        .value(f32::from(*percent))
                        .color(rgb(0x1463df))
                        .accessibility_label("Installation progress"),
                )
                .into_any_element(),
            InstallerState::Finished => content
                .role(Role::Status)
                .aria_label("The preview completed successfully. No files were installed.")
                .child("The preview completed successfully.")
                .child(
                    div()
                        .text_color(rgb(0x526174))
                        .child("No files were installed."),
                )
                .into_any_element(),
            InstallerState::Failed { message } => content
                .role(Role::Alert)
                .aria_label(message.clone())
                .child(div().text_color(rgb(0xa5262c)).child(message.clone()))
                .into_any_element(),
        }
    }

    fn footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut buttons = div().flex().gap_2().items_center();
        #[cfg(feature = "ui-preview")]
        if self.is_preview()
            && matches!(
                self.state,
                InstallerState::Finished | InstallerState::Failed { .. }
            )
        {
            buttons = buttons.child(
                Button::new("restart-preview")
                    .label("Restart preview")
                    .on_click(cx.listener(|this, _, window, cx| {
                        window.focus(&this.focus, cx);
                        this.preview_task = None;
                        this.state = InstallerState::Ready;
                        cx.notify();
                    })),
            );
        }
        // Closing is safe during the simulation: it performs no installation work.
        buttons = buttons.child(
            Button::new("close")
                .label("Close")
                .on_click(|_, window, _| window.remove_window()),
        );

        if matches!(
            self.state,
            InstallerState::Ready | InstallerState::Installing { .. }
        ) {
            let install = Button::new("install")
                .custom(
                    ButtonCustomVariant::new(cx)
                        .color(rgb(0x1463df).into())
                        .foreground(rgb(0xffffff).into())
                        .hover(rgb(0x1056c6).into())
                        .active(rgb(0x0d47a6).into()),
                )
                .label("Install")
                .disabled(matches!(self.state, InstallerState::Installing { .. }));
            #[cfg(feature = "ui-preview")]
            let install = install.on_click(cx.listener(|this, _, window, cx| {
                window.focus(&this.focus, cx);
                this.start_preview(cx);
            }));
            buttons = buttons.child(install);
        }
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .px_6()
            .py_4()
            .border_t_1()
            .border_color(rgb(0xdce3ec))
            .bg(rgb(0xf5f7fa))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(0x526174))
                    .child("Made with SIMP"),
            )
            .child(buttons)
            .into_any_element()
    }
}

impl Render for InstallerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (name, version, description) = match &self.details {
            Some(details) => (
                details.name.clone(),
                format!("Version {}", details.version),
                details.description.clone(),
            ),
            None => (
                "SIMP Installer".into(),
                String::new(),
                "Simple Installers Made Painless".into(),
            ),
        };
        let mut content = div()
            .id("installer-content")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_6()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_3()
                            .child(
                                div()
                                    .id("application-name")
                                    .text_size(px(26.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .role(Role::Heading)
                                    .aria_label(name.clone())
                                    .child(name),
                            )
                            .child(
                                div()
                                    .id("application-version")
                                    .text_size(px(12.))
                                    .text_color(rgb(0x526174))
                                    .role(Role::Label)
                                    .aria_value(version.clone())
                                    .child(version),
                            ),
                    )
                    .child(
                        div()
                            .id("application-description")
                            .text_color(rgb(0x526174))
                            .role(Role::Label)
                            .aria_value(description.clone())
                            .child(description),
                    ),
            );

        if self.is_preview() {
            content = content.child(
                div()
                    .id("preview-notice")
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(0xeaf2ff))
                    .text_color(rgb(0x174b91))
                    .text_size(px(12.))
                    .role(Role::Note)
                    .aria_label("UI preview. No files will be installed.")
                    .child("UI preview. No files will be installed."),
            );
        }
        if let Some(destination) = self
            .details
            .as_ref()
            .and_then(|details| details.destination.as_ref())
        {
            content = content.child(
                div()
                    .id("destination")
                    .flex()
                    .flex_col()
                    .gap_1()
                    .role(Role::Label)
                    .aria_value(format!("Installation folder: {}", destination.display()))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(0x526174))
                            .child("Installation folder"),
                    )
                    .child(div().child(destination.display().to_string())),
            );
        }
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .id("state-heading")
                        .text_size(px(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .role(Role::Heading)
                        .aria_label(self.heading())
                        .child(self.heading()),
                )
                .child(self.body()),
        );

        div()
            .id("installer")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .text_color(rgb(0x202b3c))
            .font_family("Segoe UI")
            .text_size(px(14.))
            .child(content)
            .child(self.footer(cx))
    }
}
