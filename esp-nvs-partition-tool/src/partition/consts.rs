// NVS page layout
pub const FLASH_SECTOR_SIZE: usize = 4096;
pub const PAGE_HEADER_SIZE: usize = 32;
pub const ENTRY_STATE_BITMAP_SIZE: usize = 32;
pub const ENTRY_SIZE: usize = 32;
pub const ENTRIES_PER_PAGE: usize = 126;

// Page states
pub const PAGE_STATE_ACTIVE: u32 = 0xFFFFFFFE;
pub const PAGE_STATE_FULL: u32 = 0xFFFFFFFC;
pub const PAGE_STATE_FREEING: u32 = 0xFFFFFFF8;
pub const PAGE_STATE_CORRUPT: u32 = 0x00000000;

// Entry types from ESP-IDF
pub const ITEM_TYPE_U8: u8 = 0x01;
pub const ITEM_TYPE_I8: u8 = 0x11;
pub const ITEM_TYPE_U16: u8 = 0x02;
pub const ITEM_TYPE_I16: u8 = 0x12;
pub const ITEM_TYPE_U32: u8 = 0x04;
pub const ITEM_TYPE_I32: u8 = 0x14;
pub const ITEM_TYPE_U64: u8 = 0x08;
pub const ITEM_TYPE_I64: u8 = 0x18;
pub const ITEM_TYPE_SIZED: u8 = 0x21;
pub const ITEM_TYPE_BLOB: u8 = 0x41; // Legacy single-page blob (version 1 format)
pub const ITEM_TYPE_BLOB_INDEX: u8 = 0x48;
pub const ITEM_TYPE_BLOB_DATA: u8 = 0x42;

// Reserved value for unused fields
pub const RESERVED_U16: u16 = 0xFFFF;

// Entry states
pub const ENTRY_STATE_WRITTEN: u8 = 0b10;

// Maximum data bytes per BLOB_DATA chunk.
// Each chunk uses one header entry + up to (ENTRIES_PER_PAGE - 1) data entries.
pub const MAX_DATA_PER_CHUNK: usize = (ENTRIES_PER_PAGE - 1) * ENTRY_SIZE; // 4000 bytes
