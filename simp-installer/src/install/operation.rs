use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use eyre::WrapErr;
use simp_core::{PackageEntry, PathScope, PathScopePolicy};
use tokio::{
    fs::{File, create_dir_all},
    io::{AsyncWriteExt, BufWriter},
};

use super::InstallerPackage;
use crate::platform::{self, ShortcutLocation};

/// Progress reported by the installation operation, independent of the UI.
#[derive(Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    package: Arc<InstallerPackage>,
    options: InstallOptions,
}

impl InstallOperation {
    pub fn new(package: impl Into<Arc<InstallerPackage>>, options: InstallOptions) -> Self {
        Self {
            package: package.into(),
            options,
        }
    }

    pub async fn run(&self, events: &mut (impl EventSink + ?Sized)) -> eyre::Result<()> {
        events.emit(InstallEvent::Started {
            total_files: self.package.manifest.entries.len() as u64,
        });
        self.validate_entries()?;
        self.package.manifest.validate_main_executable()?;
        if let Some(target) = &self.options.main_executable {
            self.package.manifest.validate_file_target(target)?;
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
                .package
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
        self.prepare_optional_tasks().await?;
        self.create_destination().await?;
        self.install_files(events).await?;
        self.apply_optional_tasks(events).await?;
        events.emit(InstallEvent::Finished);
        return Ok(());
    }

    fn validate_entries(&self) -> eyre::Result<()> {
        let mut entries_set: HashSet<&String> = HashSet::new();
        for entry in self.package.manifest.entries.iter() {
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
        create_dir_all(&self.options.destination).await?;
        return Ok(());
    }

    async fn install_files(&self, events: &mut (impl EventSink + ?Sized)) -> eyre::Result<()> {
        let total = self.package.manifest.entries.len() as u64;
        let mut source = File::open(&self.package.path)
            .await
            .wrap_err("Failed to open installer payload source")?;
        let mut completed: u64 = 0;
        for entry in self.package.manifest.entries.iter() {
            let target_path = self.options.destination.join(&entry.name);
            self.write_entry(&mut source, entry, &target_path).await?;
            completed += 1;
            events.emit(InstallEvent::InstallingFile {
                path: target_path,
                completed,
                total,
            });
        }
        return Ok(());
    }

    async fn write_entry(
        &self,
        source: &mut File,
        entry: &PackageEntry,
        path: &Path,
    ) -> eyre::Result<()> {
        if tokio::fs::try_exists(path).await?
            && tokio::fs::canonicalize(path).await? == self.package.path
        {
            eyre::bail!("Installation would overwrite its own payload source");
        }
        let mut file = BufWriter::new(File::create(path).await?);
        entry
            .extract(source, &mut file)
            .await
            .wrap_err_with(|| format!("Failed to extract {}", entry.name))?;
        file.flush().await?;
        return Ok(());
    }

    async fn prepare_optional_tasks(&self) -> eyre::Result<()> {
        if self.options.desktop_shortcut || self.options.start_menu_entry || self.options.autostart
        {
            platform::validate_shortcut_name(&self.package.manifest.core_config.name)?;
            if !self.options.destination.is_absolute() {
                eyre::bail!("Shortcut installation directory must be an absolute path");
            }
            if !cfg!(windows) {
                eyre::bail!("Optional installation tasks are only supported on Windows");
            }
        }
        if let Some(scope) = self.options.add_to_path {
            platform::validate_path_entry(&self.options.destination)?;
            let directory = self.options.destination.clone();
            tokio::task::spawn_blocking(move || platform::check_path_access(scope, &directory))
                .await
                .wrap_err("PATH permission check worker failed")??;
        }
        Ok(())
    }

    async fn apply_optional_tasks(
        &self,
        events: &mut (impl EventSink + ?Sized),
    ) -> eyre::Result<()> {
        if let Some(exe_path) = &self.options.main_executable {
            if self.options.autostart {
                self.create_autostart(exe_path, events).await?;
            }

            if self.options.desktop_shortcut {
                self.create_desktop_shortcut(exe_path, events).await?;
            }

            if self.options.start_menu_entry {
                self.create_start_menu_entry(exe_path, events).await?;
            }
        }

        if let Some(scope) = self.options.add_to_path {
            self.add_to_path(scope, events).await?;
        }

        return Ok(());
    }

    async fn create_desktop_shortcut(
        &self,
        exe_path: &Path,
        events: &mut (impl EventSink + ?Sized),
    ) -> eyre::Result<()> {
        self.create_shortcut(
            InstallTask::DesktopShortcut,
            ShortcutLocation::Desktop,
            exe_path,
            events,
        )
        .await
    }

    async fn create_start_menu_entry(
        &self,
        exe_path: &Path,
        events: &mut (impl EventSink + ?Sized),
    ) -> eyre::Result<()> {
        self.create_shortcut(
            InstallTask::StartMenuEntry,
            ShortcutLocation::StartMenu,
            exe_path,
            events,
        )
        .await
    }

    async fn create_autostart(
        &self,
        exe_path: &Path,
        events: &mut (impl EventSink + ?Sized),
    ) -> eyre::Result<()> {
        self.create_shortcut(
            InstallTask::Autostart,
            ShortcutLocation::Startup,
            exe_path,
            events,
        )
        .await
    }

    async fn create_shortcut(
        &self,
        task: InstallTask,
        location: ShortcutLocation,
        exe_path: &Path,
        events: &mut (impl EventSink + ?Sized),
    ) -> eyre::Result<()> {
        events.emit(InstallEvent::ApplyingTask { task });
        let name = self.package.manifest.core_config.name.clone();
        let target = self.options.destination.join(exe_path);
        let directory = self.options.destination.clone();
        tokio::task::spawn_blocking(move || {
            platform::create_shortcut(location, &name, &target, &directory)
        })
        .await
        .wrap_err("Shortcut worker failed")?
        .wrap_err_with(|| format!("Could not apply {task:?}"))
    }

    async fn add_to_path(
        &self,
        path_scope: PathScope,
        events: &mut (impl EventSink + ?Sized),
    ) -> eyre::Result<()> {
        events.emit(InstallEvent::ApplyingTask {
            task: InstallTask::AddToPath,
        });
        let directory = self.options.destination.clone();
        tokio::task::spawn_blocking(move || platform::add_to_path(path_scope, &directory))
            .await
            .wrap_err("PATH update worker failed")?
            .wrap_err("Could not add the installation directory to PATH")
    }
}

#[cfg(test)]
mod tests {
    use super::{EventSink, InstallEvent, InstallOperation, InstallOptions};
    use simp_core::{
        CoreConfig, InstallTarget, PackageEntry, PackageManifest, PathScope, PathScopePolicy,
    };
    use std::path::Path;

    fn operation_with_entries(names: &[&str]) -> InstallOperation {
        InstallOperation::new(
            super::InstallerPackage {
                path: "unused-package.exe".into(),
                manifest: PackageManifest {
                    payload_offset: 0,
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
                        .map(|name| PackageEntry {
                            name: (*name).into(),
                            offset: 0,
                            compressed_size: 0,
                            uncompressed_size: 0,
                        })
                        .collect(),
                },
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

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "simp-stream-install-test-{}-{timestamp}",
                std::process::id()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if self.0.parent() == Some(std::env::temp_dir().as_path())
                && self.0.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .starts_with("simp-stream-install-test-")
                })
            {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    async fn package_with_files(directory: &TestDirectory) -> super::InstallerPackage {
        use tokio::io::AsyncWriteExt;
        let source_directory = directory.0.join("sources");
        tokio::fs::create_dir(&source_directory).await.unwrap();
        tokio::fs::write(source_directory.join("empty.txt"), [])
            .await
            .unwrap();
        tokio::fs::write(source_directory.join("app.cmd"), b"echo hello\r\n")
            .await
            .unwrap();
        let mut data = tokio::fs::File::create(source_directory.join("data.bin"))
            .await
            .unwrap();
        let block = vec![42u8; 64 * 1024];
        for _ in 0..16 {
            data.write_all(&block).await.unwrap();
        }
        data.flush().await.unwrap();
        drop(data);
        let source = simp_core::SourceManifest {
            core_config: operation_with_entries(&[])
                .package
                .manifest
                .core_config
                .clone(),
            entries: ["empty.txt", "data.bin", "app.cmd"]
                .into_iter()
                .map(|name| simp_core::SourceEntry::new(&source_directory.join(name)).unwrap())
                .collect(),
        };
        let path = directory.0.join("package.bin");
        let mut output = tokio::fs::File::create(&path).await.unwrap();
        output.write_all(b"fake bootstrap").await.unwrap();
        let manifest_offset = source
            .encode(&mut output, b"fake bootstrap".len() as u64)
            .await
            .unwrap();
        output
            .write_all(&<[u8; simp_core::Footer::SIZE]>::from(simp_core::Footer {
                version: simp_core::Footer::FORMAT_VERSION,
                manifest_offset,
            }))
            .await
            .unwrap();
        output.flush().await.unwrap();
        drop(output);
        crate::install::parse_installer(path).await.unwrap()
    }

    struct RecordedEvents(Vec<InstallEvent>);
    impl EventSink for RecordedEvents {
        fn emit(&mut self, event: InstallEvent) {
            self.0.push(event);
        }
    }

    fn operation_for_package(
        package: super::InstallerPackage,
        destination: std::path::PathBuf,
    ) -> InstallOperation {
        let mut options = operation_with_entries(&[]).options;
        options.destination = destination;
        InstallOperation::new(package, options)
    }

    #[tokio::test]
    async fn streams_package_files_to_disk_and_reports_completed_files() {
        let directory = TestDirectory::new();
        let package = package_with_files(&directory).await;
        let destination = directory.0.join("installed");
        let mut events = RecordedEvents(Vec::new());
        operation_for_package(package, destination.clone())
            .run(&mut events)
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::metadata(destination.join("empty.txt"))
                .await
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            tokio::fs::read(destination.join("app.cmd")).await.unwrap(),
            b"echo hello\r\n"
        );
        let data = tokio::fs::read(destination.join("data.bin")).await.unwrap();
        assert_eq!(data.len(), 1024 * 1024);
        assert!(data.iter().all(|byte| *byte == 42));
        assert_eq!(
            events.0.first(),
            Some(&InstallEvent::Started { total_files: 3 })
        );
        assert_eq!(events.0.last(), Some(&InstallEvent::Finished));
        let completed: Vec<_> = events
            .0
            .iter()
            .filter_map(|event| match event {
                InstallEvent::InstallingFile {
                    completed, total, ..
                } => Some((*completed, *total)),
                _ => None,
            })
            .collect();
        assert_eq!(completed, [(1, 3), (2, 3), (3, 3)]);
    }

    #[tokio::test]
    async fn corrupted_payload_stops_installation_before_reporting_file_success() {
        use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
        let directory = TestDirectory::new();
        let package = package_with_files(&directory).await;
        let entry = &package.manifest.entries[0];
        let mut file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&package.path)
            .await
            .unwrap();
        file.seek(SeekFrom::Start(entry.offset + entry.compressed_size - 1))
            .await
            .unwrap();
        let mut checksum = [0u8; 1];
        file.read_exact(&mut checksum).await.unwrap();
        checksum[0] ^= 1;
        file.seek(SeekFrom::Start(entry.offset + entry.compressed_size - 1))
            .await
            .unwrap();
        file.write_all(&checksum).await.unwrap();
        file.flush().await.unwrap();
        drop(file);
        let destination = directory.0.join("installed");
        let mut events = RecordedEvents(Vec::new());
        assert!(
            operation_for_package(package, destination.clone())
                .run(&mut events)
                .await
                .is_err()
        );
        assert_eq!(events.0, [InstallEvent::Started { total_files: 3 }]);
        assert!(!destination.join("data.bin").exists());
    }

    #[tokio::test]
    async fn rejects_installation_overwriting_its_payload_source() {
        let directory = TestDirectory::new();
        let mut package = package_with_files(&directory).await;
        let collision = directory.0.join("empty.txt");
        tokio::fs::rename(&package.path, &collision).await.unwrap();
        package.path = tokio::fs::canonicalize(collision).await.unwrap();
        let before = tokio::fs::read(&package.path).await.unwrap();
        let source_path = package.path.clone();
        let mut events = RecordedEvents(Vec::new());
        let error = operation_for_package(package, directory.0.clone())
            .run(&mut events)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("overwrite its own payload source")
        );
        assert_eq!(tokio::fs::read(source_path).await.unwrap(), before);
        assert_eq!(events.0, [InstallEvent::Started { total_files: 3 }]);
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
            std::sync::Arc::get_mut(&mut operation.package)
                .unwrap()
                .manifest
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

    #[tokio::test]
    async fn shortcut_errors_emit_the_selected_task_and_stop_later_tasks() {
        struct Events(Vec<InstallEvent>);
        impl EventSink for Events {
            fn emit(&mut self, event: InstallEvent) {
                self.0.push(event);
            }
        }
        for task in [
            super::InstallTask::Autostart,
            super::InstallTask::DesktopShortcut,
            super::InstallTask::StartMenuEntry,
        ] {
            let mut operation = operation_with_entries(&[]);
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            operation.options.destination = std::env::temp_dir().join(format!(
                "simp-missing-shortcut-target-{}-{timestamp}",
                std::process::id()
            ));
            assert!(!operation.options.destination.exists());
            operation.options.main_executable = Some("missing.exe".into());
            operation.options.autostart = task == super::InstallTask::Autostart;
            operation.options.desktop_shortcut = task != super::InstallTask::StartMenuEntry;
            operation.options.start_menu_entry = true;
            let mut events = Events(Vec::new());
            assert!(operation.apply_optional_tasks(&mut events).await.is_err());
            assert_eq!(events.0, vec![InstallEvent::ApplyingTask { task }]);
        }
    }

    #[tokio::test]
    async fn invalid_shortcut_names_and_path_entries_fail_before_file_installation() {
        struct IgnoreEvents;
        impl EventSink for IgnoreEvents {
            fn emit(&mut self, _: InstallEvent) {}
        }
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let destination = std::env::temp_dir().join(format!(
            "simp-invalid-tasks-{}-{timestamp};bad",
            std::process::id()
        ));
        let mut operation = operation_with_entries(&["app.exe"]);
        operation.options.destination = destination.clone();
        operation.options.add_to_path = Some(PathScope::User);
        assert!(operation.run(&mut IgnoreEvents).await.is_err());
        assert!(!destination.exists());
        operation.options.add_to_path = None;
        operation.options.main_executable = Some("app.exe".into());
        operation.options.desktop_shortcut = true;
        std::sync::Arc::get_mut(&mut operation.package)
            .unwrap()
            .manifest
            .core_config
            .name = "../Other App".into();
        assert!(operation.run(&mut IgnoreEvents).await.is_err());
        assert!(!destination.exists());
    }
}
