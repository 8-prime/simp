use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use eyre::WrapErr;
use simp_core::PathScope;
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, HANDLE, LPARAM, WPARAM},
        Globalization::{CSTR_EQUAL, CompareStringOrdinal},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize, IPersistFile, STGM_READ,
            },
            Environment::ExpandEnvironmentStringsW,
            Registry::{
                HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE,
                REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
                RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
            },
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
        UI::{
            Shell::{
                FOLDERID_Desktop, FOLDERID_Programs, FOLDERID_Startup, IShellLinkW,
                KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath, SLGP_RAWPATH, ShellLink,
            },
            WindowsAndMessaging::{
                HWND_BROADCAST, SMTO_ABORTIFHUNG, SW_SHOWNORMAL, SendMessageTimeoutW,
                WM_SETTINGCHANGE,
            },
        },
    },
    core::{GUID, Interface, PCWSTR, w},
};

use super::{ShortcutLocation, validate_shortcut_name};

const MAX_PATH_UNITS: usize = 32_766;
const SEMICOLON: u16 = b';' as u16;
const PERCENT: u16 = b'%' as u16;
const SLASH: u16 = b'/' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const QUOTE: u16 = b'"' as u16;

struct ComApartment;

impl ComApartment {
    fn initialize() -> eyre::Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn wide_path(path: &Path) -> eyre::Result<Vec<u16>> {
    let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        eyre::bail!("Path contains a NUL character");
    }
    value.push(0);
    Ok(value)
}

fn known_folder(id: &GUID) -> eyre::Result<PathBuf> {
    unsafe {
        let value = SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None)?;
        let path = PathBuf::from(OsString::from_wide(value.as_wide()));
        CoTaskMemFree(Some(value.0.cast()));
        Ok(path)
    }
}

pub(crate) fn create_shortcut(
    location: ShortcutLocation,
    name: &str,
    target: &Path,
    working_directory: &Path,
) -> eyre::Result<()> {
    validate_shortcut_name(name)?;
    if !target.is_file() {
        eyre::bail!(
            "Shortcut target is not an installed file: {}",
            target.display()
        );
    }
    let folder = known_folder(match location {
        ShortcutLocation::Desktop => &FOLDERID_Desktop,
        ShortcutLocation::StartMenu => &FOLDERID_Programs,
        ShortcutLocation::Startup => &FOLDERID_Startup,
    })
    .wrap_err("Could not resolve the current user's shortcut folder")?;
    std::fs::create_dir_all(&folder)?;
    save_shortcut(
        &folder.join(format!("{name}.lnk")),
        target,
        working_directory,
    )
}

fn save_shortcut(shortcut: &Path, target: &Path, working_directory: &Path) -> eyre::Result<()> {
    if !target.is_absolute() || !working_directory.is_absolute() {
        eyre::bail!("Shortcut target and working directory must be absolute paths");
    }
    let shortcut_wide = wide_path(shortcut)?;
    let target_wide = wide_path(target)?;
    let working_wide = wide_path(working_directory)?;
    let _apartment = ComApartment::initialize()?;
    // COM interfaces stay on this thread and are released before the apartment guard.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let file: IPersistFile = link.cast()?;
        if shortcut.try_exists()? {
            file.Load(PCWSTR(shortcut_wide.as_ptr()), STGM_READ)
                .wrap_err_with(|| {
                    format!("Existing shortcut cannot be read: {}", shortcut.display())
                })?;
            let mut existing_target = vec![0; MAX_PATH_UNITS + 1];
            link.GetPath(
                &mut existing_target,
                std::ptr::null_mut(),
                SLGP_RAWPATH.0 as u32,
            )?;
            existing_target.truncate(
                existing_target
                    .iter()
                    .position(|unit| *unit == 0)
                    .unwrap_or(existing_target.len()),
            );
            if equal_paths(&existing_target, &target_wide[..target_wide.len() - 1]) {
                return Ok(());
            }
            eyre::bail!(
                "Refusing to replace an unrelated shortcut: {}",
                shortcut.display()
            );
        }
        link.SetPath(PCWSTR(target_wide.as_ptr()))?;
        link.SetWorkingDirectory(PCWSTR(working_wide.as_ptr()))?;
        link.SetShowCmd(SW_SHOWNORMAL)?;
        file.Save(PCWSTR(shortcut_wide.as_ptr()), true)
            .wrap_err_with(|| format!("Could not save shortcut: {}", shortcut.display()))?;
    }
    Ok(())
}

