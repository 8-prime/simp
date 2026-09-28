//! Development UI simulation. This module performs no filesystem operations.

use std::time::Duration;

use gpui_kit::Context;

use super::installer::{AppDetails, InstallerState, InstallerView};

impl InstallerView {
    pub(super) fn preview(details: AppDetails, fail: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            state: InstallerState::Ready,
            details: Some(details),
            preview_failure: Some(fail),
            preview_task: None,
        }
    }

    pub(super) fn start_preview(&mut self, cx: &mut Context<Self>) {
        let Some(fail) = self.preview_failure else {
            return;
        };
        if !matches!(self.state, InstallerState::Ready) {
            return;
        }
        self.state = InstallerState::Installing {
            percent: 0,
            action: "Preparing installation",
        };
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
                "Preparing installation"
            } else if step < 17 {
                "Copying application files"
            } else {
                "Finishing up"
            },
        }
    }
}
