use std::path::PathBuf;

use gpui_kit::base::{Progress, Radio, RadioGroup};
use gpui_kit::component::{
    Disableable, IconName,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
};
use gpui_kit::{
    AnyElement, AppContext, Context, Entity, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, ParentElement, PathPromptOptions, Render, Role, StatefulInteractiveElement,
    Styled, Window, div, px, relative, rgb, transparent_black,
};
use simp_core::Manifest;

use super::wizard::{AppDetails, InstallerState, Page};
use crate::args::LaunchMode;

const INK: u32 = 0x101539;
const BLUE: u32 = 0x087bff;
const BORDER: u32 = 0xced8e5;
const SURFACE: u32 = 0xf5f7fa;

pub(super) struct InstallerView {
    pub focus: FocusHandle,
    pub state: InstallerState,
    pub details: Option<AppDetails>,
    manifest: Option<Manifest>,
    destination_input: Entity<InputState>,
    license_accepted: bool,
    pub show_details: bool,
    pub log: Vec<String>,
    pub current_file: String,
    launch_on_finish: bool,
    release_notes: bool,
    folder_error: Option<String>,
    #[cfg(feature = "ui-preview")]
    pub preview_failure: Option<bool>,
    // Dropping the view cancels the simulation. No detached task keeps it alive.
    #[cfg(feature = "ui-preview")]
    pub preview_task: Option<gpui_kit::Task<()>>,
}

