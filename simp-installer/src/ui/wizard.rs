use std::path::PathBuf;

use simp_core::{Manifest, PathScope, PathScopePolicy};

use crate::install::operation::{InstallEvent, InstallOptions, InstallTask};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Page {
    Welcome,
    License,
    Location,
    Tasks,
    Ready,
    Installing,
    Finished,
}

impl Page {
    pub const ALL: [Self; 7] = [
        Self::Welcome,
        Self::License,
        Self::Location,
        Self::Tasks,
        Self::Ready,
        Self::Installing,
        Self::Finished,
    ];
}

// Presentation state only. Events and the operation result drive runtime progress.
pub(super) enum InstallerState {
    Choosing(Page),
    Installing { percent: u8, action: &'static str },
    Finished,
    Failed { message: String },
}

impl InstallerState {
    pub fn from_install_event(event: &InstallEvent) -> Self {
        let (percent, action) = match event {
            InstallEvent::Started { .. } => (0, "Preparing to install..."),
            InstallEvent::InstallingFile {
                completed, total, ..
            } => {
                let percent = if *total == 0 {
                    0
                } else {
                    (u128::from((*completed).min(*total)) * 100 / u128::from(*total)) as u8
                };
                (percent, "Installing files...")
            }
            InstallEvent::ApplyingTask { .. } => (100, "Applying selected tasks..."),
            // Await the worker result before reporting success to the user.
            InstallEvent::Finished => (100, "Finalizing installation..."),
        };
        Self::Installing { percent, action }
    }

    pub fn page(&self) -> Option<Page> {
        match self {
            Self::Choosing(page) => Some(*page),
            Self::Installing { .. } => Some(Page::Installing),
            Self::Finished => Some(Page::Finished),
            Self::Failed { .. } => None,
        }
    }
}

pub(super) struct OptionalTask {
    pub kind: InstallTask,
    pub label: &'static str,
    pub selected: bool,
}

pub(super) struct AppDetails {
    pub name: String,
    pub version: String,
    pub description: String,
    pub license: Option<String>,
    pub destination: PathBuf,
    pub main_executable: Option<PathBuf>,
    pub path_scope_policy: PathScopePolicy,
    pub path_scope: PathScope,
    pub required_bytes: u64,
    pub tasks: Vec<OptionalTask>,
}

impl AppDetails {
    pub fn from_manifest(manifest: &Manifest) -> Self {
        let config = &manifest.core_config;
        let path_scope_policy = config.path_scope.unwrap_or(PathScopePolicy::User);
        Self {
            name: config.name.clone(),
            version: config.version.clone(),
            description: config.description.clone().unwrap_or_default(),
            license: config.eula.clone().filter(|text| !text.trim().is_empty()),
            destination: config.target.resolve(&config.name),
            main_executable: config.main_executable.as_ref().map(PathBuf::from),
            path_scope_policy,
            path_scope: path_scope_policy.initial_scope(),
            required_bytes: manifest.entries.iter().fold(0_u64, |total, entry| {
                total.saturating_add(entry.data.len() as u64)
            }),
            tasks: [
                (
                    InstallTask::DesktopShortcut,
                    "Create a desktop shortcut",
                    config.desktop_icon,
                ),
                (
                    InstallTask::StartMenuEntry,
                    "Create a Start Menu entry",
                    config.start_menu,
                ),
                (
                    InstallTask::Autostart,
                    "Launch application on startup",
                    config.auto_start,
                ),
                (
                    InstallTask::AddToPath,
                    match path_scope_policy {
                        PathScopePolicy::User => "Add application to user PATH",
                        PathScopePolicy::System => "Add application to system PATH",
                        PathScopePolicy::UserChoice => "Add application to PATH",
                    },
                    config.add_to_path,
                ),
            ]
            .into_iter()
            .filter(|(kind, _, _)| {
                *kind == InstallTask::AddToPath || config.main_executable.is_some()
            })
            .filter_map(|(kind, label, selected)| {
                selected.map(|selected| OptionalTask {
                    kind,
                    label,
                    selected,
                })
            })
            .collect(),
        }
    }

    pub fn install_options(&self) -> InstallOptions {
        let selected = |kind| {
            self.tasks
                .iter()
                .any(|task| task.kind == kind && task.selected)
        };
        InstallOptions {
            destination: self.destination.clone(),
            main_executable: self.main_executable.clone(),
            desktop_shortcut: selected(InstallTask::DesktopShortcut),
            start_menu_entry: selected(InstallTask::StartMenuEntry),
            autostart: selected(InstallTask::Autostart),
            add_to_path: selected(InstallTask::AddToPath).then_some(self.path_scope),
        }
    }

    pub fn pages(&self) -> Vec<Page> {
        Page::ALL
            .into_iter()
            .filter(|page| match page {
                Page::License => self.license.is_some(),
                Page::Tasks => !self.tasks.is_empty(),
                _ => true,
            })
            .collect()
    }

    pub fn adjacent_page(&self, current: Page, forward: bool) -> Option<Page> {
        let pages = self.pages();
        let index = pages.iter().position(|page| *page == current)?;
        let next = if forward {
            index.checked_add(1)?
        } else {
            index.checked_sub(1)?
        };
        pages
            .get(next)
            .copied()
            .filter(|page| !matches!(page, Page::Installing | Page::Finished))
    }

