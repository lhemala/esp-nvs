use std::path::PathBuf;

use clap::{
    Parser,
    Subcommand,
};
use esp_nvs_partition_tool::{
    NvsPartition,
    FLASH_SECTOR_SIZE,
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Generate {
            input,
            output,
            size,
        } => {
            println!("Parsing CSV file: {}", input.display());
            let partition = NvsPartition::from_csv_file(&input)?;
            println!("Found {} entries", partition.entries.len());

            println!("Generating partition binary...");
            partition.generate_partition_file(&output, size)?;

            println!("Successfully generated NVS partition: {}", output.display());
            println!("Size: {} bytes ({} pages)", size, size / FLASH_SECTOR_SIZE);

            Ok(())
        }
        Commands::Parse { input, output } => {
            println!("Parsing binary file: {}", input.display());
            let partition = NvsPartition::parse_partition_file(&input)?;
            println!("Found {} entries", partition.entries.len());

            println!("Writing CSV file...");
            partition.to_csv_file(&output)?;

            println!("Successfully parsed NVS partition to: {}", output.display());

            Ok(())
        }
    }
}
