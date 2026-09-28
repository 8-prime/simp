// The operation is deliberately unfinished. Runtime installation stays disabled
// until every TODO below is implemented and the UI handoff is connected.
#![allow(dead_code, unused_variables)]

use std::path::{Path, PathBuf};

use simp_core::{Manifest, ManifestEntry};

/// Progress reported by the installation operation, independent of the UI.
pub enum InstallEvent {
    Started {
        total_files: u64,
    },
    InstallingFile {
        path: PathBuf,
        completed: u64,
        total: u64,
    },
    ApplyingTask {
        task: InstallTask,
    },
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InstallTask {
    DesktopShortcut,
    StartMenuEntry,
    Autostart,
    AddToPath,
}

/// Final actions selected in the wizard. An unavailable (`None`) task and a
/// visible but unchecked task both become `false` here.
pub struct InstallOptions {
    pub destination: PathBuf,
    pub desktop_shortcut: bool,
    pub start_menu_entry: bool,
    pub autostart: bool,
    pub add_to_path: bool,
}

pub trait EventSink {
    fn emit(&mut self, event: InstallEvent);
}

/// Synchronous installation work. The caller chooses which thread runs it.
pub struct InstallOperation {
    manifest: Manifest,
    options: InstallOptions,
}

impl InstallOperation {
    pub fn new(manifest: Manifest, options: InstallOptions) -> Self {
        Self { manifest, options }
    }

    pub fn run(&self, events: &mut dyn EventSink) -> eyre::Result<()> {
        todo!(
            "validate entries, create the destination, install files, apply selected tasks, and emit progress through events"
        )
    }

    fn validate_entries(&self) -> eyre::Result<()> {
        todo!(
            "reject empty, absolute, parent-traversing, and duplicate entry paths before writing anything"
        )
    }

    fn create_destination(&self) -> eyre::Result<()> {
        todo!("create the selected installation directory with a useful error on failure")
    }

    fn install_files(&self, events: &mut dyn EventSink) -> eyre::Result<()> {
        todo!("write each validated entry and emit InstallingFile progress")
    }

    fn write_entry(&self, entry: &ManifestEntry, path: &Path) -> eyre::Result<()> {
        todo!("write one validated manifest entry beneath the destination directory")
    }

    fn apply_optional_tasks(&self, events: &mut dyn EventSink) -> eyre::Result<()> {
        todo!("perform only the tasks selected in InstallOptions and emit ApplyingTask events")
    }
}
