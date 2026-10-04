// File installation is implemented. Optional Windows integration is still pending.
#![allow(dead_code, unused_variables)]

use std::{
    collections::HashSet,
    default,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use simp_core::{Manifest, ManifestEntry, PathScope, PathScopePolicy};
use tokio::{
    fs::{File, create_dir_all},
    io::AsyncWriteExt,
};

/// Progress reported by the installation operation, independent of the UI.
pub enum InstallEvent {
    Started {
        total_files: u64,
    },
    /// Emitted after a file is fully written. Counts are files, not bytes.
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

/// Final actions selected in the wizard, sharing one application launch target.
pub struct InstallOptions {
    pub destination: PathBuf,
    /// Packaged launch target for shortcuts and autostart, relative to `destination`.
    pub main_executable: Option<PathBuf>,
    pub desktop_shortcut: bool,
    pub start_menu_entry: bool,
    pub autostart: bool,
    pub add_to_path: Option<PathScope>,
}

pub trait EventSink {
    fn emit(&mut self, event: InstallEvent);
}

/// Asynchronous installation work, independent of the UI event loop.
pub struct InstallOperation {
    manifest: Arc<Manifest>,
    options: InstallOptions,
}

impl InstallOperation {
    pub fn new(manifest: impl Into<Arc<Manifest>>, options: InstallOptions) -> Self {
        Self {
            manifest: manifest.into(),
            options,
        }
    }

    pub async fn run(&self, events: &mut (impl EventSink + ?Sized)) -> eyre::Result<()> {
        events.emit(InstallEvent::Started {
            total_files: self.manifest.entries.len() as u64,
        });
        self.validate_entries()?;
        self.manifest.validate_main_executable()?;
        if let Some(target) = &self.options.main_executable {
            self.manifest.validate_file_target(target)?;
        } else if self.options.desktop_shortcut
            || self.options.start_menu_entry
            || self.options.autostart
        {
            eyre::bail!(
                "A main executable is required for desktop shortcuts, Start Menu entries, or autostart"
            );
        }
        if let Some(scope) = self.options.add_to_path {
            let policy = self
                .manifest
                .core_config
                .path_scope
                .unwrap_or(PathScopePolicy::default());
            if !policy.allows(scope) {
                eyre::bail!(
                    "Selected PATH scope {scope:?} is not permitted by the package policy {policy:?}"
                );
            }
        }
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

    async fn install_files(&self, events: &mut (impl EventSink + ?Sized)) -> eyre::Result<()> {
        let total = self.manifest.entries.len() as u64;
        let mut completed: u64 = 0;
        for entry in self.manifest.entries.iter() {
            let target_path = self.options.destination.join(&entry.name);
            self.write_entry(&entry, &target_path).await?;
            completed += 1;
            events.emit(InstallEvent::InstallingFile {
                path: target_path,
                completed,
                total,
            });
        }
        return Ok(());
    }

    async fn write_entry(&self, entry: &ManifestEntry, path: &Path) -> eyre::Result<()> {
        let mut file = File::create(path).await?;
        file.write_all(&entry.data).await?;
        file.flush().await?;
        return Ok(());
    }

    async fn apply_optional_tasks(&self, events: &mut dyn EventSink) -> eyre::Result<()> {
        //TODO: "perform selected tasks asynchronously (use spawn_blocking for blocking Windows APIs) and emit ApplyingTask events"
        return Ok(());
    }
}

#[cfg(test)]
mod tests {
    use super::{EventSink, InstallEvent, InstallOperation, InstallOptions};
    use simp_core::{
        CoreConfig, InstallTarget, Manifest, ManifestEntry, PathScope, PathScopePolicy,
    };
    use std::path::Path;

    fn operation_with_entries(names: &[&str]) -> InstallOperation {
        InstallOperation::new(
            Manifest {
                core_config: CoreConfig {
                    name: "Example".into(),
                    version: "1.0".into(),
                    description: None,
                    eula: None,
                    main_executable: None,
                    auto_start: None,
                    desktop_icon: None,
                    start_menu: None,
                    add_to_path: None,
                    path_scope: None,
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
                main_executable: None,
                desktop_shortcut: false,
                start_menu_entry: false,
                autostart: false,
                add_to_path: None,
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

    #[tokio::test]
    async fn invalid_install_options_fail_before_creating_the_destination() {
        struct IgnoreEvents;
        impl EventSink for IgnoreEvents {
            fn emit(&mut self, _: InstallEvent) {}
        }

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let destination = std::env::temp_dir().join(format!(
            "simp-launch-target-test-{}-{timestamp}",
            std::process::id()
        ));
        assert!(!destination.exists());
        for target in ["", "../app.exe", "missing.exe", "C:/app.exe"] {
            let mut operation = operation_with_entries(&["helper.exe", "app.exe"]);
            operation.options.destination = destination.clone();
            operation.options.main_executable = Some(target.into());
            assert!(operation.run(&mut IgnoreEvents).await.is_err(), "{target}");
            assert!(!destination.exists(), "{target}");
        }

        for (desktop_shortcut, start_menu_entry, autostart) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let mut operation = operation_with_entries(&["helper.exe", "app.exe"]);
            operation.options.destination = destination.clone();
            operation.options.desktop_shortcut = desktop_shortcut;
            operation.options.start_menu_entry = start_menu_entry;
            operation.options.autostart = autostart;
            let error = operation.run(&mut IgnoreEvents).await.unwrap_err();
            assert!(error.to_string().contains("A main executable is required"));
            assert!(!destination.exists());
        }

        for (policy, forbidden_scope) in [
            (None, PathScope::System),
            (Some(PathScopePolicy::User), PathScope::System),
            (Some(PathScopePolicy::System), PathScope::User),
        ] {
            let mut operation = operation_with_entries(&[]);
            std::sync::Arc::get_mut(&mut operation.manifest)
                .unwrap()
                .core_config
                .path_scope = policy;
            operation.options.destination = destination.clone();
            operation.options.add_to_path = Some(forbidden_scope);
            let error = operation.run(&mut IgnoreEvents).await.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("not permitted by the package policy")
            );
            assert!(!destination.exists());
        }
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
