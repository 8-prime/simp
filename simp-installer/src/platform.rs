use std::path::PathBuf;

use simp_core::InstallTarget;

// Resolve well-known Windows locations at runtime, on the installing machine.
pub(crate) fn initial_destination(target: &InstallTarget, app_name: &str) -> PathBuf {
    match target {
        InstallTarget::Custom(path) => PathBuf::from(path),
        InstallTarget::ProgramFiles => std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .map(|base| base.join(app_name))
            .unwrap_or_default(),
        InstallTarget::AppData => std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|base| base.join("Programs").join(app_name))
            .unwrap_or_default(),
    }
}
