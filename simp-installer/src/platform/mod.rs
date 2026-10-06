#[cfg(not(windows))]
use std::path::Path;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::{add_to_path, check_path_access, create_shortcut, validate_path_entry};

#[derive(Clone, Copy)]
pub(crate) enum ShortcutLocation {
    Desktop,
    StartMenu,
    Startup,
}

pub(crate) fn validate_shortcut_name(name: &str) -> eyre::Result<()> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2"
                        | "3"
                        | "4"
                        | "5"
                        | "6"
                        | "7"
                        | "8"
                        | "9"
                        | "\u{b9}"
                        | "\u{b2}"
                        | "\u{b3}"
                )
            })
        });
    if name.is_empty()
        || name.ends_with([' ', '.'])
        || name.encode_utf16().count() > 251
        || reserved
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        eyre::bail!("Application name {name:?} cannot be used as a Windows shortcut filename");
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn create_shortcut(
    _: ShortcutLocation,
    _: &str,
    _: &Path,
    _: &Path,
) -> eyre::Result<()> {
    eyre::bail!("Optional installation tasks are only supported on Windows")
}

#[cfg(not(windows))]
pub(crate) fn check_path_access(_: simp_core::PathScope, _: &Path) -> eyre::Result<()> {
    eyre::bail!("Optional installation tasks are only supported on Windows")
}

#[cfg(not(windows))]
pub(crate) fn validate_path_entry(_: &Path) -> eyre::Result<()> {
    eyre::bail!("Optional installation tasks are only supported on Windows")
}

#[cfg(not(windows))]
pub(crate) fn add_to_path(_: simp_core::PathScope, _: &Path) -> eyre::Result<()> {
    eyre::bail!("Optional installation tasks are only supported on Windows")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_names_cannot_escape_the_known_folder_or_use_reserved_names() {
        for name in [
            "",
            "../Other",
            "C:\\Other",
            "App/Other",
            "App:Other",
            "App.",
            "App ",
            "CON",
            "nul.txt",
            "COM1",
            "LPT9",
            "App\0",
        ] {
            assert!(validate_shortcut_name(name).is_err(), "{name:?}");
        }
        assert!(validate_shortcut_name(&"x".repeat(252)).is_err());
        for name in [
            "Example Application",
            "COM10",
            "App.exe",
            "\u{65e5}\u{672c}\u{8a9e}",
        ] {
            assert!(validate_shortcut_name(name).is_ok(), "{name:?}");
        }
    }
}
