//! Manifest used only by the development UI preview.

use crate::install::InstallerPackage;
use simp_core::{CoreConfig, InstallTarget, PackageEntry, PackageManifest, PathScopePolicy};

pub fn manifest() -> InstallerPackage {
    InstallerPackage { path: std::path::PathBuf::new(), manifest: PackageManifest {
        payload_offset: 0,
        core_config: CoreConfig {
            name: "Example Application".into(),
            version: "1.0.0".into(),
            main_executable: Some("example.exe".into()),
            description: Some("Example Application is a powerful and easy-to-use application that helps you be more productive.".into()),
            eula: Some("END USER LICENSE AGREEMENT\n\nThis is sample license text for the SIMP setup preview. It is provided only to demonstrate the license screen.\n\nRead the application publisher's license agreement before installing their software. A packaged application will display the agreement provided by its publisher here.\n\nBy selecting the acceptance option below, you can continue through this demonstration. No application files will be installed and no system settings will be changed.\n\nThe preview includes an installation folder, optional tasks, an installation summary, and simulated progress.\n\nYou can return to earlier screens to review your choices, or select Cancel to close the preview.".into()),
            auto_start: Some(true),
            desktop_icon: Some(true),
            start_menu: Some(true),
            add_to_path: Some(false),
            path_scope: Some(PathScopePolicy::UserChoice),
            target: InstallTarget::Custom(
                r"C:\Program Files\Example Application".into(),
            ),
        },
        entries: vec![PackageEntry {
            name: "example.txt".into(),
            offset: 0, compressed_size: 0,
            uncompressed_size: b"Example file for the installer preview.\n".len() as u64,
        }, PackageEntry {
            name: "example.exe".into(),
            offset: 0, compressed_size: 0, uncompressed_size: 0,
        }],
    } }
}
