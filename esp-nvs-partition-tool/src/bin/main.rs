use std::fs;
use std::path::PathBuf;

use clap::{
    Parser,
    Subcommand,
};
use esp_nvs_partition_tool::{
    EntryContent,
    NVS_KEY_SIZE,
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

        /// Encrypt the partition with the keys in this file (`eky || tky`, 64 byte)
        #[arg(short, long)]
        keyfile: Option<PathBuf>,
    },
    /// Parse NVS partition binary to CSV file
    Parse {
        /// Input binary file path
        input: PathBuf,

        /// Output CSV file path
        output: PathBuf,

        /// Decrypt the partition with the keys in this file (`eky || tky`, 64 byte)
        #[arg(short, long)]
        keyfile: Option<PathBuf>,
    },
}

/// Reads `eky || tky` from the start of a key file, as written by `nvs_partition_gen.py`.
fn read_keyfile(path: &PathBuf) -> Result<[u8; NVS_KEY_SIZE], Box<dyn std::error::Error>> {
    let keys = fs::read(path)?;

    keys.get(..NVS_KEY_SIZE)
        .and_then(|keys| keys.try_into().ok())
        .ok_or_else(|| format!("{} is shorter than {NVS_KEY_SIZE} byte", path.display()).into())
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
            keyfile,
        } => {
            println!("Parsing CSV file: {}", input.display());
            let content = fs::read_to_string(&input)?;
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
            let data = match &keyfile {
                Some(path) => partition.generate_encrypted_partition(size, &read_keyfile(path)?)?,
                None => partition.generate_partition(size)?,
            };
            fs::write(&output, &data)?;

            println!("Successfully generated NVS partition: {}", output.display());
            println!("Size: {} bytes ({} pages)", size, size / esp_nvs::FLASH_SECTOR_SIZE);

            Ok(())
        }
        Commands::Parse { input, output, keyfile } => {
            println!("Parsing binary file: {}", input.display());
            let data = fs::read(&input)?;
            let partition = match &keyfile {
                Some(path) => NvsPartition::try_from_encrypted_bytes(data, &read_keyfile(path)?)?,
                None => NvsPartition::try_from_bytes(data)?,
            };
            println!("Found {} entries", partition.entries.len());

            println!("Writing CSV file...");
            let csv_content = partition.to_csv()?;
            fs::write(&output, &csv_content)?;

            println!("Successfully parsed NVS partition to: {}", output.display());

            Ok(())
        }
    }
}
