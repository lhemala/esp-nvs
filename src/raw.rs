#[cfg(feature = "debug-logs")]
use alloc::format;
use core::fmt::{Debug, Formatter};
use core::mem::{size_of, transmute};
use core::slice::from_raw_parts;

use crate::Key;
use crate::error::Error;
use crate::internal::{ThinPageHeader, ThinPageState, VersionOffset};
use crate::platform::FnCrc32;
use crate::u24::u24;

// -1 is for the leading item of type BLOB_DATA or SZ (for str)
pub(crate) const MAX_BLOB_DATA_PER_PAGE: usize = (ENTRIES_PER_PAGE - 1) * size_of::<Item>();
pub(crate) const MAX_BLOB_SIZE: usize =
    MAX_BLOB_DATA_PER_PAGE * (u8::MAX as usize - VersionOffset::V1 as usize);
pub(crate) const FLASH_SECTOR_SIZE: usize = 4096;
pub(crate) const ENTRY_STATE_BITMAP_SIZE: usize = 32;
pub(crate) const ENTRIES_PER_PAGE: usize = 126;

// Compile-time assertion to ensure page structure size matches flash sector size
const _: () = assert!(
    size_of::<PageHeader>() + ENTRY_STATE_BITMAP_SIZE + ENTRIES_PER_PAGE * size_of::<Item>()
        == FLASH_SECTOR_SIZE,
    "Page structure size must equal flash sector size"
);

#[repr(C, packed)]
pub(crate) struct RawPage {
    pub(crate) header: PageHeader,
    pub(crate) entry_state_bitmap: [u8; ENTRY_STATE_BITMAP_SIZE],
    pub(crate) items: Items,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub(crate) union Items {
    pub(crate) raw: [u8; ENTRIES_PER_PAGE * size_of::<Item>()],
    pub(crate) entries: [Item; ENTRIES_PER_PAGE],
}

#[cfg(feature = "debug-logs")]
impl Debug for Items {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        unsafe { self.entries.fmt(f) }
    }
}

#[derive(strum::FromRepr, Debug, PartialEq)]
#[repr(u8)]
pub(crate) enum EntryMapState {
    Empty = 0b11,
    Written = 0b10,
    Erased = 0b00,
    Illegal = 0b01,
}

const PSB_INIT: u32 = 0x1;
const PSB_FULL: u32 = 0x2;
const PSB_FREEING: u32 = 0x4;
const PSB_CORRUPT: u32 = 0x8;

#[derive(strum::FromRepr, strum::Display, Debug, PartialEq, Copy, Clone)]
#[repr(u32)]
pub(crate) enum PageState {
    // All bits set, default state after flash erase. Page has not been initialized yet.
    Uninitialized = u32::MAX,

    // Page is initialized, and will accept writes.
    Active = PageState::Uninitialized as u32 & !PSB_INIT,

    // Page is marked as full and will not accept new writes.
    Full = PageState::Active as u32 & !PSB_FULL,

    // Data is being moved from this page to a new one.
    Freeing = PageState::Full as u32 & !PSB_FREEING,

    // Page was found to be in a corrupt and unrecoverable state.
    // Instead of being erased immediately, it will be kept for diagnostics and data recovery.
    // It will be erased once we run out out free pages.
    Corrupt = PageState::Freeing as u32 & !PSB_CORRUPT,

    // Page object wasn't loaded from flash memory
    Invalid = 0,
}

impl From<PageState> for ThinPageState {
    fn from(val: PageState) -> Self {
        match val {
            PageState::Uninitialized => ThinPageState::Uninitialized,
            PageState::Active => ThinPageState::Active,
            PageState::Full => ThinPageState::Full,
            PageState::Freeing => ThinPageState::Freeing,
            PageState::Corrupt => ThinPageState::Corrupt,
            PageState::Invalid => ThinPageState::Invalid,
        }
    }
}

const PAGE_STATE_UNINITIALIZED: u32 = PageState::Uninitialized as u32;
const PAGE_STATE_ACTIVE: u32 = PageState::Active as u32;
const PAGE_STATE_FULL: u32 = PageState::Full as u32;
const PAGE_STATE_FREEING: u32 = PageState::Freeing as u32;
const PAGE_STATE_CORRUPT: u32 = PageState::Corrupt as u32;
const PAGE_STATE_INVALID: u32 = PageState::Invalid as u32;

