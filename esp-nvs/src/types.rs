//! Newtype wrappers for NVS indices, sequences, and keys.
//!
//! These types provide type safety for various index and sequence values
//! used throughout the NVS implementation.

use core::fmt;

/// Maximum Key length is 15 bytes + 1 byte for the null terminator.
pub const MAX_KEY_LENGTH: usize = 15;
pub(crate) const MAX_KEY_NUL_TERMINATED_LENGTH: usize = MAX_KEY_LENGTH + 1;

/// A 16-byte key used for NVS storage (15 characters + null terminator)
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(pub(crate) [u8; MAX_KEY_NUL_TERMINATED_LENGTH]);

impl Key {
    /// Creates a 16 byte, null-padded byte array used as key for values and namespaces.
    ///
    /// Usage: `Key::from_array(b"my_key")`
    ///
    /// Tip: use a const context if possible to ensure that the key is transformed at compile time:
    ///   `let my_key = const { Key::from_array(b"my_key") };`
    pub const fn from_array<const M: usize>(src: &[u8; M]) -> Self {
        assert!(M <= MAX_KEY_LENGTH);
        let mut dst = [0u8; MAX_KEY_NUL_TERMINATED_LENGTH];
        let mut i = 0;
        while i < M {
            dst[i] = src[i];
            i += 1;
        }
        Self(dst)
    }

    /// Creates a 16 byte, null-padded byte array used as key for values and namespaces.
    ///
    /// Usage: `Key::from_slice(b"my_key")`
    ///
    /// Tip: use a const context if possible to ensure that the key is transformed at compile time:
    ///   `let my_key = const { Key::from_slice("my_key".as_bytes()) };`
    pub const fn from_slice(src: &[u8]) -> Self {
        assert!(src.len() <= MAX_KEY_LENGTH);
        let mut dst = [0u8; MAX_KEY_NUL_TERMINATED_LENGTH];
        let mut i = 0;
        while i < src.len() {
            dst[i] = src[i];
            i += 1;
        }
        Self(dst)
    }

    /// Creates a 16 byte, null-padded byte array used as key for values and namespaces.
    ///
    /// Usage: `Key::from_str("my_key")`
    ///
    /// Tip: use a const context if possible to ensure that the key is transformed at compile time:
    ///   `let my_key = const { Key::from_str("my_key") };`
    pub const fn from_str(s: &str) -> Self {
        let bytes = s.as_bytes();
        Self::from_slice(bytes)
    }

    /// Converts a key to a byte array.
    pub const fn as_bytes(&self) -> &[u8; MAX_KEY_NUL_TERMINATED_LENGTH] {
        &self.0
    }

    /// Returns the key as a string slice, excluding null padding.
    pub fn as_str(&self) -> &str {
        let len = self.0.iter().position(|&b| b == 0).unwrap_or(self.0.len());
        // Safety: NVS keys are always valid ASCII/UTF-8
        unsafe { core::str::from_utf8_unchecked(&self.0[..len]) }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // for debug representation, print as binary string
        write!(f, "Key(b\"")?;

        // skip the null terminator at the end, which is always null,
        // and might be confusing in the output if you passed a 15-byte key,
        // and it shows a \0 at the end.
        for &byte in &self.0[..self.0.len() - 1] {
            // escape_default would escape 0 as \x00, but \0 is more readable
            if byte == 0 {
                write!(f, "\\0")?;
                continue;
            }

            write!(f, "{}", core::ascii::escape_default(byte))?;
        }

        write!(f, "\")")
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Key {
    fn format(&self, f: defmt::Formatter) {
        // for defmt representation, print as binary string
        defmt::write!(f, "Key(b\"");

        // skip the null terminator at the end, which is always null,
        // and might be confusing in the output if you passed a 15-byte key,
        // and it shows a \0 at the end. We can't use core::ascii::escape_default
        // for defmt so some characters are manually escaped.
        for &byte in &self.0[..self.0.len() - 1] {
            match byte {
                b'\t' => defmt::write!(f, "\\t"),
                b'\n' => defmt::write!(f, "\\n"),
                b'\r' => defmt::write!(f, "\\r"),
                b'\\' => defmt::write!(f, "\\\\"),
                b'"' => defmt::write!(f, "\\\""),
                0x20..=0x7e => defmt::write!(f, "{}", byte as char),
                _ => defmt::write!(f, "\\x{:02x}", byte),
            }
        }

        defmt::write!(f, "\")");
    }
}

impl AsRef<[u8]> for Key {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Index of an item within a page's entry array.
pub(crate) struct ItemIndex(pub(crate) u8);

impl From<u8> for ItemIndex {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl From<ItemIndex> for u8 {
    fn from(val: ItemIndex) -> Self {
        val.0
    }
}

/// Sequence number for page ordering.
pub(crate) struct PageSequence(pub(crate) u32);

/// Index of a namespace in the namespace table.
#[derive(Ord, PartialOrd, Eq, PartialEq, Copy, Clone)]
#[cfg_attr(feature = "debug-logs", derive(Debug))]
pub(crate) struct NamespaceIndex(pub(crate) u8);

/// Index of a page in the pages vector.
pub(crate) struct PageIndex(pub(crate) usize);

impl From<usize> for PageIndex {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<PageIndex> for usize {
    fn from(val: PageIndex) -> Self {
        val.0
    }
}

/// Chunk index discriminator for blob operations.
#[derive(Clone)]
#[cfg_attr(feature = "debug-logs", derive(Debug))]
pub(crate) enum ChunkIndex {
    Any,
    BlobIndex,
    BlobData(u8),
}

/// Version offset for blob chunk indices, used to distinguish between
/// two versions of a blob during atomic updates.
#[derive(PartialEq, Ord, PartialOrd, Eq, Clone)]
#[cfg_attr(feature = "debug-logs", derive(Debug))]
pub(crate) enum VersionOffset {
    V0 = 0x00,
    V1 = 0x80,
}

impl VersionOffset {
    pub(crate) fn invert(&self) -> VersionOffset {
        if *self == VersionOffset::V0 {
            VersionOffset::V1
        } else {
            VersionOffset::V0
        }
    }
}

impl From<u8> for VersionOffset {
    fn from(value: u8) -> Self {
        if value < VersionOffset::V1 as u8 {
            VersionOffset::V0
        } else {
            VersionOffset::V1
        }
    }
}
