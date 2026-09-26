use std::{fs, path::PathBuf};

use clap::Parser;
use simp_core::SimpConfig;

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
    format!("{}-{}-installer.exe", config.name, config.version)
}

fn main() -> eyre::Result<()> {
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

    // parse args
    // input path to simp.json
    // optional output dir for final installer

    // combine installer exe with files specified in simp, together with simp.config

    println!("{:?}", INSTALLER_EXE.len());
    Ok(())
}