    pub fn can_continue(&self, page: Page, license_accepted: bool) -> bool {
        match page {
            Page::License => license_accepted,
            Page::Location => self.destination.is_absolute(),
            Page::Welcome | Page::Tasks => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simp_core::{CoreConfig, InstallTarget};

    fn manifest() -> Manifest {
        Manifest {
            core_config: CoreConfig {
                name: "Example".into(),
                version: "1.0".into(),
                description: None,
                eula: Some("License text".into()),
                main_executable: Some("app.exe".into()),
                auto_start: Some(false),
                desktop_icon: Some(true),
                start_menu: Some(true),
                add_to_path: None,
                path_scope: None,
                target: InstallTarget::Custom(std::env::temp_dir().display().to_string()),
            },
            entries: ["helper.exe", "app.exe"]
                .into_iter()
                .map(|name| simp_core::ManifestEntry {
                    name: name.into(),
                    data: Vec::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn navigation_requires_consent_and_an_absolute_destination() {
        let mut details = AppDetails::from_manifest(&manifest());
        assert!(!details.can_continue(Page::License, false));
        assert!(details.can_continue(Page::License, true));
        assert!(details.can_continue(Page::Location, true));
        details.destination = PathBuf::from("relative/folder");
        assert!(!details.can_continue(Page::Location, true));
        details.destination = PathBuf::new();
        assert!(!details.can_continue(Page::Location, true));
        assert_eq!(details.adjacent_page(Page::Ready, true), None);
        assert_eq!(details.adjacent_page(Page::Welcome, false), None);
    }

    #[test]
    fn omitted_license_and_tasks_are_skipped_in_both_directions() {
        let mut manifest = manifest();
        manifest.core_config.eula = Some("  ".into());
        manifest.core_config.auto_start = None;
        manifest.core_config.desktop_icon = None;
        manifest.core_config.start_menu = None;
        let details = AppDetails::from_manifest(&manifest);
        assert_eq!(
            details.adjacent_page(Page::Welcome, true),
            Some(Page::Location)
        );
        assert_eq!(
            details.adjacent_page(Page::Location, false),
            Some(Page::Welcome)
        );
        assert_eq!(
            details.adjacent_page(Page::Location, true),
            Some(Page::Ready)
        );
        assert_eq!(
            details.adjacent_page(Page::Ready, false),
            Some(Page::Location)
        );
        assert_eq!(details.pages().len(), 5);
    }

    #[test]
    fn optional_tasks_preserve_configured_defaults_including_false() {
        let details = AppDetails::from_manifest(&manifest());
        assert_eq!(details.tasks.len(), 3);
        assert!(details.tasks[0].selected);
        assert!(details.tasks[1].selected);
        assert!(!details.tasks[2].selected);
        assert_eq!(details.pages(), Page::ALL);
    }

    #[test]
    fn install_options_share_a_launch_target_and_preserve_current_selections() {
        let mut details = AppDetails::from_manifest(&manifest());
        let options = details.install_options();
        assert_eq!(options.destination, details.destination);
        assert_eq!(options.main_executable, Some(PathBuf::from("app.exe")));
        assert!(options.desktop_shortcut);
        assert!(!options.autostart);
        assert!(options.start_menu_entry);
        assert!(options.add_to_path.is_none());

        details.tasks[0].selected = false;
        details.tasks[1].selected = false;
        details.tasks[2].selected = true;
        let options = details.install_options();
        assert_eq!(options.main_executable, Some(PathBuf::from("app.exe")));
        assert!(!options.desktop_shortcut);
        assert!(!options.start_menu_entry);
        assert!(options.autostart);
    }

    #[test]
    fn launch_tasks_are_unavailable_without_a_main_executable_but_path_is_available() {
        let mut manifest = manifest();
        manifest.core_config.main_executable = None;
        manifest.core_config.add_to_path = Some(true);
        let details = AppDetails::from_manifest(&manifest);
        assert_eq!(details.tasks.len(), 1);
        assert!(details.tasks[0].kind == InstallTask::AddToPath);
        let options = details.install_options();
        assert!(options.main_executable.is_none());
        assert!(!options.desktop_shortcut);
        assert!(!options.start_menu_entry);
        assert!(!options.autostart);
        assert_eq!(options.add_to_path, Some(PathScope::User));
    }

    #[test]
    fn path_scope_policy_sets_the_initial_scope_and_keeps_disabled_tasks_absent() {
        for (policy, expected) in [
            (None, PathScope::User),
            (Some(PathScopePolicy::User), PathScope::User),
            (Some(PathScopePolicy::System), PathScope::System),
            (Some(PathScopePolicy::UserChoice), PathScope::User),
        ] {
            let mut manifest = manifest();
            manifest.core_config.path_scope = policy;
            manifest.core_config.add_to_path = Some(true);
            let mut details = AppDetails::from_manifest(&manifest);
            assert_eq!(details.install_options().add_to_path, Some(expected));
            let task = details
                .tasks
                .iter_mut()
                .find(|task| task.kind == InstallTask::AddToPath)
                .unwrap();
            task.selected = false;
            assert!(details.install_options().add_to_path.is_none());
        }
    }

    #[test]
    fn progress_handles_empty_totals_and_large_counts_without_reporting_success_early() {
        for (completed, total, expected) in [
            (1, 2, 50),
            (0, 0, 0),
            (3, 2, 100),
            (u64::MAX, u64::MAX, 100),
        ] {
            assert!(matches!(
                InstallerState::from_install_event(&InstallEvent::InstallingFile {
                    path: "app.exe".into(), completed, total,
                }),
                InstallerState::Installing { percent, .. } if percent == expected
            ));
        }
        assert!(matches!(
            InstallerState::from_install_event(&InstallEvent::Finished),
            InstallerState::Installing { percent: 100, .. }
        ));
    }
}
