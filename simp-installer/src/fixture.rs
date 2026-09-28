//! Manifest used only by the development UI preview.

use simp_core::{CoreConfig, InstallTarget, Manifest, ManifestEntry};

pub fn manifest() -> Manifest {
    Manifest {
        core_config: CoreConfig {
            name: "Example App".into(),
            version: "1.0.0".into(),
            description: Some("A small application to try your new installer.".into()),
            eula: None,
            auto_start: None,
            desktop_icon: None,
            start_menu: None,
            add_to_path: None,
            target: InstallTarget::Custom(
                r"C:\Users\You\AppData\Local\Programs\Example App".into(),
            ),
        },
        entries: vec![ManifestEntry {
            name: "example.txt".into(),
            data: b"Example file for the installer preview.\n".to_vec(),
        }],
    }
}