impl InstallerView {
    pub fn new(
        installer: Result<(LaunchMode, Manifest), String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        #[cfg(feature = "ui-preview")]
        let mut preview_failure = None;
        let (details, manifest, state) = match installer {
            #[cfg(feature = "ui-preview")]
            Ok((LaunchMode::Preview { fail }, manifest)) => {
                preview_failure = Some(fail);
                let details = AppDetails::from_manifest(&manifest);
                (
                    Some(details),
                    Some(manifest),
                    InstallerState::Choosing(Page::Welcome),
                )
            }
            Ok((LaunchMode::Runtime, manifest)) => {
                let details = AppDetails::from_manifest(&manifest);
                (
                    Some(details),
                    Some(manifest),
                    InstallerState::Choosing(Page::Welcome),
                )
            }
            Err(message) => (None, None, InstallerState::Failed { message }),
        };
        let destination = details
            .as_ref()
            .map(|details| details.destination.display().to_string())
            .unwrap_or_default();
        let destination_input = cx.new(|cx| InputState::new(window, cx).default_value(destination));
        cx.subscribe(&destination_input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(details) = &mut this.details {
                    details.destination = PathBuf::from(input.read(cx).value().as_str());
                }
                this.folder_error = None;
                cx.notify();
            }
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            state,
            details,
            manifest,
            destination_input,
            license_accepted: false,
            show_details: true,
            log: Vec::new(),
            current_file: String::new(),
            launch_on_finish: true,
            release_notes: false,
            folder_error: None,
            #[cfg(feature = "ui-preview")]
            preview_failure,
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

    fn start_install(&mut self, _cx: &mut Context<Self>) {
        let (Some(_manifest), Some(details)) = (self.manifest.as_ref(), self.details.as_ref())
        else {
            return;
        };
        let _options = details.install_options();
        todo!(
            "start an async task, await InstallOperation::run with the manifest and selected options, then forward InstallEvent values to the view"
        )
    }

    fn name(&self) -> &str {
        self.details
            .as_ref()
            .map(|details| details.name.as_str())
            .unwrap_or("Application")
    }

    fn heading(&self) -> String {
        match self.state.page() {
            Some(Page::Welcome) => "Welcome to the Setup Wizard".into(),
            Some(Page::License) => "End User License Agreement".into(),
            Some(Page::Location) => "Choose Install Location".into(),
            Some(Page::Tasks) => "Optional Tasks".into(),
            Some(Page::Ready) => "Ready to Install".into(),
            Some(Page::Installing) => format!("Installing {}", self.name()),
            Some(Page::Finished) => "Completing the Setup Wizard".into(),
            None if self.details.is_none() => "Installation unavailable".into(),
            None => "Installation couldn't finish".into(),
        }
    }

    fn subtitle(&self) -> String {
        match self.state.page() {
            Some(Page::Welcome) => format!("This will install {} on your computer.", self.name()),
            Some(Page::License) => {
                "Please read the following license agreement carefully before continuing.".into()
            }
            Some(Page::Location) => {
                format!("Choose the folder where {} will be installed.", self.name())
            }
            Some(Page::Tasks) => format!(
                "Choose the additional tasks you would like Setup to perform while installing {}.",
                self.name()
            ),
            Some(Page::Ready) => format!(
                "Setup is ready to begin installing {} on your computer.",
                self.name()
            ),
            Some(Page::Installing) => format!(
                "Please wait while Setup installs {} on your computer.",
                self.name()
            ),
            Some(Page::Finished) if self.is_preview() => {
                "The setup preview has completed successfully. No files were installed.".into()
            }
            Some(Page::Finished) => format!(
                "{} has been successfully installed on your computer.",
                self.name()
            ),
            None => "Setup could not complete. Review the details below.".into(),
        }
    }

    fn navigate(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let InstallerState::Choosing(page) = self.state else {
            return;
        };
        let Some(details) = &self.details else { return };
        if forward && !details.can_continue(page, self.license_accepted) {
            return;
        }
        if let Some(page) = details.adjacent_page(page, forward) {
            self.state = InstallerState::Choosing(page);
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select installation folder".into()),
        });
        let view = cx.entity().downgrade();
        window
            .spawn(cx, async move |cx| {
                let result = selection.await;
                let _ = cx.update(|window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        match result {
                            Ok(Ok(Some(paths))) => {
                                if let Some(path) = paths.into_iter().next() {
                                    this.destination_input.update(cx, |input, cx| {
                                        input.set_value(path.display().to_string(), window, cx);
                                    });
                                    if let Some(details) = &mut this.details {
                                        details.destination = path;
                                    }
                                    this.folder_error = None;
                                }
                            }
                            Ok(Ok(None)) => {}
                            _ => this.folder_error = Some(
                                "The folder picker could not open. Enter a folder path instead."
                                    .into(),
                            ),
                        }
                        cx.notify();
                    });
                });
            })
            .detach();
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div().flex().flex_col().gap_4();
        match &self.state {
            InstallerState::Choosing(Page::Welcome) => {
                let mut content = content.pt_2();
                if let Some(description) = self
                    .details
                    .as_ref()
                    .map(|details| details.description.as_str())
                    .filter(|description| !description.trim().is_empty())
                {
                    content = content.child(description.to_owned());
                }
                content.into_any_element()
            }
            InstallerState::Choosing(Page::License) => {
                let accepting_view = cx.entity().downgrade();
                let declining_view = cx.entity().downgrade();
                content
                    .gap_3()
                    .child(
                        div()
                            .id("license-text")
                            .h(px(180.))
                            .border_1()
                            .border_color(rgb(BORDER))
                            .rounded(px(4.))
                            .p_3()
                            .text_size(px(13.))
                            .child(
                                self.details
                                    .as_ref()
                                    .and_then(|details| details.license.clone())
                                    .unwrap_or_default(),
                            )
                            .overflow_y_scrollbar(),
                    )
                    .child(
                        RadioGroup::new("license-consent")
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                license_radio(
                                    "accept-license",
                                    "I accept the terms in the License Agreement",
                                    self.license_accepted,
                                )
                                .on_change(move |_, _, _, cx| {
                                    if let Some(view) = accepting_view.upgrade() {
                                        view.update(cx, |this, cx| {
                                            this.license_accepted = true;
                                            cx.notify();
                                        });
                                    }
                                }),
                            )
                            .child(
                                license_radio(
                                    "decline-license",
                                    "I do not accept the terms in the License Agreement",
                                    !self.license_accepted,
                                )
                                .on_change(move |_, _, _, cx| {
                                    if let Some(view) = declining_view.upgrade() {
                                        view.update(cx, |this, cx| {
                                            this.license_accepted = false;
                                            cx.notify();
                                        });
                                    }
                                }),
                            ),
                    )
                    .into_any_element()
            }
            InstallerState::Choosing(Page::Location) => {
                let mut content =
                    content.pt_6().child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.destination_input)
                                        .h(px(36.))
                                        .text_size(px(13.))
                                        .aria_label("Installation folder"),
                                ),
                            )
                            .child(secondary_button("browse", "Browse...").on_click(
                                cx.listener(|this, _, window, cx| this.browse(window, cx)),
                            )),
                    );
                if let Some(details) = &self.details {
                    let mib = details.required_bytes.div_ceil(1024 * 1024).max(1);
                    content = content.child(
                        div()
                            .text_size(px(13.))
                            .child(format!("At least {mib} MB of free disk space is required.")),
                    );
                    if !details.destination.is_absolute() {
                        content = content.child(
                            div()
                                .text_color(rgb(0xa5262c))
                                .text_size(px(13.))
                                .child("Enter a full folder path to continue."),
                        );
                    }
                }
                if let Some(error) = &self.folder_error {
                    content = content.child(div().text_color(rgb(0xa5262c)).child(error.clone()));
                }
                content.into_any_element()
            }
            InstallerState::Choosing(Page::Tasks) => {
                let mut content = content.pt_3();
                if let Some(details) = &self.details {
                    for (index, task) in details.tasks.iter().enumerate() {
                        content = content.child(
                            Checkbox::new(("optional-task", index))
                                .label(task.label)
                                .checked(task.selected)
                                .text_size(px(14.))
                                .on_change(cx.listener(move |this, checked, _, cx| {
                                    if let Some(task) = this
                                        .details
                                        .as_mut()
                                        .and_then(|details| details.tasks.get_mut(index))
                                    {
                                        task.selected = *checked;
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                }
                content.into_any_element()
            }
            InstallerState::Choosing(Page::Ready) => {
                let mut summary = div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_4()
                    .min_h(px(190.))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .rounded(px(5.))
                    .bg(rgb(SURFACE));
                if let Some(details) = &self.details {
                    summary = summary
                        .child(summary_row(
                            "Application:",
                            format!("{} ({})", details.name, details.version),
                        ))
                        .child(summary_row(
                            "Install location:",
                            details.destination.display().to_string(),
                        ));
                    if !details.tasks.is_empty() {
                        let tasks: Vec<_> = details
                            .tasks
                            .iter()
                            .filter(|task| task.selected)
                            .map(|task| task.label)
                            .collect();
                        summary = summary.child(summary_row(
                            "Additional tasks:",
                            if tasks.is_empty() {
                                "None".into()
                            } else {
                                tasks.join("\n")
                            },
                        ));
                    }
                }
                if !self.is_preview() {
                    content = content.child(
                        div()
                            .text_color(rgb(0xa5262c))
                            .text_size(px(13.))
                            .child("Installation is not available in this build yet."),
                    );
                }
                content.child(summary).into_any_element()
            }
            InstallerState::Installing { percent, action } => {
                let mut content = content
                    .gap_2()
                    .child(
                        div()
                            .id("current-action")
                            .flex()
                            .justify_between()
                            .role(Role::Status)
                            .aria_label(format!("{action} {percent}%"))
                            .child(*action)
                            .child(format!("{percent}%")),
                    )
                    .child(
                        Progress::new("installation-progress")
                            .value(f32::from(*percent))
                            .w_full()
                            .h(px(16.))
                            .border_1()
                            .border_color(rgb(BORDER))
                            .rounded(px(3.))
                            .bg(rgb(0xe2e7ef))
                            .overflow_hidden()
                            .accessibility_label("Installation progress")
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(f32::from(*percent) / 100.))
                                    .bg(rgb(0x24c66a)),
                            ),
                    )
                    .child(div().text_size(px(12.)).child(self.current_file.clone()))
                    .child(
                        Button::new("show-details")
                            .ghost()
                            .self_start()
                            .h(px(24.))
                            .px_0()
                            .text_size(px(12.))
                            .icon(if self.show_details {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .label(if self.show_details {
                                "Hide details"
                            } else {
                                "Show details"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_details = !this.show_details;
                                cx.notify();
                            })),
                    );
                if self.show_details {
                    content = content.child(
                        div()
                            .id("installation-log")
                            .h(px(126.))
                            .border_1()
                            .border_color(rgb(BORDER))
                            .rounded(px(4.))
                            .bg(rgb(SURFACE))
                            .px_3()
                            .py_2()
                            .font_family("Consolas")
                            .text_size(px(12.))
                            .child(self.log.join("\n"))
                            .overflow_y_scrollbar(),
                    );
                }
                content.into_any_element()
            }
            InstallerState::Finished => content
                .pt_4()
                .child(
                    Checkbox::new("launch-on-finish")
                        .label("Launch application now")
                        .checked(self.launch_on_finish)
                        .on_change(cx.listener(|this, checked, _, cx| {
                            this.launch_on_finish = *checked;
                            cx.notify();
                        })),
                )
                .child(
                    Checkbox::new("view-release-notes")
                        .label("View release notes")
                        .checked(self.release_notes)
                        .on_change(cx.listener(|this, checked, _, cx| {
                            this.release_notes = *checked;
                            cx.notify();
                        })),
                )
                .into_any_element(),
            InstallerState::Failed { message } => content
                .child(
                    div()
                        .id("installation-error")
                        .role(Role::Alert)
                        .aria_label(message.clone())
                        .border_1()
                        .border_color(rgb(0xefc9cc))
                        .rounded(px(5.))
                        .bg(rgb(0xfff6f6))
                        .p_4()
                        .text_color(rgb(0xa5262c))
                        .child(message.clone()),
                )
                .into_any_element(),
            InstallerState::Choosing(_) => content.into_any_element(),
        }
    }

    fn footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = self.state.page();
        let mut dots = div()
            .id("wizard-progress")
            .flex()
            .items_center()
            .gap(px(9.))
            .role(Role::Status);
        if let Some(details) = &self.details {
            let pages = details.pages();
            if let Some(index) = pages.iter().position(|step| Some(*step) == page) {
                dots = dots.aria_label(format!(
                    "Step {} of {}: {}",
                    index + 1,
                    pages.len(),
                    self.heading()
                ));
            } else {
                dots = dots.aria_label("Installation stopped");
            }
            for step in pages {
                let active = Some(step) == page;
                dots = dots.child(
                    div()
                        .size(px(10.))
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(if active { BLUE } else { 0x9cacc3 }))
                        .bg(rgb(if active { BLUE } else { 0xffffff })),
                );
            }
        }
        let mut buttons = div().flex().items_center().gap_2();
        match &self.state {
            InstallerState::Choosing(page) => {
                if *page != Page::Welcome {
                    buttons = buttons.child(secondary_button("back", "‹ Back").on_click(
                        cx.listener(|this, _, window, cx| this.navigate(false, window, cx)),
                    ));
                }
                let ready = *page == Page::Ready;
                let enabled = self.details.as_ref().is_some_and(|details| {
                    if ready {
                        self.is_preview()
                    } else {
                        details.can_continue(*page, self.license_accepted)
                    }
                });
                buttons = buttons.child(
                    primary_button("next", if ready { "Install" } else { "Next ›" })
                        .disabled(!enabled)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if ready {
                                if this.is_preview() {
                                    #[cfg(feature = "ui-preview")]
                                    this.start_preview(cx);
                                } else {
                                    this.start_install(cx);
                                }
                                window.focus(&this.focus, cx);
                            } else {
                                this.navigate(true, window, cx);
                            }
                        })),
                );
            }
            InstallerState::Installing { .. } => {
                buttons = buttons
                    .child(secondary_button("back", "‹ Back").disabled(true))
                    .child(secondary_button("next", "Next ›").disabled(true));
            }
            InstallerState::Finished => {
                return footer_row(
                    dots,
                    buttons.child(
                        primary_button("finish", "Finish")
                            .w(px(104.))
                            .on_click(|_, window, _| window.remove_window()),
                    ),
                );
            }
            InstallerState::Failed { .. } =>
            {
                #[cfg(feature = "ui-preview")]
                if self.is_preview() {
                    buttons =
                        buttons.child(primary_button("restart-preview", "Try again").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.preview_task = None;
                                this.state = InstallerState::Choosing(Page::Ready);
                                window.focus(&this.focus, cx);
                                cx.notify();
                            }),
                        ));
                }
            }
        }
        // Closing only stops the UI simulation; no installation side effects exist yet.
        buttons = buttons.child(
            secondary_button("cancel", if page.is_none() { "Close" } else { "Cancel" })
                .on_click(|_, window, _| window.remove_window()),
        );
        footer_row(dots, buttons)
    }
}

