#![doc = include_str!("../README.md")]
#![cfg_attr(not(target_arch = "x86_64"), no_std)]

extern crate alloc;

#[cfg(feature = "encryption")]
pub mod encryption;
pub mod error;
#[cfg(feature = "encryption")]
pub mod keys;
pub mod mem_flash;
pub mod platform;
pub mod raw;

mod blob;
mod compaction;
mod get;
mod init;
mod items;
mod nvs;
mod page;
mod set;
mod statistics;
mod types;
mod u24;

#[cfg(feature = "encryption")]
pub use encryption::{
    EncryptedFlash,
    NVS_KEY_SIZE,
};
pub use get::Get;
pub use nvs::Nvs;
pub use raw::{
    ENTRIES_PER_PAGE,
    ENTRY_STATE_BITMAP_SIZE,
    FLASH_SECTOR_SIZE,
    ITEM_SIZE,
    ItemType,
    MAX_BLOB_DATA_PER_PAGE,
    MAX_BLOB_SIZE,
    PAGE_HEADER_SIZE,
    PageState,
};
pub use set::Set;
pub use statistics::{
    EntryStatistics,
    NvsStatistics,
    PageStatistics,
};
pub use types::{
    Key,
    MAX_KEY_LENGTH,
};