struct RegistryKey(HKEY);

impl RegistryKey {
    fn open(root: HKEY, subkey: PCWSTR, access: REG_SAM_FLAGS) -> eyre::Result<Option<Self>> {
        let mut key = HKEY::default();
        let status = unsafe { RegOpenKeyExW(root, subkey, None, access, &mut key) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        status.ok()?;
        Ok(Some(Self(key)))
    }

    fn create(root: HKEY, subkey: PCWSTR) -> eyre::Result<Self> {
        let mut key = HKEY::default();
        unsafe {
            RegCreateKeyExW(
                root,
                subkey,
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                None,
                &mut key,
                None,
            )
            .ok()?;
        }
        Ok(Self(key))
    }
}

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn environment_key(scope: PathScope) -> (HKEY, PCWSTR) {
    match scope {
        PathScope::User => (HKEY_CURRENT_USER, w!("Environment")),
        PathScope::System => (
            HKEY_LOCAL_MACHINE,
            w!("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment"),
        ),
    }
}

fn is_elevated() -> eyre::Result<bool> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0;
        let result = GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        let _ = CloseHandle(token);
        result?;
        Ok(elevation.TokenIsElevated != 0)
    }
}

pub(crate) fn check_path_access(scope: PathScope, directory: &Path) -> eyre::Result<()> {
    validate_path_entry(directory)?;
    if scope == PathScope::System && !is_elevated()? {
        eyre::bail!(
            "Updating system PATH requires administrator privileges. Restart setup using Run as administrator, or select user PATH."
        );
    }
    let (root, subkey) = environment_key(scope);
    let existing = if let Some(key) =
        RegistryKey::open(root, subkey, KEY_QUERY_VALUE | KEY_SET_VALUE)
            .wrap_err("Cannot open the selected PATH registry key for writing")?
    {
        read_path(&key)?
    } else if scope == PathScope::System {
        eyre::bail!("The system environment registry key is missing");
    } else {
        None
    };
    let (kind, value) = existing.unwrap_or((REG_EXPAND_SZ, Vec::new()));
    let directory: Vec<_> = directory.as_os_str().encode_wide().collect();
    append_path(&value, &directory, kind == REG_EXPAND_SZ)?;
    Ok(())
}

pub(crate) fn validate_path_entry(path: &Path) -> eyre::Result<()> {
    if !path.is_absolute()
        || path
            .as_os_str()
            .encode_wide()
            .any(|unit| matches!(unit, 0 | PERCENT | SEMICOLON))
    {
        eyre::bail!(
            "PATH entries must be absolute directory paths without NUL, '%' or ';' characters"
        );
    }
    if path
        .as_os_str()
        .encode_wide()
        .take(MAX_PATH_UNITS + 1)
        .count()
        > MAX_PATH_UNITS
    {
        eyre::bail!("Installation directory is too long for a Windows PATH entry");
    }
    Ok(())
}

