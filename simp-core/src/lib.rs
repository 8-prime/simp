use std::path::PathBuf;

use eyre::eyre;
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
