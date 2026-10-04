use std::path::{Component, Path, PathBuf};

use bitcode::{Decode, Encode};
use serde::{Deserialize, Serialize};

pub struct Footer {
    pub version: u16,
    pub manifest_offset: u64,
}

impl Footer {
    pub const MAGIC: u32 = 0x73696D70;
    pub const SIZE: usize = 14;
    pub const FORMAT_VERSION: u16 = 3;
}

impl From<Footer> for [u8; 14] {
    fn from(value: Footer) -> Self {
        let mut buf = [0u8; 14];

        buf[0..4].copy_from_slice(Footer::MAGIC.to_le_bytes().as_ref());
        buf[4..6].copy_from_slice(value.version.to_le_bytes().as_ref());
        buf[6..14].copy_from_slice(value.manifest_offset.to_le_bytes().as_ref());

        return buf;
    }
}

impl TryFrom<&[u8]> for Footer {
    type Error = ();

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.len() < Footer::SIZE {
            return Err(());
        }

        let version = u16::from_le_bytes(value[4..6].try_into().map_err(|_| ())?);
        let offset = u64::from_le_bytes(value[6..14].try_into().map_err(|_| ())?);

        return Ok(Footer {
            manifest_offset: offset,
            version: version,
        });
    }
}

#[derive(Serialize, Deserialize, Encode, Decode, PartialEq, Debug, Clone)]
pub struct CoreConfig {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub eula: Option<String>,
    /// UTF-8 package path relative to the installation directory, not a source path.
    pub main_executable: Option<String>,
    pub auto_start: Option<bool>,
    pub desktop_icon: Option<bool>,
    pub start_menu: Option<bool>,
    pub add_to_path: Option<bool>,
    pub path_scope: Option<PathScopePolicy>,
    pub target: InstallTarget,
}

#[derive(Serialize, Deserialize, Encode, Decode, PartialEq, Eq, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PathScope {
    User,
    System,
}

#[derive(Serialize, Deserialize, Encode, Decode, PartialEq, Eq, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PathScopePolicy {
    User,
    System,
    UserChoice,
}

impl Default for PathScopePolicy {
    fn default() -> Self {
        PathScopePolicy::User
    }
}

impl PathScopePolicy {
    pub fn initial_scope(self) -> PathScope {
        match self {
            Self::User | Self::UserChoice => PathScope::User,
            Self::System => PathScope::System,
        }
    }

    pub fn allows(self, scope: PathScope) -> bool {
        match self {
            Self::User => scope == PathScope::User,
            Self::System => scope == PathScope::System,
            Self::UserChoice => true,
        }
    }
}

#[derive(Serialize, Deserialize, Encode, Decode, PartialEq, Debug, Clone)]
pub enum InstallTarget {
    ProgramFiles,
    AppData,
    Custom(String),
}

