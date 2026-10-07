//! Runs the operation on Tokio and forwards its events to the UI executor.

use std::sync::Arc;

use crate::install::InstallerPackage;
use tokio::{runtime::Handle, sync::mpsc, task::JoinHandle};

use crate::install::operation::{EventSink, InstallEvent, InstallOperation, InstallOptions};

struct UiEvents(mpsc::UnboundedSender<InstallEvent>);

impl EventSink for UiEvents {
    fn emit(&mut self, event: InstallEvent) {
        let _ = self.0.send(event);
    }
}

pub(super) fn start(
    runtime: &Handle,
    manifest: Arc<InstallerPackage>,
    options: InstallOptions,
) -> (
    mpsc::UnboundedReceiver<InstallEvent>,
    JoinHandle<Result<(), String>>,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let operation = InstallOperation::new(manifest, options);
    let worker = runtime.spawn(async move {
        operation
            .run(&mut UiEvents(sender))
            .await
            .map_err(|error| format!("{error:#}"))
    });
    (receiver, worker)
}
