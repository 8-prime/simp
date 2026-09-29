// The operation is deliberately unfinished. Runtime installation stays disabled
// until every TODO below is implemented and the UI handoff is connected.
#![allow(dead_code, unused_variables)]

use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

use simp_core::{Manifest, ManifestEntry};
use tokio::{
    fs::{File, create_dir_all},
    io::AsyncWriteExt,
};

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

/// Asynchronous installation work, independent of the UI event loop.
pub struct InstallOperation {
    manifest: Manifest,
    options: InstallOptions,
}

impl InstallOperation {
    pub fn new(manifest: Manifest, options: InstallOptions) -> Self {
        Self { manifest, options }
    }

    pub async fn run(&self, events: &mut dyn EventSink) -> eyre::Result<()> {
        events.emit(InstallEvent::Started {
            total_files: self.manifest.entries.len() as u64,
        });
        self.validate_entries()?;
        self.create_destination().await?;
        self.install_files(events).await?;
        events.emit(InstallEvent::Finished);
        return Ok(());
    }

    fn validate_entries(&self) -> eyre::Result<()> {
        let mut entries_set: HashSet<&String> = HashSet::new();
        for entry in self.manifest.entries.iter() {
            if entry.name.is_empty() {
                eyre::bail!("File name in manifest cannot be empty")
            }

            if Self::is_path_trav(&self.options.destination, &entry.name) {
                eyre::bail!(
                    "File {} tries to traverse path, which is disallowed",
                    entry.name
                )
            }

            if entries_set.contains(&entry.name) {
                eyre::bail!("Duplicate file {} found in manifest", entry.name)
            }
            entries_set.insert(&entry.name);
        }
        Ok(())
    }

    fn is_path_trav(base_path: &Path, file_name: &str) -> bool {
        let file_path = Path::new(file_name);

        // `starts_with` is lexical: base/../outside would still start with base.
        if file_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return true;
        }

        !base_path.join(file_path).starts_with(base_path)
    }

    async fn create_destination(&self) -> eyre::Result<()> {
        if self.options.destination.exists() {
            return Ok(());
        }

        create_dir_all(&self.options.destination).await?;
        return Ok(());
    }

    async fn install_files(&self, events: &mut dyn EventSink) -> eyre::Result<()> {
        for entry in self.manifest.entries.iter() {
            let target_path = self.options.destination.join(&entry.name);
            self.write_entry(&entry, &target_path).await?;
            events.emit(InstallEvent::InstallingFile {
                path: target_path,
                completed: 0, //TODO: correctly set this plz
                total: 0,     //TODO: correctly set this plz
            });
        }
        return Ok(());
    }

    async fn write_entry(&self, entry: &ManifestEntry, path: &Path) -> eyre::Result<()> {
        let mut file = File::create(path).await?;
        file.write_all(&entry.data).await?;

        return Ok(());
    }

    async fn apply_optional_tasks(&self, events: &mut dyn EventSink) -> eyre::Result<()> {
        //TODO: "perform selected tasks asynchronously (use spawn_blocking for blocking Windows APIs) and emit ApplyingTask events"
        return Ok(());
    }
}

#[cfg(test)]
mod tests {
    use super::{InstallOperation, InstallOptions};
    use simp_core::{CoreConfig, InstallTarget, Manifest, ManifestEntry};
    use std::path::Path;

    fn operation_with_entries(names: &[&str]) -> InstallOperation {
        InstallOperation::new(
            Manifest {
                core_config: CoreConfig {
                    name: "Example".into(),
                    version: "1.0".into(),
                    description: None,
                    eula: None,
                    auto_start: None,
                    desktop_icon: None,
                    start_menu: None,
                    add_to_path: None,
                    target: InstallTarget::ProgramFiles,
                },
                entries: names
                    .iter()
                    .map(|name| ManifestEntry {
                        name: (*name).into(),
                        data: Vec::new(),
                    })
                    .collect(),
            },
            InstallOptions {
                destination: "install".into(),
                desktop_shortcut: false,
                start_menu_entry: false,
                autostart: false,
                add_to_path: false,
            },
        )
    }

    #[test]
    fn validate_entries_accepts_safe_names_and_rejects_invalid_ones() {
        assert!(
            operation_with_entries(&["app.exe", "data/config.json"])
                .validate_entries()
                .is_ok()
        );
        for names in [vec![""], vec!["../outside.txt"], vec!["app.exe", "app.exe"]] {
            assert!(operation_with_entries(&names).validate_entries().is_err());
        }
    }

    #[test]
    fn path_traversal_rejects_parent_components() {
        let base = Path::new("install");
        assert!(InstallOperation::is_path_trav(base, "../outside.txt"));
        assert!(InstallOperation::is_path_trav(base, "sub/../inside.txt"));
    }

    #[test]
    fn path_traversal_accepts_relative_files_beneath_the_destination() {
        let base = Path::new("install");
        assert!(!InstallOperation::is_path_trav(base, "file.txt"));
        assert!(!InstallOperation::is_path_trav(base, "sub/file.txt"));
    }

    #[cfg(windows)]
    #[test]
    fn path_traversal_rejects_windows_absolute_and_drive_relative_paths() {
        let base = Path::new(r"C:\Apps\Simp");
        for file in [
            r"C:\Windows\file.txt",
            r"\Windows\file.txt",
            r"C:file.txt",
            r"\\server\share\file.txt",
        ] {
            assert!(InstallOperation::is_path_trav(base, file));
        }
    }
}
