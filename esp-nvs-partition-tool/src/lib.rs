//! ESP-IDF compatible NVS (Non-Volatile Storage) partition table parser and
//! generator.

pub mod error;
pub mod partition;

mod csv;

pub use error::Error;
pub use partition::{
    DataValue,
    EntryContent,
    FileEncoding,
    MAX_KEY_LENGTH,
    NvsEntry,
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
    /// Attempt to parse either a binary or CSV NVS partition from the given
    /// input.
    ///
    /// Binary partitions are detected by checking whether the first byte
    /// looks like an NVS page-state value (≥ 0x80). If so the data is
    /// parsed as binary; otherwise it is interpreted as CSV text.
    pub fn try_from<D>(data: D) -> Result<Self, Error>
    where
        D: Into<Vec<u8>>,
    {
        let input: Vec<u8> = data.into();

        // NVS binary partitions start with the page state (u32 LE).
        // Valid page states (Active = 0xFE, Full = 0xFC, Freeing = 0xF8, etc.)
        // all have their first byte well above 0x80, while CSV text is always
        // valid ASCII (< 0x80). We use 0x80 as the threshold to reliably
        // distinguish the two formats.
        if input.first().is_some_and(|&b| b >= 0x80) {
            Self::try_from_bytes(input)
        } else {
            Self::try_from_str(
                String::from_utf8(input).map_err(|e| Error::InvalidValue(format!("input is not valid UTF-8: {e}")))?,
            )
        }
    }

    /// Attempt to parse a CSV NVS partition from the given string.
    ///
    /// File-type entries store the path exactly as written in the CSV.
    pub fn try_from_str<S>(string: S) -> Result<Self, Error>
    where
        S: Into<String>,
    {
        csv::parser::parse_csv(&string.into())
    }

    /// Attempt to parse a binary NVS partition from the given bytes.
    pub fn try_from_bytes<B>(bytes: B) -> Result<Self, Error>
    where
        B: Into<Vec<u8>>,
    {
        partition::parser::parse_binary_data(&bytes.into())
    }

    /// Serialize this partition to CSV and return the content as a `String`.
    ///
    /// Entries are written in their original insertion order. A namespace
    /// header row is emitted whenever the namespace changes between
    /// consecutive entries. `Encoding::Binary` values are serialized as
    /// base64, matching the ESP-IDF `nvs_partition_tool` convention.
    pub fn to_csv(self) -> Result<String, Error> {
        csv::writer::write_csv_content(self)
    }

    /// Generate an NVS partition binary in memory.
    ///
    /// `size` must be a multiple of 4096 (the ESP-IDF flash sector size).
    pub fn generate_partition(&self, size: usize) -> Result<Vec<u8>, Error> {
        partition::generator::generate_partition_data(self, size)
    }

    /// Find an entry with the given name in the NVS partition.
    pub fn find(&self, name: &str) -> Option<&NvsEntry> {
        self.entries.iter().find(|e| e.key == name)
    }

    /// Find an entry by key, returning a mutable reference.
    pub fn find_mut(&mut self, name: &str) -> Option<&mut NvsEntry> {
        self.entries.iter_mut().find(|e| e.key == name)
    }
}