impl From<u32> for PageState {
    fn from(val: u32) -> Self {
        match val {
            PAGE_STATE_UNINITIALIZED => PageState::Uninitialized,
            PAGE_STATE_ACTIVE => PageState::Active,
            PAGE_STATE_FULL => PageState::Full,
            PAGE_STATE_FREEING => PageState::Freeing,
            PAGE_STATE_CORRUPT => PageState::Corrupt,
            PAGE_STATE_INVALID => PageState::Invalid,
            _ => PageState::Corrupt,
        }
    }
}

#[derive(strum::FromRepr, strum::Display, Debug, Eq, PartialEq, Copy, Clone)]
#[repr(u8)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ItemType {
    U8 = 0x01,
    I8 = 0x11,
    U16 = 0x02,
    I16 = 0x12,
    U32 = 0x04,
    I32 = 0x14,
    U64 = 0x08,
    I64 = 0x18,
    Sized = 0x21,
    Blob = 0x41,
    BlobData = 0x42,
    BlobIndex = 0x48,
    Any = 0xff,
}

impl ItemType {
    pub(crate) fn get_primitive_bytes_width(&self) -> Result<usize, Error> {
        match self {
            ItemType::U8 | ItemType::I8 => Ok(1),
            ItemType::U16 | ItemType::I16 => Ok(2),
            ItemType::U32 | ItemType::I32 => Ok(4),
            ItemType::U64 | ItemType::I64 => Ok(8),
            _ => Err(Error::ItemTypeMismatch(*self)),
        }
    }
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub(crate) struct PageHeader {
    pub(crate) state: u32,
    pub(crate) sequence: u32,
    pub(crate) version: u8,
    pub(crate) _unused: [u8; 19],
    pub(crate) crc: u32,
}

pub(crate) union PageHeaderRaw {
    pub(crate) page_header: PageHeader,
    pub(crate) raw: [u8; size_of::<PageHeader>()],
}

impl From<PageHeader> for ThinPageHeader {
    fn from(val: PageHeader) -> Self {
        ThinPageHeader {
            state: PageState::from(val.state).into(),
            sequence: val.sequence,
            version: val.version,
            crc: val.crc,
        }
    }
}

impl Debug for PageHeader {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let state = self.state;
        let sequence = self.sequence;
        let version = self.version;
        let crc = self.crc;
        match state {
            PAGE_STATE_FULL | PAGE_STATE_ACTIVE => {
                f.write_fmt(format_args!("PageHeader {{ state: {state:?>13}, sequence: {sequence:>4}, version: 0x{version:0>2x}, crc: 0x{crc:0>4x}}}"))
            }
            _ => f.write_fmt(format_args!("PageHeader {{ state: {state:?>13} }}"))
        }
    }
}

impl PageHeader {
    pub(crate) fn calculate_crc32(&self, crc32: FnCrc32) -> u32 {
        let buf: [u8; 32] = unsafe { transmute(*self) };
        let interesting_parts = &buf[4..28];
        crc32(u32::MAX, interesting_parts)
    }
}

#[repr(C, packed)]
#[derive(Copy, Clone, PartialEq)]
pub(crate) struct Item {
    pub(crate) namespace_index: u8,
    pub(crate) type_: ItemType, // name stolen from the Linux kernel
    pub(crate) span: u8,
    pub(crate) chunk_index: u8,
    pub(crate) crc: u32,
    pub(crate) key: Key,
    pub(crate) data: ItemData,
}
pub(crate) union RawItem {
    pub(crate) item: Item,
    pub(crate) raw: [u8; size_of::<Item>()],
}