impl InstallTarget {
    pub fn resolve(&self, app_name: &String) -> PathBuf {
        match self {
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
}

#[derive(Serialize, Deserialize)]
pub struct InstallComponent {
    pub component_path: PathBuf,
}

#[derive(Serialize, Deserialize)]
pub struct SimpConfig {
    pub core_config: CoreConfig,
    pub components: Vec<InstallComponent>,
}

impl SimpConfig {
    pub fn components_are_valid(&self) -> eyre::Result<()> {
        for comp in self.components.iter() {
            if !comp.component_path.exists() {
                eyre::bail!(
                    "Installer component {:#?} does not specify a valid path",
                    comp.component_path
                );
            }
        }
        Ok(())
    }
}

#[derive(Encode, Decode, PartialEq, Debug)]
pub struct ManifestEntry {
    pub name: String,
    pub data: Vec<u8>,
}

impl ManifestEntry {
    pub async fn new(component_path: &PathBuf) -> eyre::Result<Self> {
        let Some(file_name) = component_path.file_name() else {
            eyre::bail!("Cannot determine file name for path {:#?}", component_path);
        };
        let name = file_name
            .to_str()
            .ok_or_else(|| eyre::eyre!("File name is not valid UTF-8"))?
            .to_owned();
        let bytes: Vec<u8> = tokio::fs::read(&component_path).await?;
        return Ok(ManifestEntry {
            name: name,
            data: bytes,
        });
    }
}

#[derive(Encode, Decode, PartialEq, Debug)]
pub struct Manifest {
    pub core_config: CoreConfig,
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    pub async fn new(config: &SimpConfig) -> eyre::Result<Self> {
        let mut manifest = Manifest {
            core_config: config.core_config.clone(),
            entries: vec![],
        };

        for comp in config.components.iter() {
            Manifest::handle_component(comp, &mut manifest).await?;
        }

        manifest.validate_main_executable()?;
        return Ok(manifest);
    }

    pub fn encode(&self) -> Vec<u8> {
        return bitcode::encode(self);
    }

    pub fn decode(bytes: &[u8]) -> eyre::Result<Self> {
        let manifest: Self = bitcode::decode(bytes)?;
        manifest.validate_main_executable()?;
        Ok(manifest)
    }

    pub fn validate_main_executable(&self) -> eyre::Result<()> {
        if let Some(target) = &self.core_config.main_executable {
            self.validate_file_target(Path::new(target))?;
        }
        Ok(())
    }

    /// Application launch targets must identify exactly one packaged file.
    pub fn validate_file_target(&self, target: &Path) -> eyre::Result<()> {
        if !target
            .components()
            .any(|part| matches!(part, Component::Normal(_)))
            || target.components().any(|part| {
                matches!(
                    part,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            eyre::bail!(
                "Application target {} must be a relative file path within the installation directory",
                target.display()
            );
        }

        let matches = self
            .entries
            .iter()
            .filter(|entry| Path::new(&entry.name) == target)
            .count();
        match matches {
            1 => Ok(()),
            0 => eyre::bail!(
                "Application target {} is not a packaged file",
                target.display()
            ),
            _ => eyre::bail!(
                "Application target {} matches multiple packaged files",
                target.display()
            ),
        }
    }

    async fn handle_component(
        component: &InstallComponent,
        manifest: &mut Manifest,
    ) -> eyre::Result<()> {
        let path = &component.component_path;
        if path.is_file() {
            manifest
                .entries
                .push(ManifestEntry::new(&component.component_path).await?);
        } else if path.is_dir() {
            Manifest::handle_dir_component(&component.component_path, manifest).await?;
        } else {
            eyre::bail!(
                "Required component at path {:#?} does not exist",
                component.component_path
            );
        }
        Ok(())
    }

    async fn handle_dir_component(
        component_path: &PathBuf,
        manifest: &mut Manifest,
    ) -> eyre::Result<()> {
        let mut pending = vec![component_path.clone()];

        while let Some(dir) = pending.pop() {
            let mut entries = tokio::fs::read_dir(&dir).await?;

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                let kind = entry.file_type().await?;

                if kind.is_dir() {
                    pending.push(path);
                } else if kind.is_file() {
                    manifest.entries.push(ManifestEntry::new(&path).await?);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_with_custom_target_round_trips() {
        let manifest = Manifest {
            core_config: CoreConfig {
                name: "Example".into(),
                version: "1.0.0".into(),
                description: None,
                eula: None,
                main_executable: Some("app.exe".into()),
                auto_start: None,
                desktop_icon: None,
                start_menu: None,
                add_to_path: None,
                path_scope: None,
                target: InstallTarget::Custom("C:/Example".into()),
            },
            entries: vec![ManifestEntry {
                name: "app.exe".into(),
                data: vec![1, 2, 3],
            }],
        };

        let encoded = manifest.encode();
        let decoded = Manifest::decode(&encoded).unwrap();
        assert_eq!(decoded, manifest);
    }

    fn manifest_with_target(target: Option<&str>) -> Manifest {
        Manifest {
            core_config: CoreConfig {
                name: "Example".into(),
                version: "1.0.0".into(),
                description: None,
                eula: None,
                main_executable: target.map(str::to_owned),
                auto_start: None,
                desktop_icon: None,
                start_menu: None,
                add_to_path: None,
                path_scope: None,
                target: InstallTarget::ProgramFiles,
            },
            entries: ["helper.exe", "app.exe"]
                .into_iter()
                .map(|name| ManifestEntry {
                    name: name.into(),
                    data: Vec::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn main_executable_identifies_a_packaged_file_without_guessing() {
        assert!(
            manifest_with_target(Some("app.exe"))
                .validate_main_executable()
                .is_ok()
        );
        assert!(
            manifest_with_target(None)
                .validate_main_executable()
                .is_ok()
        );
        for target in [
            "",
            ".",
            "missing.exe",
            "../app.exe",
            "sub/../app.exe",
            "C:/app.exe",
        ] {
            let manifest = manifest_with_target(Some(target));
            assert!(manifest.validate_main_executable().is_err(), "{target}");
            assert!(Manifest::decode(&manifest.encode()).is_err(), "{target}");
        }
        let mut manifest = manifest_with_target(Some("app.exe"));
        manifest.entries.push(ManifestEntry {
            name: "app.exe".into(),
            data: Vec::new(),
        });
        assert!(manifest.validate_main_executable().is_err());
    }

    #[test]
    fn main_executable_is_optional_in_existing_json_configs() {
        let config: SimpConfig =
            serde_json::from_str(include_str!("../../examples/hello-world/simp.json")).unwrap();
        assert_eq!(
            config.core_config.main_executable.as_deref(),
            Some("hello.cmd")
        );
        let mut json = serde_json::to_value(&config.core_config).unwrap();
        json.as_object_mut().unwrap().remove("main_executable");
        let config: CoreConfig = serde_json::from_value(json).unwrap();
        assert!(config.main_executable.is_none());
    }

    #[tokio::test]
    async fn building_a_manifest_validates_the_installed_target() {
        let mut config: SimpConfig =
            serde_json::from_str(include_str!("../../examples/hello-world/simp.json")).unwrap();
        let example_directory =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/hello-world");
        for component in &mut config.components {
            component.component_path = example_directory.join(&component.component_path);
        }
        let manifest = Manifest::new(&config).await.unwrap();
        assert_eq!(
            manifest.core_config.main_executable.as_deref(),
            Some("hello.cmd")
        );
        assert_eq!(Manifest::decode(&manifest.encode()).unwrap(), manifest);

        config.core_config.main_executable = Some("missing.exe".into());
        assert!(Manifest::new(&config).await.is_err());
    }

    #[test]
    fn path_scope_policies_parse_from_json_and_survive_packaging() {
        for (json, policy) in [
            ("\"user\"", PathScopePolicy::User),
            ("\"system\"", PathScopePolicy::System),
            ("\"user_choice\"", PathScopePolicy::UserChoice),
        ] {
            assert_eq!(
                serde_json::from_str::<PathScopePolicy>(json).unwrap(),
                policy
            );
            let mut manifest = manifest_with_target(None);
            manifest.core_config.path_scope = Some(policy);
            assert_eq!(Manifest::decode(&manifest.encode()).unwrap(), manifest);
        }
        let config: CoreConfig = serde_json::from_str(
            r#"{"name":"Example","version":"1.0","target":"AppData","add_to_path":true}"#,
        )
        .unwrap();
        assert!(config.path_scope.is_none());
        assert_eq!(
            config
                .path_scope
                .unwrap_or(PathScopePolicy::User)
                .initial_scope(),
            PathScope::User
        );
        assert!(serde_json::from_str::<PathScopePolicy>("\"automatic\"").is_err());
    }

    #[test]
    fn path_scope_policies_allow_only_the_configured_scopes() {
        for (policy, user_allowed, system_allowed) in [
            (PathScopePolicy::User, true, false),
            (PathScopePolicy::System, false, true),
            (PathScopePolicy::UserChoice, true, true),
        ] {
            assert_eq!(policy.allows(PathScope::User), user_allowed);
            assert_eq!(policy.allows(PathScope::System), system_allowed);
        }
    }
}