fn read_path(key: &RegistryKey) -> eyre::Result<Option<(REG_VALUE_TYPE, Vec<u16>)>> {
    // Query twice because registry strings need not be terminated and can change size.
    for _ in 0..4 {
        let mut kind = REG_VALUE_TYPE::default();
        let mut size = 0;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                w!("Path"),
                None,
                Some(&mut kind),
                None,
                Some(&mut size),
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        status.ok()?;
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            eyre::bail!("Existing PATH registry value is neither REG_SZ nor REG_EXPAND_SZ");
        }
        if size as usize > (MAX_PATH_UNITS + 1) * 2 {
            eyre::bail!("Existing PATH registry value is too large");
        }
        let mut bytes = vec![0_u8; size as usize];
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                w!("Path"),
                None,
                Some(&mut kind),
                Some(bytes.as_mut_ptr()),
                Some(&mut size),
            )
        };
        if status == ERROR_MORE_DATA || status == ERROR_FILE_NOT_FOUND {
            continue;
        }
        status.ok()?;
        bytes.truncate(size as usize);
        if (kind != REG_SZ && kind != REG_EXPAND_SZ) || !bytes.len().is_multiple_of(2) {
            eyre::bail!("Existing PATH registry value is not a valid Windows string");
        }
        let mut value: Vec<_> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        if let Some(end) = value.iter().position(|unit| *unit == 0) {
            if value[end..].iter().any(|unit| *unit != 0) {
                eyre::bail!("Existing PATH contains an embedded NUL");
            }
            value.truncate(end);
        }
        return Ok(Some((kind, value)));
    }
    eyre::bail!("PATH registry value kept changing while it was being read")
}

fn normalized_path(value: &[u16]) -> Vec<u16> {
    let value = if value.len() >= 2 && value.first() == Some(&QUOTE) && value.last() == Some(&QUOTE)
    {
        &value[1..value.len() - 1]
    } else {
        value
    };
    let mut value: Vec<_> = value
        .iter()
        .map(|unit| if *unit == SLASH { BACKSLASH } else { *unit })
        .collect();
    while value.len() > 3 && value.last() == Some(&BACKSLASH) {
        value.pop();
    }
    value
}

fn equal_paths(first: &[u16], second: &[u16]) -> bool {
    unsafe {
        CompareStringOrdinal(&normalized_path(first), &normalized_path(second), true) == CSTR_EQUAL
    }
}

fn expanded_path(value: &[u16]) -> eyre::Result<Vec<u16>> {
    let source: Vec<_> = value.iter().copied().chain(std::iter::once(0)).collect();
    let mut size = unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), None) };
    for _ in 0..4 {
        if size == 0 {
            return Err(windows::core::Error::from_win32().into());
        }
        if size as usize > MAX_PATH_UNITS + 1 {
            eyre::bail!("Expanded PATH entry is too large");
        }
        let mut value = vec![0; size as usize];
        let written =
            unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), Some(&mut value)) };
        if written > size {
            size = written;
            continue;
        }
        if written == 0 {
            return Err(windows::core::Error::from_win32().into());
        }
        value.truncate(written as usize - 1);
        return Ok(value);
    }
    eyre::bail!("Environment kept changing while expanding PATH")
}

fn append_path(
    existing: &[u16],
    directory: &[u16],
    expand: bool,
) -> eyre::Result<Option<Vec<u16>>> {
    for entry in existing
        .split(|unit| *unit == SEMICOLON)
        .filter(|entry| !entry.is_empty())
    {
        let expanded = if expand {
            expanded_path(entry)?
        } else {
            entry.to_vec()
        };
        if equal_paths(&expanded, directory) {
            return Ok(None);
        }
    }
    let mut value = existing.to_vec();
    if !value.is_empty() && value.last() != Some(&SEMICOLON) {
        value.push(SEMICOLON);
    }
    value.extend_from_slice(directory);
    if value.len() > MAX_PATH_UNITS {
        eyre::bail!("Adding the installation directory would make PATH too long");
    }
    Ok(Some(value))
}

fn update_path(key: &RegistryKey, directory: &Path) -> eyre::Result<bool> {
    validate_path_entry(directory)?;
    let (kind, existing) = read_path(key)?.unwrap_or((REG_EXPAND_SZ, Vec::new()));
    let directory: Vec<_> = directory.as_os_str().encode_wide().collect();
    let Some(value) = append_path(&existing, &directory, kind == REG_EXPAND_SZ)? else {
        return Ok(false);
    };
    let bytes: Vec<_> = value
        .into_iter()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    unsafe {
        RegSetValueExW(key.0, w!("Path"), None, kind, Some(&bytes)).ok()?;
    }
    Ok(true)
}

