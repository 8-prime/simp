use std::path::PathBuf;

pub mod operation;

use simp_core::{Footer, Manifest};
use tokio::fs::read;

pub async fn parse_installer(path: PathBuf) -> eyre::Result<Manifest> {
    let bytes = read(&path)
        .await
        .map_err(|_| eyre::eyre!("Failed to read installer {:#?}", path))?;

    parse_manifest_bytes(&bytes)
}

fn parse_manifest_bytes(bytes: &[u8]) -> eyre::Result<Manifest> {
    let footer_start = bytes
        .len()
        .checked_sub(Footer::SIZE)
        .ok_or_else(|| eyre::eyre!("Installer is too small to contain a footer"))?;

    let footer_bytes = &bytes[footer_start..];

    if !footer_bytes.starts_with(&Footer::MAGIC.to_le_bytes()) {
        eyre::bail!("Installer footer has invalid magic bytes");
    }

    let footer = Footer::try_from(footer_bytes)
        .map_err(|_| eyre::eyre!("Failed to read footer from footer bytes of installer"))?;

    if footer.version != 1 {
        eyre::bail!("Unsupported installer format version {}", footer.version);
    }

    let manifest_start = usize::try_from(footer.manifest_offset)
        .map_err(|_| eyre::eyre!("Manifest offset is too large for this platform"))?;

    let manifest_bytes = bytes
        .get(manifest_start..footer_start)
        .ok_or_else(|| eyre::eyre!("Invalid manifest byte range"))?;

    Manifest::decode(manifest_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use simp_core::{CoreConfig, InstallTarget, ManifestEntry};

    #[test]
    fn reads_manifest_between_executable_and_footer() {
        let manifest = Manifest {
            core_config: CoreConfig {
                name: "Example".into(),
                version: "1.0.0".into(),
                description: None,
                eula: None,
                auto_start: None,
                desktop_icon: None,
                start_menu: None,
                add_to_path: None,
                target: InstallTarget::ProgramFiles,
            },
            entries: vec![ManifestEntry {
                name: "example.txt".into(),
                data: b"hello".to_vec(),
            }],
        };
        let mut bytes = b"fake executable".to_vec();
        let manifest_offset = bytes.len() as u64;
        bytes.extend(manifest.encode());
        bytes.extend(<[u8; Footer::SIZE]>::from(Footer {
            version: 1,
            manifest_offset,
        }));

        assert_eq!(parse_manifest_bytes(&bytes).unwrap(), manifest);
    }

    #[test]
    fn rejects_missing_footer_and_invalid_manifest_range() {
        assert!(parse_manifest_bytes(b"short").is_err());

        let footer = Footer {
            version: 1,
            manifest_offset: 100,
        };
        assert!(parse_manifest_bytes(&<[u8; Footer::SIZE]>::from(footer)).is_err());
    }

    #[test]
    fn rejects_unknown_footer_format() {
        let footer = Footer {
            version: 2,
            manifest_offset: 0,
        };
        let mut bytes = <[u8; Footer::SIZE]>::from(footer);
        assert!(parse_manifest_bytes(&bytes).is_err());

        bytes = Footer {
            version: 1,
            manifest_offset: 0,
        }
        .into();
        bytes[0] = 0;
        assert!(parse_manifest_bytes(&bytes).is_err());
    }
}
