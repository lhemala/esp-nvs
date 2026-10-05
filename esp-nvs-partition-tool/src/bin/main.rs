use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{
    Parser,
    Subcommand,
};
use esp_nvs_partition_tool::{
    EntryContent,
    NvsPartition,
};

#[derive(Parser)]
#[command(name = "esp-nvs-partition-tool")]
#[command(about = "ESP NVS partition generator and parser", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate NVS partition binary from CSV file
    Generate {
        /// Input CSV file path
        input: PathBuf,

        /// Output binary file path
        output: PathBuf,

        /// Partition size in bytes (must be multiple of 4096)
        #[arg(short, long, value_parser = parse_size)]
        size: usize,
    },
    /// Parse NVS partition binary to CSV file
    Parse {
        /// Input binary file path
        input: PathBuf,

        /// Output CSV file path
        output: PathBuf,
    },
}

fn parse_size(s: &str) -> Result<usize, String> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        usize::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else {
        s.parse::<usize>().map_err(|e| e.to_string())
    }
}

fn main() -> ExitCode {
    // Returning the error from `main` prints its `Debug` form, which is not meant for users.
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Adds the path to an error from accessing a file.
fn with_path(path: &std::path::Path) -> impl FnOnce(std::io::Error) -> String + '_ {
    move |e| format!("{}: {e}", path.display())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Generate { input, output, size } => {
            println!("Parsing CSV file: {}", input.display());
            let content = fs::read_to_string(&input).map_err(with_path(&input))?;
            let mut partition = NvsPartition::try_from_str(&content)?;

            // Resolve relative file paths against the CSV file's parent
            // directory.
            if let Some(base) = input.parent() {
                for entry in &mut partition.entries {
                    if let EntryContent::File { file_path, .. } = &mut entry.content {
                        if file_path.is_relative() {
                            *file_path = base.join(&file_path);
                        }
                    }
                }
            }

            println!("Found {} entries", partition.entries.len());

            println!("Generating partition binary...");
            let data = partition.generate_partition(size)?;
            fs::write(&output, &data).map_err(with_path(&output))?;

            println!("Successfully generated NVS partition: {}", output.display());
            println!("Size: {} bytes ({} pages)", size, size / esp_nvs::FLASH_SECTOR_SIZE);

            Ok(())
        }
        Commands::Parse { input, output } => {
            println!("Parsing binary file: {}", input.display());
            let data = fs::read(&input).map_err(with_path(&input))?;
            let partition = NvsPartition::try_from_bytes(data)?;
            println!("Found {} entries", partition.entries.len());

            println!("Writing CSV file...");
            let csv_content = partition.to_csv()?;
            fs::write(&output, &csv_content).map_err(with_path(&output))?;

            println!("Successfully parsed NVS partition to: {}", output.display());

            Ok(())
        }
    }
}