pub(crate) fn add_to_path(scope: PathScope, directory: &Path) -> eyre::Result<()> {
    check_path_access(scope, directory)?;
    let (root, subkey) = environment_key(scope);
    let key = RegistryKey::create(root, subkey)?;
    if update_path(&key, directory)? {
        // Notification is best-effort: an unresponsive application must not hang setup.
        unsafe {
            // Tell windows that env vars have changed
            let _ = SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(w!("Environment").as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                1000,
                None,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };
    use windows::Win32::System::Registry::{REG_DWORD, RegDeleteTreeW};

    static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

    fn test_id() -> String {
        format!(
            "{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_TEST.fetch_add(1, Ordering::Relaxed)
        )
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("simp-shortcut-{} with spaces", test_id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct TestRegistry {
        key: Option<RegistryKey>,
        subkey: Vec<u16>,
    }

    impl TestRegistry {
        fn new() -> Self {
            let subkey: Vec<_> = format!("Software\\SIMP\\Tests\\{}", test_id())
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let key = RegistryKey::create(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr())).unwrap();
            Self {
                key: Some(key),
                subkey,
            }
        }

        fn key(&self) -> &RegistryKey {
            self.key.as_ref().unwrap()
        }

        fn set(&self, kind: REG_VALUE_TYPE, value: &str) {
            let bytes: Vec<_> = value
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(u16::to_le_bytes)
                .collect();
            self.set_bytes(kind, &bytes);
        }

        fn set_bytes(&self, kind: REG_VALUE_TYPE, bytes: &[u8]) {
            unsafe {
                RegSetValueExW(self.key().0, w!("Path"), None, kind, Some(bytes))
                    .ok()
                    .unwrap();
            }
        }
    }

    impl Drop for TestRegistry {
        fn drop(&mut self) {
            self.key.take();
            unsafe {
                let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(self.subkey.as_ptr()));
            }
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().collect()
    }

    #[test]
    fn shell_link_round_trips_spaces_and_preserves_existing_shortcuts() {
        let directory = TestDirectory::new();
        let target = directory.0.join("my application.exe");
        std::fs::write(&target, []).unwrap();
        let shortcut = directory.0.join("My Application.lnk");
        save_shortcut(&shortcut, &target, &directory.0).unwrap();
        let original = std::fs::read(&shortcut).unwrap();
        assert!(!original.is_empty());
        {
            let _apartment = ComApartment::initialize().unwrap();
            unsafe {
                let link: IShellLinkW =
                    CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
                let persist: IPersistFile = link.cast().unwrap();
                let filename = wide_path(&shortcut).unwrap();
                persist.Load(PCWSTR(filename.as_ptr()), STGM_READ).unwrap();
                let mut buffer = vec![0; MAX_PATH_UNITS + 1];
                link.GetPath(&mut buffer, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
                    .unwrap();
                let end = buffer.iter().position(|unit| *unit == 0).unwrap();
                assert_eq!(PathBuf::from(OsString::from_wide(&buffer[..end])), target);
                link.GetWorkingDirectory(&mut buffer).unwrap();
                let end = buffer.iter().position(|unit| *unit == 0).unwrap();
                assert_eq!(
                    PathBuf::from(OsString::from_wide(&buffer[..end])),
                    directory.0
                );
                link.GetArguments(&mut buffer).unwrap();
                assert_eq!(buffer[0], 0);
            }
        }
        save_shortcut(&shortcut, &target, &directory.0).unwrap();
        assert_eq!(std::fs::read(&shortcut).unwrap(), original);
        assert!(save_shortcut(&shortcut, &directory.0.join("other.exe"), &directory.0).is_err());
        assert_eq!(std::fs::read(&shortcut).unwrap(), original);
    }

    #[test]
    fn shortcut_locations_use_current_user_known_folders() {
        for id in [&FOLDERID_Desktop, &FOLDERID_Programs, &FOLDERID_Startup] {
            assert!(known_folder(id).unwrap().is_absolute());
        }
    }

    #[test]
    fn path_updates_preserve_existing_entries_and_registry_type() {
        let registry = TestRegistry::new();
        let directory = Path::new(r"C:\Program Files\SIMP");
        assert!(update_path(registry.key(), directory).unwrap());
        assert_eq!(
            read_path(registry.key()).unwrap(),
            Some((REG_EXPAND_SZ, wide(r"C:\Program Files\SIMP")))
        );
        for kind in [REG_SZ, REG_EXPAND_SZ] {
            registry.set(kind, r"%SystemRoot%\System32;C:\Existing;;");
            assert!(update_path(registry.key(), directory).unwrap());
            assert_eq!(
                read_path(registry.key()).unwrap(),
                Some((
                    kind,
                    wide(r"%SystemRoot%\System32;C:\Existing;;C:\Program Files\SIMP")
                ))
            );
            assert!(!update_path(registry.key(), Path::new("c:/program files/simp/")).unwrap());
        }
        registry.set(REG_SZ, "");
        assert!(update_path(registry.key(), directory).unwrap());
        assert_eq!(
            read_path(registry.key()).unwrap(),
            Some((REG_SZ, wide(r"C:\Program Files\SIMP")))
        );
    }

    #[test]
    fn duplicate_detection_handles_expansion_case_quotes_and_trailing_separators() {
        let root = std::env::var_os("SystemRoot").unwrap();
        let root: Vec<_> = root.encode_wide().collect();
        assert!(
            append_path(&wide("%SystemRoot%"), &root, true)
                .unwrap()
                .is_none()
        );
        assert!(
            append_path(&wide("%SystemRoot%"), &root, false)
                .unwrap()
                .is_some()
        );
        assert!(
            append_path(&wide("\"C:\\Example\\\""), &wide("c:/example"), false)
                .unwrap()
                .is_none()
        );
        assert!(append_path(&[], &wide("C:\\"), false).unwrap().is_some());
        assert!(
            append_path(&wide("C:"), &wide("C:\\"), false)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn invalid_path_values_and_overflows_are_rejected_without_overwriting() {
        let registry = TestRegistry::new();
        let directory = Path::new(r"C:\SIMP");
        registry.set_bytes(REG_DWORD, &7_u32.to_le_bytes());
        assert!(update_path(registry.key(), directory).is_err());
        for bytes in [vec![65], vec![65, 0, 0, 0, 66, 0]] {
            registry.set_bytes(REG_SZ, &bytes);
            assert!(update_path(registry.key(), directory).is_err());
        }
        let long_value = "x".repeat(MAX_PATH_UNITS);
        registry.set(REG_SZ, &long_value);
        assert!(update_path(registry.key(), directory).is_err());
        assert_eq!(
            read_path(registry.key()).unwrap(),
            Some((REG_SZ, wide(&long_value)))
        );
        for path in [
            r"C:\SIMP;C:\Injected",
            r"C:\%Expanded%",
            "relative",
            "C:\\Bad\0Path",
        ] {
            assert!(validate_path_entry(Path::new(path)).is_err());
        }
    }

    #[test]
    fn registry_strings_preserve_non_utf8_windows_paths_and_unterminated_values() {
        let registry = TestRegistry::new();
        registry.set_bytes(REG_SZ, &[0, 216]);
        assert!(update_path(registry.key(), Path::new(r"C:\SIMP")).unwrap());
        let (_, value) = read_path(registry.key()).unwrap().unwrap();
        assert_eq!(value[0], 0xd800);
        assert_eq!(value[1], SEMICOLON);
    }

    #[test]
    fn system_path_requires_elevation_without_writing_any_environment_values() {
        if !is_elevated().unwrap() {
            let error = check_path_access(PathScope::System, Path::new(r"C:\SIMP")).unwrap_err();
            assert!(error.to_string().contains("administrator privileges"));
        }
    }
}