impl Item {
    #[cfg(feature = "debug-logs")]
    fn get_primitive(&self) -> Result<u64, Error> {
        let width = match self.type_ {
            ItemType::I8 | ItemType::I16 | ItemType::I32 | ItemType::I64 => {
                (self.type_ as u8) - 0x10
            }
            ItemType::U8 | ItemType::U16 | ItemType::U32 | ItemType::U64 => self.type_ as u8,
            _ => return Err(Error::ItemTypeMismatch(self.type_)),
        };

        let mut mask = 0u64;
        for i in 0..width {
            mask |= u64::from(u8::MAX) << (i * 8)
        }
        Ok(unsafe { self.data.primitive & mask })
    }
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub(crate) union ItemData {
    pub(crate) raw: [u8; 8],
    pub(crate) primitive: u64,
    pub(crate) sized: ItemDataSized,
    pub(crate) blob_index: ItemDataBlobIndex,
}

impl PartialEq for ItemData {
    fn eq(&self, other: &Self) -> bool {
        unsafe { self.raw == other.raw }
    }
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub(crate) struct ItemDataSized {
    pub(crate) size: u16,
    _reserved: u16,
    pub(crate) crc: u32,
}

impl ItemDataSized {
    pub(crate) fn new(size: u16, crc: u32) -> Self {
        Self {
            size,
            _reserved: u16::MAX,
            crc,
        }
    }
}

#[cfg(feature = "debug-logs")]
impl Debug for ItemDataSized {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let size = self.size;
        let crc = self.crc;
        f.write_fmt(format_args!(
            "ItemDataSized {{size: {size}, crc: {crc:0>8x}}}"
        ))
    }
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub(crate) struct ItemDataBlobIndex {
    pub(crate) size: u32,
    pub(crate) chunk_count: u8,
    pub(crate) chunk_start: u8,
}

#[cfg(feature = "debug-logs")]
impl Debug for ItemDataBlobIndex {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let size = self.size;
        let chunk_count = self.chunk_count;
        let chunk_start = self.chunk_start;
        f.write_fmt(format_args!("ItemDataBlobIndex {{size: {size}, chunk_count: {chunk_count}, chunk_start: {chunk_start}}}"))
    }
}

impl Item {
    pub(crate) fn calculate_hash(&self, crc32: FnCrc32) -> u24 {
        Self::calculate_hash_ref(crc32, self.namespace_index, &self.key, self.chunk_index)
    }

    /// `calculate_hash_ref` follows the details of the C++ implementation and accepts more collisions in
    /// favor of memory efficiency
    pub(crate) fn calculate_hash_ref(
        crc32: FnCrc32,
        namespace_index: u8,
        key: &Key,
        chunk_index: u8,
    ) -> u24 {
        let mut result = u32::MAX;
        result = crc32(result, &[namespace_index]);
        result = crc32(result, unsafe {
            from_raw_parts(key.0.as_ptr(), key.0.len())
        });
        result = crc32(result, &[chunk_index]);
        u24::from_u32(result & 0xFFFFFF)
    }

    pub(crate) fn calculate_crc32(&self, crc32: FnCrc32) -> u32 {
        let buf: [u8; 32] = unsafe { transmute(*self) };
        let mut result = u32::MAX;
        result = crc32(result, &buf[0..4]);
        result = crc32(result, self.key.0.as_ref());
        result = unsafe { crc32(result, &self.data.raw) };
        result
    }
}

#[cfg(feature = "debug-logs")]
impl Debug for Item {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let ns_idx = self.namespace_index;
        let type_ = self.type_;
        let span = self.span;
        let chunk_index = self.chunk_index;
        let crc = self.crc;
        let key = slice_with_nullbytes_to_str(&self.key);
        let value = match type_ {
            ItemType::Sized | ItemType::BlobData => unsafe { format!("{:?}", self.data.sized) },
            ItemType::Blob | ItemType::BlobIndex => unsafe {
                format!("{:?}", self.data.blob_index)
            },
            ItemType::I8
            | ItemType::I16
            | ItemType::I32
            | ItemType::I64
            | ItemType::U8
            | ItemType::U16
            | ItemType::U32
            | ItemType::U64 => format!("{}", self.get_primitive().unwrap()),
            ItemType::Any => unsafe { format!("{:?}", self.data.raw) },
        };
        f.write_fmt(format_args!("Item {{ns_idx: {ns_idx}, type: {type_:<9?}, span: {span}, chunk_idx: {chunk_index:>3}, crc: {crc:0>8x}, key: '{key}', value: {value}}}"))
    }
}

/// We know that keys and namespace names are saved in 16 byte arrays. Because they are originally C strings
/// they are followed by a null terminator in case they are shorter than 16 byte. We have to slice
/// before the null terminator if we want to transmute them to a str.
#[cfg(feature = "debug-logs")]
pub(crate) fn slice_with_nullbytes_to_str(raw: &[u8]) -> &str {
    let sliced = match raw.iter().position(|&e| e == 0x00) {
        None => raw,
        Some(idx) => &raw[..idx],
    };
    unsafe { core::str::from_utf8_unchecked(sliced) }
}
