use std::{
    fs::{self},
    path::PathBuf,
};

use clap::Parser;
use simp_core::{Footer, Manifest, SimpConfig};
use tokio::{fs::File, io::AsyncWriteExt};

const INSTALLER_EXE: &[u8] = include_bytes!("../../target/release/simp-installer.exe");

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(short, long, default_value = "./simp.json")]
    input: PathBuf,

    #[arg(short, long)]
    output: Option<PathBuf>,
}

pub fn build_installer_name(config: &SimpConfig) -> String {
    format!(
        "{}-{}-installer.exe",
        config.core_config.name, config.core_config.version
    )
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = Args::parse();

    if !args.input.exists() {
        eprintln!("Must specify valid path to simp.json");
        std::process::exit(1);
    }

    let contents = fs::read_to_string(&args.input)?;
    let config: SimpConfig = serde_json::from_str(&contents)?;

    if let Err(e) = config.components_are_valid() {
        eprintln!("{}", e);
        std::process::exit(1);
    }

    let output = match args.output {
        Some(mut path) => {
            if path.is_dir() {
                path.push(build_installer_name(&config));
                path
            } else {
                if path.extension().and_then(|ext| ext.to_str()) != Some("exe") {
                    eprintln!("Output file must end with .exe");
                    std::process::exit(1);
                }

                path
            }
        }

        None => {
            let mut path = args.input.clone();
            path.set_file_name(build_installer_name(&config));
            path
        }
    };

    let manifest = Manifest::new(&config).await?;
    let mut file = File::create(output).await?;
    file.write_all(INSTALLER_EXE).await?;
    let encoded: Vec<u8> = manifest.encode();
    file.write_all(&encoded).await?;

    let footer = Footer {
        manifest_offset: u64::try_from(INSTALLER_EXE.len())?,
        version: Footer::FORMAT_VERSION,
    };

    let footer_buf: [u8; 14] = footer.into();
    file.write_all(footer_buf.as_ref()).await?;

    println!("{:?}", INSTALLER_EXE.len());
    Ok(())
}
