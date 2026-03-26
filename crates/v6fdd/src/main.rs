use clap::Parser;
use std::path::PathBuf;
use v6_core::fdd::filesystem::Filesystem;

/// v6fdd — Vector-06c FDD image utility
#[derive(Parser)]
#[command(name = "v6fdd", about = "FDD image utility for Vector-06c")]
struct Cli {
    /// Template FDD image (e.g., boot sector + OS)
    #[arg(short = 't', long = "template")]
    template: Option<PathBuf>,

    /// Files to add to the FDD image (can be repeated)
    #[arg(short = 'i', long = "input", required = true)]
    input: Vec<PathBuf>,

    /// Output FDD image file
    #[arg(short = 'o', long = "output", required = true)]
    output: PathBuf,
}

fn main() {
    env_logger::init();
    let cli = Cli::parse();

    let mut fs = if let Some(ref tpl) = cli.template {
        let data = std::fs::read(tpl).unwrap_or_else(|e| {
            eprintln!("Error reading template file {:?}: {}", tpl, e);
            std::process::exit(1);
        });
        Filesystem::from_bytes(&data)
    } else {
        Filesystem::new()
    };

    for input_path in &cli.input {
        let data = std::fs::read(input_path).unwrap_or_else(|e| {
            eprintln!("Error reading input file {:?}: {}", input_path, e);
            std::process::exit(1);
        });
        let basename = input_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "UNKNOWN".to_string());

        match fs.save_file(&basename, &data) {
            Some(free) => {
                eprintln!("Saved {} ({} bytes), free space: {} bytes", basename, data.len(), free);
            }
            None => {
                eprintln!("Disk full, cannot save {}", basename);
                std::process::exit(1);
            }
        }
    }

    // Ensure output directory exists
    if let Some(parent) = cli.output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| {
                eprintln!("Error creating output directory: {}", e);
                std::process::exit(1);
            });
        }
    }

    std::fs::write(&cli.output, &fs.bytes).unwrap_or_else(|e| {
        eprintln!("Error writing output file {:?}: {}", cli.output, e);
        std::process::exit(1);
    });

    eprintln!("FDD image written to {:?}", cli.output);
}