fn secondary_button(id: &'static str, label: &'static str) -> Button {
    Button::new(id)
        .label(label)
        .h(px(36.))
        .min_w(px(78.))
        .px_3()
        .rounded(px(4.))
        .text_size(px(13.))
}

fn primary_button(id: &'static str, label: &'static str) -> Button {
    secondary_button(id, label).primary()
}

fn license_radio(id: &'static str, label: &'static str, checked: bool) -> Radio {
    let mut indicator = div()
        .size(px(16.))
        .flex_shrink_0()
        .rounded_full()
        .border_1()
        .border_color(rgb(if checked { BLUE } else { 0x9cacc3 }))
        .bg(rgb(if checked { BLUE } else { 0xffffff }))
        .flex()
        .items_center()
        .justify_center();
    if checked {
        indicator = indicator.child(div().size(px(6.)).rounded_full().bg(rgb(0xffffff)));
    }
    Radio::new(id)
        .checked(checked)
        .accessibility_label(label)
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(13.))
        .rounded(px(3.))
        .border_1()
        .border_color(transparent_black())
        .focus_visible(|style| style.border_color(rgb(BLUE)))
        .child(indicator)
        .child(label)
}

fn summary_row(label: &'static str, value: String) -> impl IntoElement {
    div()
        .flex()
        .items_start()
        .gap_3()
        .child(
            div()
                .w(px(118.))
                .flex_shrink_0()
                .font_weight(FontWeight::SEMIBOLD)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(value))
}

