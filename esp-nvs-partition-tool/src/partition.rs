pub mod crc;

pub(crate) mod consts;
pub(crate) mod generator;
pub(crate) mod parser;

use std::path::PathBuf;

pub use consts::FLASH_SECTOR_SIZE;

use crate::error::Error;

/// Maximum Key length is 15 bytes + 1 byte for the null terminator.
pub const MAX_KEY_LENGTH: usize = 15;

/// A single NVS key-value entry belonging to a namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvsEntry {
    /// The namespace this entry belongs to (max 15 bytes).
    pub namespace: String,
    /// The key identifying this entry within its namespace (max 15 bytes).
    pub key: String,
    /// The payload — either inline data or a reference to an external file.
    pub content: EntryContent,
}

/// The content of an NVS entry — either inline data or a file reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryContent {
    /// Inline data whose encoding is determined by the [`DataValue`] variant.
    Data(DataValue),
    /// A reference to a file whose content will be read at generation time.
    File {
        /// How the file content is interpreted.
        encoding: FileEncoding,
        /// Path to the file (resolved relative to the CSV location).
        file_path: PathBuf,
    },
}

/// The encoding used to interpret file content for NVS file entries.
///
/// `String` reads the file as UTF-8 text. `Hex2Bin` decodes hex-encoded
/// content. `Base64` decodes base64-encoded content. `Binary` uses the
/// raw bytes directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEncoding {
    /// UTF-8 text.
    String,
    /// Hex-encoded binary data.
    Hex2Bin,
    /// Base64-encoded binary data.
    Base64,
    /// Raw binary data.
    Binary,
}

impl std::str::FromStr for FileEncoding {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "string" => Ok(Self::String),
            "hex2bin" => Ok(Self::Hex2Bin),
            "base64" => Ok(Self::Base64),
            "binary" => Ok(Self::Binary),
            _ => Err(Error::InvalidEncoding(s.to_string())),
        }
    }
}

impl FileEncoding {
    /// Return the encoding name as a static string slice.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Hex2Bin => "hex2bin",
            Self::Base64 => "base64",
            Self::Binary => "binary",
        }
    }
}

impl std::fmt::Display for FileEncoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A concrete data value stored in an NVS entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataValue {
    /// Unsigned 8-bit integer.
    U8(u8),
    /// Signed 8-bit integer.
    I8(i8),
    /// Unsigned 16-bit integer.
    U16(u16),
    /// Signed 16-bit integer.
    I16(i16),
    /// Unsigned 32-bit integer.
    U32(u32),
    /// Signed 32-bit integer.
    I32(i32),
    /// Unsigned 64-bit integer.
    U64(u64),
    /// Signed 64-bit integer.
    I64(i64),
    /// UTF-8 string (without null terminator).
    String(String),
    /// Opaque byte blob.
    Binary(Vec<u8>),
}

impl DataValue {
    /// Return the CSV encoding column string for this value.
    ///
    /// `Binary` maps to `"base64"` because blobs parsed from a binary
    /// partition have no original CSV encoding, and the ESP-IDF convention
    /// is base64.
    pub fn encoding_str(&self) -> &'static str {
        match self {
            Self::U8(_) => "u8",
            Self::I8(_) => "i8",
            Self::U16(_) => "u16",
            Self::I16(_) => "i16",
            Self::U32(_) => "u32",
            Self::I32(_) => "i32",
            Self::U64(_) => "u64",
            Self::I64(_) => "i64",
            Self::String(_) => "string",
            Self::Binary(_) => "base64",
        }
    }
}

impl std::fmt::Display for DataValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::U8(v) => write!(f, "{v}"),
            Self::I8(v) => write!(f, "{v}"),
            Self::U16(v) => write!(f, "{v}"),
            Self::I16(v) => write!(f, "{v}"),
            Self::U32(v) => write!(f, "{v}"),
            Self::I32(v) => write!(f, "{v}"),
            Self::U64(v) => write!(f, "{v}"),
            Self::I64(v) => write!(f, "{v}"),
            Self::String(s) => f.write_str(s),
            Self::Binary(b) => {
                use base64::Engine;
                f.write_str(&base64::engine::general_purpose::STANDARD.encode(b))
            }
        }
    }
}

impl NvsEntry {
    /// Create a new entry with inline data.
    ///
    /// The encoding is derived automatically from the [`DataValue`] variant.
    pub fn new_data(namespace: String, key: String, value: DataValue) -> Self {
        Self {
            namespace,
            key,
            content: EntryContent::Data(value),
        }
    }

    /// Create a new entry that references an external file.
    ///
    /// The file content will be read and converted according to `encoding`
    /// at partition generation time.
    pub fn new_file(
        namespace: String,
        key: String,
        encoding: FileEncoding,
        file_path: PathBuf,
    ) -> Self {
        Self {
            namespace,
            key,
            content: EntryContent::File {
                encoding,
                file_path,
            },
        }
    }
}

/// Validate that `key` is non-empty and within the NVS maximum key length.
pub(crate) fn validate_key(key: &str) -> Result<(), Error> {
    if key.is_empty() {
        return Err(Error::InvalidKey("key must not be empty".to_string()));
    }
    if key.len() > MAX_KEY_LENGTH {
        return Err(Error::InvalidKey(format!(
            "key '{}' is too long (max {} characters)",
            key, MAX_KEY_LENGTH
        )));
    }
    Ok(())
}
