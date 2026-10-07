use std::path::PathBuf;

use eyre::WrapErr;
use simp_core::PackageManifest;
use tokio::fs::File;

pub mod operation;

/// Loaded metadata and the file that supplies payloads during installation.
pub struct InstallerPackage {
    pub manifest: PackageManifest,
    pub path: PathBuf,
}

pub async fn parse_installer(path: PathBuf) -> eyre::Result<InstallerPackage> {
    let path = tokio::fs::canonicalize(&path)
        .await
        .wrap_err_with(|| format!("Failed to locate installer {}", path.display()))?;
    let mut file = File::open(&path)
        .await
        .wrap_err_with(|| format!("Failed to open installer {}", path.display()))?;
    let manifest = PackageManifest::decode(&mut file).await?;
    Ok(InstallerPackage { manifest, path })
}
