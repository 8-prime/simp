#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use simp_core::Manifest;

use crate::args::LaunchMode;

mod args;
#[cfg(feature = "ui-preview")]
mod fixture;
mod install;
mod platform;
mod ui;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let installer = match args::parse(std::env::args_os().skip(1)) {
        Ok(mode) => load_manifest(&mode)
            .await
            .map(|manifest| (mode, manifest))
            .map_err(|error| format!("{error:#}")),
        Err(message) => Err(message),
    };

    ui::run(installer);
    Ok(())
}

async fn load_manifest(mode: &LaunchMode) -> eyre::Result<Manifest> {
    match mode {
        LaunchMode::Runtime => install::parse_installer(std::env::current_exe()?).await,
        #[cfg(feature = "ui-preview")]
        LaunchMode::Preview { .. } => Ok(fixture::manifest()),
    }
}

#[cfg(all(test, feature = "ui-preview"))]
#[tokio::test]
async fn preview_loads_manifest_without_a_packaged_executable() {
    let manifest = load_manifest(&LaunchMode::Preview { fail: false })
        .await
        .unwrap();

    assert_eq!(manifest.core_config.name, "Example Application");
    assert_eq!(manifest.entries[0].name, "example.txt");
}
