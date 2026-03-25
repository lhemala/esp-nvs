//! Blob handling types and data structures.
//!
//! This module contains types used for tracking and managing multi-chunk
//! blob storage in NVS.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::Key;
use crate::types::{
    NamespaceIndex,
    VersionOffset,
};

/// Key for the blob index: (namespace, version_offset, key).
pub(crate) type BlobIndexKey = (NamespaceIndex, VersionOffset, Key);

/// Value for the blob index: (optional blob index data, observed chunk data).
pub(crate) type BlobIndexValue = (Option<BlobIndexEntryBlobIndexData>, BlobObservedData);

/// The value will only have multiple entries if we are interrupted while writing an updated blob.
/// Since we clean up on init, there are at most two.
pub(crate) type BlobIndex = BTreeMap<BlobIndexKey, BlobIndexValue>;

/// Data about chunks observed on a specific page.
#[cfg_attr(feature = "debug-logs", derive(Debug))]
pub(crate) struct ChunkData {
    pub(crate) page_sequence: u32,
    pub(crate) chunk_count: u8,
    pub(crate) data_size: u32,
}

/// Observed blob data chunks across pages.
#[cfg_attr(feature = "debug-logs", derive(Debug))]
pub(crate) struct BlobObservedData {
    pub(crate) chunks_by_page: Vec<ChunkData>,
}

/// Blob index entry metadata from the BlobIndex item.
#[cfg_attr(feature = "debug-logs", derive(Debug))]
pub(crate) struct BlobIndexEntryBlobIndexData {
    pub(crate) item_index: u8,
    pub(crate) page_sequence: u32,
    pub(crate) size: u32,
    pub(crate) chunk_count: u8,
}
