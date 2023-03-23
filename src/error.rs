use crate::raw;

pub use raw::ItemType;

/// Errors that can occur during NVS operations. The list is likely to stay as is but marked as
/// non-exhaustive to allow for future additions without breaking the API. A caller would likely only
/// need to handle NamespaceNotFound and KeyNotFound as the other errors are static.
#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    /// The partition offset has to be aligned to the size of a flash sector (4k)
    InvalidPartitionOffset,
    /// The partition size has to be a multiple of the flash sector size (4k)
    InvalidPartitionSize,
    /// The internal error value is returned from the provided `&mut impl flash::Flash`
    FlashError,
    /// Namespace not found. Either the flash was corrupted and silently fixed on
    /// startup or no value has been written yet.
    NamespaceNotFound,
    /// The max namespace length is 15 bytes plus null terminator.
    NamespaceTooLong,
    /// The namespace is malformed. The last byte must be b'\0'
    NamespaceMalformed,
    /// Strings are limited to `MAX_BLOB_DATA_PER_PAGE` while blobs can be up to `MAX_BLOB_SIZE` bytes
    ValueTooLong,
    /// The key is malformed. The last byte must be b'\0'
    KeyMalformed,
    /// The max key length is 15 bytes plus null terminator.
    KeyTooLong,
    /// Key not found. Either the flash was corrupted and silently fixed on or no value has been written yet.
    KeyNotFound,
    /// The encountered item type is reported
    ItemTypeMismatch(ItemType),
    /// Blob data is corrupted or inconsistent
    CorruptedData,
    /// Flash is full and defragmentation doesn't help.
    FlashFull,
    /// Used internally to indicate that we have to allocate a new page.
    PageFull,
}
