//! Error types for NVS operations.

use thiserror::Error;

pub use crate::raw::ItemType;

/// Errors that can occur during NVS operations. The list is likely to stay as is but marked as
/// non-exhaustive to allow for future additions without breaking the API. A caller would likely
/// only need to handle NamespaceNotFound and KeyNotFound as the other errors are static.
#[derive(Error, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    /// The partition offset has to be aligned to the size of a flash sector (4k)
    #[error("invalid partition offset")]
    InvalidPartitionOffset,

    /// The partition size has to be a non-zero multiple of the flash sector size (4k), and the
    /// partition has to fit the flash it is on
    #[error("invalid partition size")]
    InvalidPartitionSize,

    /// The internal error value is returned from the provided `&mut impl flash::Flash`
    #[error("internal flash error")]
    FlashError,

    /// Namespace not found. Either the flash was corrupted and silently fixed on
    /// startup or no value has been written yet.
    #[error("namespace not found")]
    NamespaceNotFound,

    /// The max namespace length is 15 bytes plus null terminator.
    #[error("namespace too long")]
    NamespaceTooLong,

    /// The namespace is malformed. The last byte must be b'\0'
    #[error("namespace malformed")]
    NamespaceMalformed,

    /// Strings are limited to `MAX_BLOB_DATA_PER_PAGE` bytes.
    ///
    /// Blobs are limited to `MAX_BLOB_SIZE - 1` bytes, that is 507,999. The limit follows from the
    /// 127 chunk indices a blob version can address, each holding at most
    /// `MAX_BLOB_DATA_PER_PAGE` (4,000) bytes. Anything from `MAX_BLOB_SIZE` upwards is rejected on
    /// sight, before a single byte is written.
    ///
    /// The limit does not depend on how full the *active page* is, as long as the partition can
    /// hand out a fresh page: a blob large enough to need every chunk index retires a partially
    /// filled active page first, so all of its chunks are whole.
    ///
    /// On a partition that cannot do that, a blob within the byte limit may still fail. Usually
    /// that is [`Error::FlashFull`], but a partition too small to give the blob whole chunks
    /// runs out of chunk indices and reports `ValueTooLong` for a blob a roomier partition
    /// would accept. That bail-out happens mid-write, so unlike the byte limit it leaves the
    /// written chunks behind as orphans, which the next `Nvs::new` cleans up.
    #[error("value too long")]
    ValueTooLong,

    /// The key is malformed. The last byte must be b'\0'
    #[error("key malformed")]
    KeyMalformed,

    /// The max key length is 15 bytes plus null terminator.
    #[error("key too long")]
    KeyTooLong,

    /// Key not found. Either the flash was corrupted and silently fixed on or no value has been
    /// written yet.
    #[error("key not found")]
    KeyNotFound,

    /// The encountered item type is reported
    #[error("item type mismatch: {0}")]
    ItemTypeMismatch(ItemType),

    /// Blob data is corrupted or inconsistent
    #[error("corrupted data")]
    CorruptedData,

    /// Flash is full and defragmentation doesn't help.
    #[error("flash full")]
    FlashFull,

    /// Used internally to indicate that we have to allocate a new page.
    #[error("page full")]
    PageFull,
}
