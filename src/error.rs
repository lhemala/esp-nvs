use crate::raw;
use thiserror::Error;

pub use raw::ItemType;

/// Errors that can occur during NVS operations. The list is likely to stay as is but marked as
/// non-exhaustive to allow for future additions without breaking the API. A caller would likely only
/// need to handle NamespaceNotFound and KeyNotFound as the other errors are static.
#[derive(Error, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    /// The partition offset has to be aligned to the size of a flash sector (4k)
    #[error("invalid partition offset")]
    InvalidPartitionOffset,

    /// The partition size has to be a multiple of the flash sector size (4k)
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

    /// Strings are limited to `MAX_BLOB_DATA_PER_PAGE` while blobs can be up to `MAX_BLOB_SIZE` bytes
    #[error("value too long")]
    ValueTooLong,

    /// The key is malformed. The last byte must be b'\0'
    #[error("key malformed")]
    KeyMalformed,

    /// The max key length is 15 bytes plus null terminator.
    #[error("key too long")]
    KeyTooLong,

    /// Key not found. Either the flash was corrupted and silently fixed on or no value has been written yet.
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