fn footer_row(dots: impl IntoElement, buttons: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .gap_4()
        .h(px(64.))
        .px_5()
        .border_t_1()
        .border_color(rgb(0xe3e9f1))
        .bg(rgb(0xfcfdff))
        .child(dots)
        .child(buttons)
        .into_any_element()
}

impl Render for InstallerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = div()
            .id((
                "installer-content",
                self.state.page().map(|page| page as usize).unwrap_or(7),
            ))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_8()
            .pt_7()
            .pb_4()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .id("state-heading")
                            .text_size(px(24.))
                            .line_height(px(30.))
                            .font_weight(FontWeight::BOLD)
                            .role(Role::Heading)
                            .aria_label(self.heading())
                            .child(self.heading()),
                    )
                    .child(div().child(self.subtitle())),
            )
            .child(self.body(cx));
        let mut root = div()
            .id("installer")
            .track_focus(&self.focus)
            .tab_group()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .text_color(rgb(INK))
            .font_family("Segoe UI")
            .text_size(px(14.))
            .line_height(px(19.))
            .child(content);
        if self.is_preview() {
            root = root.child(
                div()
                    .id("preview-notice")
                    .flex_shrink_0()
                    .px_8()
                    .pb_2()
                    .text_size(px(11.))
                    .text_color(rgb(0x66758b))
                    .role(Role::Note)
                    .child("UI preview. No files or settings will be changed."),
            );
        }
        root.child(self.footer(cx))
    }
}
