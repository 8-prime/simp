//! Development UI simulation. This module performs no filesystem operations.

use std::time::Duration;

use gpui_kit::Context;

use super::{
    installer::InstallerView,
    wizard::{InstallerState, Page},
};

impl InstallerView {
    pub(super) fn start_preview(&mut self, cx: &mut Context<Self>) {
        let Some(fail) = self.preview_failure else {
            return;
        };
        if !matches!(self.state, InstallerState::Choosing(Page::Ready)) {
            return;
        }
        self.state = InstallerState::Installing {
            percent: 0,
            action: "Preparing to install...",
        };
        self.log = vec!["Preparing to install...".into()];
        self.current_file.clear();
        cx.notify();

        // Timers yield to GPUI's event loop. Real synchronous installation will run
        // on a worker and send semantic events to this view instead.
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            for step in 1..=20_u8 {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let Some(view) = this.upgrade() else { return };
                view.update(cx, |view, cx| {
                    view.state = preview_step(step, fail);
                    let line = match step {
                        4 => Some("Extracting files...".to_owned()),
                        7 | 11 => {
                            let file = if step == 7 { "app.dll" } else { "core.dll" };
                            view.current_file = view
                                .details
                                .as_ref()
                                .map(|details| details.destination.join(file).display().to_string())
                                .unwrap_or_default();
                            Some(view.current_file.clone())
                        }
                        14 => Some("Copying files...".to_owned()),
                        17 => Some("Applying selected tasks...".to_owned()),
                        19 => Some("Finalizing installation...".to_owned()),
                        _ => None,
                    };
                    if let Some(line) = line {
                        view.log.push(line);
                    }
                    cx.notify();
                });
                if fail && step == 12 {
                    return;
                }
            }
        }));
    }
}

fn preview_step(step: u8, fail: bool) -> InstallerState {
    if fail && step >= 12 {
        InstallerState::Failed {
            message: "The preview simulated a file that could not be written. In a real installation, check that the destination is writable and try again. No files were changed.".into(),
        }
    } else if step >= 20 {
        InstallerState::Finished
    } else {
        InstallerState::Installing {
            percent: step * 5,
            action: if step < 4 {
                "Preparing to install..."
            } else if step < 17 {
                "Extracting files..."
            } else {
                "Finalizing installation..."
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_reaches_finished_and_failure_never_does() {
        assert!(matches!(
            preview_step(19, false),
            InstallerState::Installing { percent: 95, .. }
        ));
        assert!(matches!(preview_step(20, false), InstallerState::Finished));
        assert!(matches!(
            preview_step(12, true),
            InstallerState::Failed { .. }
        ));
        assert!(matches!(
            preview_step(20, true),
            InstallerState::Failed { .. }
        ));
    }
}
