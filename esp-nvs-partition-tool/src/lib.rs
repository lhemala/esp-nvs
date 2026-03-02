//! ESP-IDF compatible NVS (Non-Volatile Storage) partition table parser and
//! generator.

pub mod error;
pub mod partition;

mod csv;

use std::fs;
use std::io::Write;
use std::path::Path;

pub use error::Error;
pub use partition::{
    DataValue,
    EntryContent,
    FileEncoding,
    NvsEntry,
    FLASH_SECTOR_SIZE,
    MAX_KEY_LENGTH,
};

/// A collection of NVS key-value entries, optionally spanning multiple
/// namespaces.
///
/// This is the primary in-memory representation used by the CSV and binary
/// parsers/generators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvsPartition {
    /// The ordered list of entries in this partition.
    pub entries: Vec<NvsEntry>,
}

impl NvsPartition {
    /// Parse NVS CSV content from a string.
    ///
    /// File-type entries store the path exactly as written in the CSV. Use
    /// [`NvsPartition::from_csv_file`] when parsing from a file on disk so
    /// that relative paths are resolved automatically.
    pub fn from_csv(content: &str) -> Result<Self, Error> {
        csv::parser::parse_csv(content)
    }

    /// Parse an NVS CSV file at the given `path`.
    ///
    /// File-type entries in the CSV have their paths resolved relative to the
    /// parent directory of the CSV file.
    pub fn from_csv_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let content = fs::read_to_string(&path)?;
        let mut partition = csv::parser::parse_csv(&content)?;

        // Resolve relative file paths against the CSV file's parent directory.
        if let Some(base) = path.as_ref().parent() {
            for entry in &mut partition.entries {
                if let EntryContent::File { file_path, .. } = &mut entry.content {
                    if file_path.is_relative() {
                        *file_path = base.join(&file_path);
                    }
                }
            }
        }

        Ok(partition)
    }

    /// Serialize this partition to CSV and return the content as a `String`.
    ///
    /// See [`NvsPartition::to_csv_file`] for details on ordering and
    /// encoding behavior.
    pub fn to_csv(self) -> Result<String, Error> {
        csv::writer::write_csv_content(self)
    }

    /// Serialize this partition to a CSV file at the given `path`.
    ///
    /// Entries are written in their original insertion order. A namespace
    /// header row is emitted whenever the namespace changes between
    /// consecutive entries. `Encoding::Binary` values are serialized as
    /// base64, matching the ESP-IDF `nvs_partition_tool` convention.
    pub fn to_csv_file<P: AsRef<Path>>(self, path: P) -> Result<(), Error> {
        csv::writer::write_csv(self, path)
    }

    /// Generate an NVS partition binary in memory.
    ///
    /// `size` must be a multiple of 4096 (the ESP-IDF flash sector size).
    pub fn generate_partition(&self, size: usize) -> Result<Vec<u8>, Error> {
        partition::generator::generate_partition_data(self, size)
    }

    /// Generate an NVS partition binary and write it to `path`.
    ///
    /// `size` must be a multiple of 4096 (the ESP-IDF flash sector size).
    pub fn generate_partition_file<P: AsRef<Path>>(
        &self,
        path: P,
        size: usize,
    ) -> Result<(), Error> {
        let data = self.generate_partition(size)?;
        std::fs::File::create(path)?.write_all(&data)?;
        Ok(())
    }

    /// Parse an NVS partition binary from an in-memory byte slice.
    pub fn parse_partition(data: &[u8]) -> Result<Self, Error> {
        partition::parser::parse_binary_data(data)
    }

    /// Parse an NVS partition binary file at the given `path`.
    pub fn parse_partition_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        partition::parser::parse_binary(path)
    }
}
