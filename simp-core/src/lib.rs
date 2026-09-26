use std::path::PathBuf;

use bitcode::{Decode, Encode};
use serde::{Deserialize, Serialize};

pub struct Footer {
    pub version: u16,
    pub manifest_offset: u64,
}

impl Footer {
    pub const MAGIC: u32 = 0x73696D70;
    pub const SIZE: usize = 14;
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

#[derive(Serialize, Deserialize)]
pub enum InstallTarget {
    ProgramFiles(String),
    Custom(PathBuf),
}

#[derive(Serialize, Deserialize)]
pub struct InstallComponent {
    pub component_path: PathBuf,
}

#[derive(Serialize, Deserialize)]
pub struct SimpConfig {
    pub name: String,
    pub version: String,
    pub target: InstallTarget,
    pub components: Vec<InstallComponent>,
    pub auto_start: bool,
    pub desktop_icon: bool,
    pub start_menu: bool,
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
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    pub async fn new(config: &SimpConfig) -> eyre::Result<Self> {
        let mut manifest = Manifest { entries: vec![] };
        for comp in config.components.iter() {
            Manifest::handle_component(comp, &mut manifest).await?;
        }
        return Ok(manifest);
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
