use crate::Key;
use crate::error::Error;
#[cfg(feature = "debug-logs")]
use crate::raw::slice_with_nullbytes_to_str;
use crate::raw::{
    ENTRIES_PER_PAGE, ENTRY_STATE_BITMAP_SIZE, EntryMapState, FLASH_SECTOR_SIZE, Item, ItemData,
    ItemDataBlobIndex, ItemType, MAX_BLOB_DATA_PER_PAGE, MAX_BLOB_SIZE, PageHeader, PageHeaderRaw,
    PageState, RawItem, RawPage, write_aligned,
};
use crate::u24::u24;
use crate::{Nvs, raw};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cmp;
use core::cmp::Ordering;
#[cfg(feature = "debug-logs")]
use core::fmt::{Debug, Formatter};
use core::mem::size_of;
use core::ops::Range;

use crate::error::Error::{ItemTypeMismatch, KeyNotFound, PageFull};
use crate::platform::{AlignedOps, Platform};
use alloc::collections::BTreeMap;
use core::mem;
use core::mem::offset_of;
use core::ops::Not;
#[cfg(feature = "defmt")]
use defmt::trace;
#[cfg(feature = "defmt")]
use defmt::warn;

/// Maximum Key length is 15 bytes + 1 byte for the null terminator.
/// Shorter keys need to be padded with null bytes.
pub(crate) const MAX_KEY_LENGTH: usize = 15;

type BlobIndexKey = (NamespaceIndex, VersionOffset, Key);
type BlobIndexValue = (Option<BlobIndexEntryBlobIndexData>, BlobObservedData);
/// The value will only have multiple entries if we are interrupted while writing an updated blob.
/// Since we clean up on init, there are at most two.
type BlobIndex = BTreeMap<BlobIndexKey, BlobIndexValue>;

pub(crate) struct ItemIndex(pub(crate) u8);

struct PageSequence(u32);

#[derive(Ord, PartialOrd, Eq, PartialEq, Copy, Clone)]
#[cfg_attr(feature = "debug-logs", derive(Debug))]
struct NamespaceIndex(u8);

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

#[derive(Clone)]
#[cfg_attr(feature = "debug-logs", derive(Debug))]
pub(crate) enum ChunkIndex {
    Any,
    BlobIndex,
    BlobData(u8),
}

#[derive(PartialEq, Ord, PartialOrd, Eq, Clone)]
#[cfg_attr(feature = "debug-logs", derive(Debug))]
pub(crate) enum VersionOffset {
    V0 = 0x00,
    V1 = 0x80,
}

impl VersionOffset {
    fn invert(&self) -> VersionOffset {
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

#[cfg_attr(feature = "debug-logs", derive(Debug))]
struct ChunkData {
    page_sequence: u32,
    chunk_count: u8,
    data_size: u32,
}

#[cfg_attr(feature = "debug-logs", derive(Debug))]
struct BlobObservedData {
    chunks_by_page: Vec<ChunkData>,
}

#[cfg_attr(feature = "debug-logs", derive(Debug))]
struct BlobIndexEntryBlobIndexData {
    item_index: u8,
    page_sequence: u32,
    size: u32,
    chunk_count: u8,
}

pub(crate) struct ThinPage {
    pub(crate) address: usize,
    header: ThinPageHeader,
    entry_state_bitmap: [u8; ENTRY_STATE_BITMAP_SIZE],
    item_hash_list: Vec<ItemHashListEntry>,
    erased_entry_count: u8,
    used_entry_count: u8,
}

impl ThinPage {
    pub(crate) fn uninitialized(address: usize) -> Self {
        Self {
            address,
            header: ThinPageHeader::uninitialzed(),
            entry_state_bitmap: [0xFF; 32],
            item_hash_list: vec![],
            erased_entry_count: 0,
            used_entry_count: 0,
        }
    }

    pub(crate) fn initialize<T: Platform>(
        &mut self,
        hal: &mut T,
        next_sequence: u32,
    ) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("initialize: @{:#08x}", self.address);

        #[cfg(feature = "debug-logs")]
        println!("  ThinPage: initialize {:#08x}", self.address);

        let mut raw_header = PageHeader {
            state: PageState::Active as u32,
            sequence: next_sequence,
            version: 0xFE,
            _unused: [0xFF; 19],
            crc: 0,
        };
        let crc = raw_header.calculate_crc32(T::crc32);
        raw_header.crc = crc;

        let raw_header = PageHeaderRaw {
            page_header: raw_header,
        };

        write_aligned::<T>(hal, self.address as u32, unsafe { &raw_header.raw })
            .map_err(|_| Error::FlashError)?;

        self.header.state = ThinPageState::Active;
        self.header.version = 0xFE;
        self.header.sequence = next_sequence;
        self.header.crc = crc;

        Ok(())
    }

    pub(crate) fn mark_as_full<T: Platform>(&mut self, hal: &mut T) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("mark_as_full: @{:#08x}", self.address);

        #[cfg(feature = "debug-logs")]
        println!("  ThinPage: mark_as_full");

        let raw = (PageState::Full as u32).to_le_bytes();

        write_aligned(hal, self.address as u32, &raw).map_err(|_| Error::FlashError)?;

        self.header.state = ThinPageState::Full;

        Ok(())
    }

    pub(crate) fn load_item<T: Platform>(
        &self,
        hal: &mut T,
        item_index: u8,
    ) -> Result<Item, Error> {
        #[cfg(feature = "defmt")]
        trace!("load_item: @{:#08x}[{}]", self.address, item_index);

        let mut buf = [0u8; size_of::<Item>()];
        hal.read(
            (self.address + offset_of!(RawPage, items) + size_of::<Item>() * item_index as usize)
                as _,
            &mut buf,
        )
        .map_err(|_| Error::FlashError)?;

        if buf.iter().all(|&it| it == 0xFF) {
            return Err(KeyNotFound);
        }

        // Safety: we check the crc afterwards
        let item = unsafe { mem::transmute::<[u8; 32], Item>(buf) };

        if item.crc != item.calculate_crc32(T::crc32) {
            return Err(KeyNotFound);
        }

        Ok(item)
    }
}

impl ThinPage {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_item<T: Platform>(
        &mut self,
        hal: &mut T,
        namespace_index: u8,
        key: Key,
        type_: ItemType,
        chunk_index: Option<u8>,
        span: u8,
        item_data: ItemData,
    ) -> Result<(), Error> {
        let mut item = Item {
            namespace_index,
            type_,
            span,
            chunk_index: chunk_index.unwrap_or(u8::MAX),
            crc: 0,
            key,
            data: item_data,
        };
        item.crc = item.calculate_crc32(T::crc32);

        let item_index = self.get_next_free_entry();
        let target_addr =
            self.address + offset_of!(RawPage, items) + size_of::<Item>() * item_index;

        #[cfg(feature = "defmt")]
        trace!("load_item: @{:#08x}[{}]", self.address, item_index);

        #[cfg(feature = "debug-logs")]
        println!("  internal: write_item: target_addr: 0x{target_addr:0>8x}");

        let raw_item = RawItem { item };
        write_aligned(hal, target_addr as _, unsafe { &raw_item.raw })
            .map_err(|_| Error::FlashError)?;

        self.set_entry_state(hal, item_index, EntryMapState::Written)?;

        self.used_entry_count += span;

        // Add to hash list if this is not a namespace entry (namespace_index == 0)
        if namespace_index != 0 {
            self.item_hash_list.push(ItemHashListEntry {
                hash: item.calculate_hash(T::crc32),
                index: item_index as u8,
            });
        }

        // Check if page is now full by trying to find the next free entry
        if self.get_next_free_entry() == ENTRIES_PER_PAGE {
            self.mark_as_full::<T>(hal)?;
        }

        Ok(())
    }

    pub(crate) fn write_namespace<T: Platform>(
        &mut self,
        hal: &mut T,
        key: Key,
        value: u8,
    ) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("write_namespace: @{:#08x}", self.address);

        let mut buf = [u8::MAX; 8];
        buf[..1].copy_from_slice(&value.to_le_bytes());
        self.write_item::<T>(hal, 0, key, ItemType::U8, None, 1, ItemData { raw: buf })
    }

    pub(crate) fn write_variable_sized_item<T: Platform>(
        &mut self,
        hal: &mut T,
        namespace_index: u8,
        key: Key,
        type_: ItemType,
        chunk_index: Option<u8>,
        data: &[u8],
    ) -> Result<(), Error> {
        #[cfg(feature = "debug-logs")]
        println!("internal: write_variable_sized_item");

        let data_entries = if data.len().is_multiple_of(size_of::<Item>()) {
            data.len() / size_of::<Item>()
        } else {
            data.len() / size_of::<Item>() + 1
        };
        let span = data_entries + 1;

        if span > ENTRIES_PER_PAGE {
            return Err(Error::ValueTooLong);
        }
        if span > self.get_free_entry_count() {
            return Err(PageFull);
        }

        // Check if we have enough contiguous empty entries
        let start_index = self.get_next_free_entry();

        let item_data = ItemData {
            sized: raw::ItemDataSized::new(data.len() as _, T::crc32(u32::MAX, data)),
        };

        let mut item = Item {
            namespace_index,
            type_,
            span: span as u8,
            chunk_index: chunk_index.unwrap_or(u8::MAX),
            crc: 0,
            key,
            data: item_data,
        };
        item.crc = item.calculate_crc32(T::crc32);

        #[cfg(feature = "defmt")]
        trace!(
            "write_variable_sized_item: @{:#08x}[{}-{}]",
            self.address,
            start_index,
            start_index + span - 1
        );

        // Write the header entry
        let header_addr =
            self.address + offset_of!(RawPage, items) + size_of::<Item>() * start_index;
        let raw_item = RawItem { item };

        write_aligned(hal, header_addr as _, unsafe { &raw_item.raw })
            .map_err(|_| Error::FlashError)?;

        let data_addr = header_addr + size_of::<Item>();
        write_aligned(hal, data_addr as _, data).map_err(|_| Error::FlashError)?;

        self.set_entry_state_range(
            hal,
            start_index as u8..(start_index + span) as u8,
            EntryMapState::Written,
        )?;

        self.item_hash_list.push(ItemHashListEntry {
            hash: item.calculate_hash(T::crc32),
            index: start_index as u8,
        });
        self.used_entry_count += span as u8;

        if start_index + span == ENTRIES_PER_PAGE {
            self.mark_as_full::<T>(hal)?;
        }

        Ok(())
    }

    fn load_referenced_data<T: Platform>(
        &self,
        hal: &mut T,
        // this is the index of the given &Item, not the start of the data which is +1
        item_index: u8,
        item: &Item,
    ) -> Result<Vec<u8>, Error> {
        #[cfg(feature = "defmt")]
        trace!(
            "load_referenced_data: @{:#08x}[{}-{}]",
            self.address,
            item_index + 1,
            item_index + item.span
        );

        #[cfg(feature = "debug-logs")]
        println!("internal: load_item_data");

        match item.type_ {
            ItemType::Sized | ItemType::BlobData => {}
            _ => return Err(ItemTypeMismatch(item.type_)),
        }

        let size = unsafe { item.data.sized.size } as usize;
        let aligned_size = T::align_read(size);

        let mut buf = Vec::with_capacity(aligned_size);
        // Safety: we just allocated the buffer with the exact size we need and we will override it the the call to hal.read()
        unsafe {
            Vec::set_len(&mut buf, aligned_size);
        }
        hal.read(
            (self.address
                + offset_of!(RawPage, items)
                + size_of::<Item>() * (item_index as usize + 1)) as _,
            &mut buf,
        )
        .map_err(|_| Error::FlashError)?;

        // Safety: we allocated aligned_size bytes which is always more than size
        unsafe {
            Vec::set_len(&mut buf, size);
        }

        Ok(buf)
    }

    fn set_entry_state<T: Platform>(
        &mut self,
        hal: &mut T,
        item_index: usize,
        state: EntryMapState,
    ) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!(
            "set_entry_state: @{:#08x}[{}]: {}",
            self.address, item_index, state
        );

        #[cfg(feature = "debug-logs")]
        println!("internal: set_entry_state");

        self.set_entry_state_range(hal, (item_index as u8)..(item_index as u8 + 1), state)
    }

    fn get_entry_state(&self, item_index: u8) -> EntryMapState {
        let idx = item_index / 4;
        let byte = self.entry_state_bitmap[idx as usize];
        let two_bits = (byte >> ((item_index % 4) * 2)) & 0b11;

        let state = EntryMapState::from_repr(two_bits).unwrap();

        #[cfg(feature = "defmt")]
        trace!(
            "get_entry_state: @{:#08x}[{}]: {}",
            self.address, item_index, state
        );

        #[cfg(feature = "debug-logs")]
        println!(
            "internal: get_item_state @{:#08x}[{item_index}]: {state:?}",
            self.address,
        );

        state
    }

    fn set_entry_state_range<T: Platform>(
        &mut self,
        hal: &mut T,
        indices: Range<u8>,
        state: EntryMapState,
    ) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!(
            "set_entry_state_range: @{:#08x}[{}-{}]: {}",
            self.address, indices.start, indices.end, state
        );

        let raw_state = state as u8;
        for item_index in indices.clone() {
            let mask = 0b11u8 << ((item_index % 4) * 2);
            let bits = raw_state << ((item_index % 4) * 2);
            let masked_bits = bits | !mask;

            let offset_in_map = item_index / 4;
            self.entry_state_bitmap[offset_in_map as usize] &= masked_bits;
        }

        let start_byte = (indices.start / 4) as usize;
        let end_byte = ((indices.end - 1) / 4) as usize;

        let aligned_start_byte = T::align_write_floor(start_byte);
        let aligned_end_byte = T::align_write_ceil(end_byte + 1);

        let offset_in_raw_flash =
            self.address + offset_of!(RawPage, entry_state_bitmap) + start_byte;
        let aligned_offset_in_raw_flash = T::align_write_floor(offset_in_raw_flash) as _;

        #[cfg(feature = "debug-logs")]
        println!(
            "  internal: set_entry_state_range: {:>3}..<{:>3} [0x{offset_in_raw_flash:0>4x}]",
            indices.start, indices.end
        );

        write_aligned(
            hal,
            aligned_offset_in_raw_flash,
            &self.entry_state_bitmap[aligned_start_byte..aligned_end_byte],
        )
        .map_err(|_| Error::FlashError)
    }

    fn get_next_free_entry(&self) -> usize {
        self.used_entry_count as usize + self.erased_entry_count as usize
    }

    fn get_free_entry_count(&self) -> usize {
        ENTRIES_PER_PAGE - self.get_next_free_entry()
    }

    fn is_full(&self) -> bool {
        self.get_next_free_entry() == ENTRIES_PER_PAGE
    }

    pub(crate) fn get_state(&self) -> &ThinPageState {
        &self.header.state
    }

    pub(crate) fn get_entry_statistics(&self) -> (u32, u32, u32, u32) {
        let mut empty = 0u32;
        let mut written = 0u32;
        let mut erased = 0u32;
        let mut illegal = 0u32;

        for i in 0..ENTRIES_PER_PAGE as u8 {
            match self.get_entry_state(i) {
                EntryMapState::Empty => empty += 1,
                EntryMapState::Written => written += 1,
                EntryMapState::Erased => erased += 1,
                EntryMapState::Illegal => illegal += 1,
            }
        }

        (empty, written, erased, illegal)
    }

    pub(crate) fn erase_item<T: Platform>(
        &mut self,
        hal: &mut T,
        item_index: u8,
        span: u8,
    ) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!(
            "erase_item: @{:#08x}[{}-{}]",
            self.address,
            item_index,
            item_index + span
        );

        #[cfg(feature = "debug-logs")]
        println!("internal: erase_item");

        self.set_entry_state_range(hal, item_index..(item_index + span), EntryMapState::Erased)?;

        self.erased_entry_count += span;
        self.used_entry_count -= span;
        self.item_hash_list
            .retain(|entry| entry.index != item_index);

        Ok(())
    }
}

impl PartialEq<Self> for ThinPage {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address
    }
}

impl PartialOrd<Self> for ThinPage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for ThinPage {}

impl Ord for ThinPage {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.header.state, &other.header.state) {
            (ThinPageState::Uninitialized, ThinPageState::Uninitialized) => {
                other.address.cmp(&self.address)
            }
            (ThinPageState::Uninitialized, _) => Ordering::Greater,
            (_, ThinPageState::Uninitialized) => Ordering::Less,
            (_, _) => other.header.sequence.cmp(&self.header.sequence),
        }
    }
}

#[cfg(feature = "debug-logs")]
impl Debug for ThinPage {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let address = self.address;
        let header = &self.header;
        f.write_fmt(format_args!(
            "Page {{ address: 0x{address:0>8x} {header:?} "
        ))?;
        match header.state {
            ThinPageState::Full | ThinPageState::Active => (),
            _ => {
                return f.write_fmt(format_args!("}}"));
            }
        }

        let erased_entry_count = self.erased_entry_count;
        let used_entry_count = self.used_entry_count;
        let entry_hash_list_len = self.item_hash_list.len();
        f.write_fmt(format_args!("erased_entry_count: {erased_entry_count}, used_entry_count: {used_entry_count}, entry_hash_list_len: {entry_hash_list_len}}}"))
    }
}

pub(crate) struct ThinPageHeader {
    pub(crate) state: ThinPageState,
    pub(crate) sequence: u32,
    pub(crate) version: u8,
    pub(crate) crc: u32,
}

impl ThinPageHeader {
    fn uninitialzed() -> Self {
        Self {
            state: ThinPageState::Uninitialized,
            sequence: 0,
            version: 0,
            crc: 0,
        }
    }
}

#[cfg(feature = "debug-logs")]
impl Debug for ThinPageHeader {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let state = &self.state;
        let sequence = self.sequence;
        let version = self.version;
        let crc = self.crc;
        match state {
            ThinPageState::Full | ThinPageState::Active => {
                f.write_fmt(format_args!("PageHeader {{ state: {state:>13}, sequence: {sequence:>4}, version: 0x{version:0>2x}, crc: 0x{crc:0>4x}}}"))
            }
            _ => f.write_fmt(format_args!("PageHeader {{ state: {state:>13} }}"))
        }
    }
}

#[derive(strum::Display, PartialEq)]
pub(crate) enum ThinPageState {
    Uninitialized,
    Active,
    Full,
    Freeing,
    Corrupt,
    Invalid,
}

struct ItemHashListEntry {
    pub(crate) hash: u24,
    pub(crate) index: u8,
}

enum LoadPageResult {
    Empty(ThinPage),
    Used(ThinPage, Vec<Namespace>, BlobIndex),
}

struct Namespace {
    name: Key,
    index: u8,
}

impl<T> Nvs<T>
where
    T: Platform,
{
    pub(crate) fn get_primitive(
        &mut self,
        namespace: &Key,
        key: &Key,
        type_: ItemType,
    ) -> Result<u64, Error> {
        #[cfg(feature = "defmt")]
        trace!("get_primitive");

        #[cfg(feature = "debug-logs")]
        println!("internal: get_primitive");

        if key.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::KeyMalformed);
        }
        if namespace.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::NamespaceMalformed);
        }

        let namespace_index = *self
            .namespaces
            .get(namespace)
            .ok_or(Error::NamespaceNotFound)?;

        let (_, _, item) = self.load_item(namespace_index, ChunkIndex::Any, key)?;

        if item.type_ != type_ {
            return Err(ItemTypeMismatch(item.type_));
        }
        Ok(u64::from_le_bytes(unsafe { item.data.raw }))
    }

    pub(crate) fn get_string(&mut self, namespace: &Key, key: &Key) -> Result<String, Error> {
        #[cfg(feature = "defmt")]
        trace!("get_string");

        #[cfg(feature = "debug-logs")]
        println!("internal: get_string");

        if key.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::KeyMalformed);
        }
        if namespace.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::NamespaceMalformed);
        }

        let namespace_index = *self
            .namespaces
            .get(namespace)
            .ok_or(Error::NamespaceNotFound)?;

        let (page_index, item_index, item) =
            self.load_item(namespace_index, ChunkIndex::Any, key)?;

        if item.type_ != ItemType::Sized {
            return Err(ItemTypeMismatch(item.type_));
        }

        let page = &self.pages[page_index.0];
        let data = page.load_referenced_data(&mut self.hal, item_index.0, &item)?;

        let crc = unsafe { item.data.sized.crc };
        if crc != T::crc32(u32::MAX, &data) {
            return Err(Error::KeyNotFound);
        }

        let str =
            core::str::from_utf8(&data[..data.len() - 1]).map_err(|_| Error::CorruptedData)?; // we don't want the null terminator
        Ok(str.to_string())
    }

    pub(crate) fn get_blob(&mut self, namespace: &Key, key: &Key) -> Result<Vec<u8>, Error> {
        #[cfg(feature = "defmt")]
        trace!("get_blob");

        #[cfg(feature = "debug-logs")]
        println!("internal: get_blob");

        if key.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::KeyMalformed);
        }
        if namespace.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::NamespaceMalformed);
        }

        let namespace_index = *self
            .namespaces
            .get(namespace)
            .ok_or(Error::NamespaceNotFound)?;

        let (_page_index, _item_index, item) =
            self.load_item(namespace_index, ChunkIndex::Any, key)?;

        if item.type_ != ItemType::BlobIndex {
            return Err(ItemTypeMismatch(item.type_));
        }

        let size = unsafe { item.data.blob_index.size };

        if size as usize > MAX_BLOB_SIZE {
            return Err(Error::CorruptedData);
        }

        let chunk_count = unsafe { item.data.blob_index.chunk_count };
        let chunk_start = unsafe { item.data.blob_index.chunk_start };

        let mut buf = vec![0u8; size as usize];
        let mut offset = 0usize;

        for chunk in chunk_start..chunk_start + chunk_count {
            // Bounds check before slicing
            if offset >= buf.len() {
                return Err(Error::CorruptedData); // Blob metadata inconsistent - would read beyond buffer
            }

            let (page_index, item_index, item) =
                self.load_item(namespace_index, ChunkIndex::BlobData(chunk), key)?;

            if item.type_ != ItemType::BlobData {
                return Err(ItemTypeMismatch(item.type_));
            }

            let page = &self.pages[page_index.0];
            let data = page.load_referenced_data(&mut self.hal, item_index.0, &item)?;

            let data_crc = unsafe { item.data.sized.crc };
            if data_crc != T::crc32(u32::MAX, &data) {
                return Err(Error::CorruptedData);
            }

            let read_bytes = data.len().min(buf.len() - offset);
            buf[offset..offset + read_bytes].copy_from_slice(&data[..read_bytes]);
            offset += read_bytes;
        }

        Ok(buf)
    }

    pub(crate) fn delete_key(
        &mut self,
        namespace_index: u8,
        key: &Key,
        chunk_index: ChunkIndex,
    ) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("delete_key");

        #[cfg(feature = "debug-logs")]
        println!("internal: delete_key");

        let (page_index, item_index, item) =
            self.load_item(namespace_index, chunk_index.clone(), key)?;

        let page = self.pages.get_mut(page_index.0).unwrap();

        page.erase_item::<T>(&mut self.hal, item_index.0, item.span)?;

        // If we deleted a BLOB_IDX we need to delete all associated BLOB_DATA entries
        if item.type_ == ItemType::BlobIndex {
            self.delete_blob_data(item.namespace_index, key, unsafe {
                VersionOffset::from(item.data.blob_index.chunk_start)
            })?;
        }

        Ok(())
    }

    fn delete_blob_data(
        &mut self,
        namespace_index: u8,
        key: &Key,
        chunk_start: VersionOffset,
    ) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("delete_blob_data");

        #[cfg(feature = "debug-logs")]
        println!("internal: delete_blob_data");

        let raw_chunk_start = chunk_start.clone() as u8;
        // Attempt to delete all BLOB_DATA chunks, but don't fail if some are missing
        for chunk in raw_chunk_start..(raw_chunk_start + (VersionOffset::V1 as u8 - 1)) {
            match self.delete_key(namespace_index, key, ChunkIndex::BlobData(chunk)) {
                Ok(_) => continue,
                Err(Error::KeyNotFound) => {
                    #[cfg(feature = "debug-logs")]
                    println!("internal: delete_blob_data: chunk {} not found", chunk);
                    // Chunk not found - could be corrupted or already deleted; continue
                    continue;
                }
                Err(e) => {
                    // Propagate other errors (like FlashError)
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    fn blob_is_equal(
        &mut self,
        namespace_index: u8,
        key: &Key,
        blob_item: &Item,
        data: &[u8],
    ) -> Result<bool, Error> {
        #[cfg(feature = "defmt")]
        trace!("blob_is_equal");

        #[cfg(feature = "debug-logs")]
        println!("internal: blob_is_equal");

        let blob_index_data = unsafe { blob_item.data.blob_index };
        if blob_index_data.size as usize != data.len() {
            return Ok(false);
        }

        let mut to_be_compared = data;
        let chunks = blob_index_data.chunk_count;
        let chunk_start = blob_index_data.chunk_start;

        for chunk_index in (chunk_start..chunk_start + chunks).rev() {
            let (_page_index, item_index, item) =
                self.load_item(namespace_index, ChunkIndex::BlobData(chunk_index), key)?;

            if item.type_ != ItemType::BlobData {
                return Ok(false);
            }

            let sized = unsafe { item.data.sized };
            if sized.size as usize > to_be_compared.len() {
                return Ok(false);
            }

            let page = &self.pages[_page_index.0];
            let chunk_data = page.load_referenced_data(&mut self.hal, item_index.0, &item)?;

            if sized.crc != T::crc32(u32::MAX, &chunk_data) {
                return Ok(false);
            }

            let offset = to_be_compared.len() - sized.size as usize;
            let expected_chunk_data = &to_be_compared[offset..];

            if chunk_data != expected_chunk_data {
                return Ok(false);
            }

            to_be_compared = &to_be_compared[..offset];
        }

        Ok(true)
    }

    fn find_existing_blob_version(&mut self, namespace: &Key, key: &Key) -> Option<VersionOffset> {
        #[cfg(feature = "defmt")]
        trace!("find_existing_blob_version");

        #[cfg(feature = "debug-logs")]
        println!("internal: find_existing_blob_version");

        let namespace_index = match self.namespaces.get(namespace) {
            Some(&idx) => idx,
            None => return None,
        };

        // Try to find an existing blob index (any version)
        match self.load_item(namespace_index, ChunkIndex::Any, key) {
            Ok((_page_index, _item_index, item)) => {
                if item.type_ == ItemType::BlobIndex {
                    Some(VersionOffset::from(unsafe {
                        item.data.blob_index.chunk_start
                    }))
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }

    pub(crate) fn set_primitive(
        &mut self,
        namespace: &Key,
        key: Key,
        type_: ItemType,
        value: u64,
    ) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("set_primitive");

        #[cfg(feature = "debug-logs")]
        println!("internal: set_primitive");

        if key.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::KeyMalformed);
        }
        if namespace.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::NamespaceMalformed);
        }

        let width = type_.get_primitive_bytes_width()?;
        let mut raw_value = [0xFF; 8];
        raw_value[..width].copy_from_slice(&value.to_le_bytes()[..width]);

        let mut page = self.get_active_page()?;
        let namespace_index = self.get_or_create_namespace(namespace, &mut page)?;

        // page might be full after creating a new namespace
        if page.is_full() {
            page.mark_as_full(&mut self.hal)?;
            page = self.get_active_page()?;
        }

        // the active page needs to be in the vec for it to be considered by load_item()
        self.pages.push(page);

        let old_entry_location = if let Ok((page_index, item_index, item)) =
            self.load_item(namespace_index, ChunkIndex::Any, &key)
        {
            if unsafe { item.data.raw } == raw_value {
                #[cfg(feature = "debug-logs")]
                println!("internal: set_primitive: entry already exists and matches");
                return Ok(());
            }

            #[cfg(feature = "debug-logs")]
            println!("internal: set_primitive: entry already exists and needs to be removed");

            Some((page_index, item_index))
        } else {
            None
        };

        // safe since we just pushed before
        page = self.pages.pop().unwrap();

        page.write_item::<T>(
            &mut self.hal,
            namespace_index,
            key,
            type_,
            None,
            1,
            ItemData { raw: raw_value },
        )?;

        // the page index of the old page might point to this one, so we just push it here already
        // just in case
        self.pages.push(page);

        if let Some((page_index, item_index)) = old_entry_location {
            // page_index might only change on defragmentation when load_active_page()
            // is called after we got it
            let old_page = self.pages.get_mut(page_index.0).unwrap();
            old_page.erase_item(&mut self.hal, item_index.0, 1)?;
        }

        Ok(())
    }

    pub(crate) fn set_str(&mut self, namespace: &Key, key: Key, value: &str) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("set_str");

        #[cfg(feature = "debug-logs")]
        println!("internal: set_str");

        if key.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::KeyMalformed);
        }
        if namespace.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::NamespaceMalformed);
        }

        if value.len() + 1 > MAX_BLOB_DATA_PER_PAGE {
            return Err(Error::ValueTooLong);
        }

        let mut buf = Vec::with_capacity(value.len() + 1);
        buf.extend_from_slice(value.as_bytes());
        buf.push(b'\0');

        // Check if the value already exists and matches (only if namespace exists)
        let old_entry_location = if let Some(&namespace_index) = self.namespaces.get(namespace) {
            match self.load_item(namespace_index, ChunkIndex::Any, &key) {
                Ok((page_index, item_index, item)) => {
                    if item.type_ != ItemType::Sized {
                        Some((page_index, item_index))
                    } else {
                        // Check if the data matches
                        let page = &self.pages[page_index.0];
                        let data = page.load_referenced_data(&mut self.hal, item_index.0, &item)?;

                        let crc = unsafe { item.data.sized.crc };
                        if crc == T::crc32(u32::MAX, &buf) && data == buf {
                            return Ok(());
                        }
                        Some((page_index, item_index))
                    }
                }
                Err(Error::FlashError) => return Err(Error::FlashError),
                Err(_) => None,
            }
        } else {
            None
        };

        // Load active page for writing using ThinPage
        let mut page = self.get_active_page()?;
        let namespace_index = self.get_or_create_namespace(namespace, &mut page)?;

        match page.write_variable_sized_item::<T>(
            &mut self.hal,
            namespace_index,
            key,
            ItemType::Sized,
            None,
            &buf,
        ) {
            Ok(_) => {}
            Err(Error::PageFull) => {
                page.mark_as_full::<T>(&mut self.hal)?;
                self.pages.push(page);

                page = self.get_active_page()?;
                page.write_variable_sized_item::<T>(
                    &mut self.hal,
                    namespace_index,
                    key,
                    ItemType::Sized,
                    None,
                    &buf,
                )?;
            }
            Err(e) => return Err(e),
        }

        self.pages.push(page);

        // Now delete the old entry if it exists
        if let Some((_page_index, _item_index)) = old_entry_location {
            self.delete_key(namespace_index, &key, ChunkIndex::Any)?;
        }

        Ok(())
    }

    pub(crate) fn set_blob(&mut self, namespace: &Key, key: Key, data: &[u8]) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("set_blob");

        #[cfg(feature = "debug-logs")]
        println!("internal: set_blob");

        if key.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::KeyMalformed);
        }
        if namespace.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::NamespaceMalformed);
        }

        if data.len() + 1 > MAX_BLOB_SIZE {
            return Err(Error::ValueTooLong);
        }

        // Check if we're overwriting an existing blob to determine version offset
        let old_blob_version = self.find_existing_blob_version(namespace, &key);

        // Check if the value already exists and matches (only if namespace exists)
        let should_write = if let Some(&namespace_index) = self.namespaces.get(namespace) {
            match self.load_item(namespace_index, ChunkIndex::Any, &key) {
                Ok((_page_index, _item_index, item)) => {
                    if item.type_ != ItemType::BlobIndex {
                        true // Type differs, need to write
                    } else {
                        !self.blob_is_equal(namespace_index, &key, &item, data)?
                    }
                }
                Err(_) => true, // Key doesn't exist, need to write
            }
        } else {
            true // Namespace doesn't exist, need to write
        };

        if !should_write {
            return Ok(());
        }

        // Get namespace index
        let mut page = self.get_active_page()?;
        let namespace_index = self.get_or_create_namespace(namespace, &mut page)?;
        self.pages.push(page);

        // Determine the version offset for the new blob
        let new_version_offset = match &old_blob_version {
            Some(old_offset) => old_offset.invert(),
            None => VersionOffset::V0,
        };

        let version_base = new_version_offset.clone() as u8;
        let mut chunk_count = 0u8;
        let mut offset = 0usize;

        while offset < data.len() {
            let mut page = self.get_active_page()?;

            // Calculate how much data we can fit
            let free_entries = page.get_free_entry_count();
            if free_entries <= 1 {
                page.mark_as_full::<T>(&mut self.hal)?;
                self.pages.push(page);
                continue;
            }
            let data_len = cmp::min((free_entries - 1) * size_of::<Item>(), data.len() - offset);

            match page.write_variable_sized_item::<T>(
                &mut self.hal,
                namespace_index,
                key,
                ItemType::BlobData,
                Some(version_base + chunk_count),
                &data[offset..offset + data_len],
            ) {
                Ok(_) => {
                    offset += data_len;
                    chunk_count += 1;
                    self.pages.push(page);
                }
                Err(Error::PageFull) => {
                    page.mark_as_full::<T>(&mut self.hal)?;
                    self.pages.push(page);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        // Write the blob index
        let mut page = self.get_active_page()?;
        let item_data = raw::ItemData {
            blob_index: ItemDataBlobIndex {
                size: data.len() as u32,
                chunk_count,
                chunk_start: version_base,
            },
        };
        page.write_item::<T>(
            &mut self.hal,
            namespace_index,
            key,
            ItemType::BlobIndex,
            None,
            1,
            item_data,
        )?;
        self.pages.push(page);

        // Now that the new blob version has been successfully written, delete the old version if it exists
        // _old_version is unused since it will be the first one that is bound to be found anyway as newer
        // pages appear later in self.pages
        if let Some(_old_version) = old_blob_version {
            self.delete_key(namespace_index, &key, ChunkIndex::BlobIndex)?;
        }

        Ok(())
    }

    pub(crate) fn get_active_page(&mut self) -> Result<ThinPage, Error> {
        #[cfg(feature = "defmt")]
        trace!("get_active_page");

        let page = self
            .pages
            .pop_if(|page| page.header.state == ThinPageState::Active);
        if let Some(page) = page {
            return Ok(page);
        }

        // Only try reclamation if we have no free pages left
        if self.free_pages.len() == 1 {
            self.defragment()?;
        }

        let page = self
            .pages
            .pop_if(|page| page.header.state == ThinPageState::Active);
        if let Some(page) = page {
            return Ok(page);
        }

        // After reclamation, check if we have free pages available
        if self.free_pages.len() == 1 {
            return Err(Error::FlashFull);
        }

        // at this point we have at least 2 free pages
        let mut page = self.free_pages.pop().unwrap();

        if page.header.state != ThinPageState::Uninitialized {
            self.hal
                .erase(
                    page.address as _,
                    (page.address + raw::FLASH_SECTOR_SIZE) as _,
                )
                .map_err(|_| Error::FlashError)?;
        }

        let next_sequence = self.get_next_sequence();
        page.initialize(&mut self.hal, next_sequence)?;

        Ok(page)
    }

    fn get_next_sequence(&self) -> u32 {
        match self.pages.iter().map(|page| page.header.sequence).max() {
            Some(current) => current + 1,
            None => 0,
        }
    }

    fn get_or_create_namespace(
        &mut self,
        namespace: &Key,
        page: &mut ThinPage,
    ) -> Result<u8, Error> {
        #[cfg(feature = "defmt")]
        trace!("get_or_create_namespace");

        #[cfg(feature = "debug-logs")]
        println!("internal: get_or_create_namespace");

        let namespace_index = match self.namespaces.get(namespace) {
            Some(ns_idx) => *ns_idx,
            None => {
                let namespace_index = self
                    .namespaces
                    .iter()
                    .max_by_key(|(_, idx)| **idx)
                    .map_or(1, |(_, idx)| idx + 1);

                page.write_namespace(&mut self.hal, *namespace, namespace_index)?;

                self.namespaces.insert(*namespace, namespace_index);

                namespace_index
            }
        };

        Ok(namespace_index)
    }

    pub(crate) fn load_item(
        &mut self,
        namespace_index: u8,
        chunk_index: ChunkIndex,
        key: &Key,
    ) -> Result<(PageIndex, ItemIndex, Item), Error> {
        #[cfg(feature = "defmt")]
        trace!("load_item");

        #[cfg(feature = "debug-logs")]
        println!("internal: load_item {chunk_index:?}");

        let item_chunk_index = match chunk_index {
            ChunkIndex::Any => 0xFF,
            ChunkIndex::BlobIndex => 0xFF,
            ChunkIndex::BlobData(idx) => idx,
        };

        let hash = Item::calculate_hash_ref(T::crc32, namespace_index, key, item_chunk_index);

        #[cfg(feature = "debug-logs")]
        println!("looking for hash {hash:?}");

        for (page_index, page) in self.pages.iter().enumerate() {
            for cache_entry in &page.item_hash_list {
                if cache_entry.hash == hash {
                    let item: Item = page.load_item(&mut self.hal, cache_entry.index)?;

                    if item.namespace_index != namespace_index
                        || item.key != *key
                        || item.chunk_index != item_chunk_index
                    {
                        continue;
                    }

                    return Ok((page_index.into(), cache_entry.index.into(), item));
                }
            }
        }

        Err(KeyNotFound)
    }

    pub(crate) fn load_sectors(&mut self) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("load_sectors");

        #[cfg(feature = "debug-logs")]
        println!("internal: load_sectors");

        let mut blob_index = BlobIndex::new();
        let sectors = self.sectors as usize;
        for sector_idx in 0..sectors {
            let sector_addr = self.base_address + sector_idx * FLASH_SECTOR_SIZE;
            match self.load_sector(sector_addr)? {
                LoadPageResult::Empty(page) => self.free_pages.push(page),
                LoadPageResult::Used(page, new_namespaces, new_blob_index) => {
                    self.pages.push(page);
                    new_namespaces.into_iter().for_each(|ns| {
                        self.namespaces.insert(ns.name, ns.index);
                    });
                    new_blob_index.into_iter().for_each(|(key, val)| {
                        match blob_index.get_mut(&key) {
                            Some(existing) => {
                                if let Some(index) = val.0 {
                                    existing.0 = Some(index);
                                }
                                // Merge chunks from this page into the existing data
                                existing.1.chunks_by_page.extend(val.1.chunks_by_page);
                            }
                            None => {
                                blob_index.insert(key, val);
                            }
                        }
                    })
                }
            };
        }

        #[cfg(feature = "debug-logs")]
        println!("internal: load_sectors: blob_index: {:?}", blob_index);

        self.ensure_active_page_order()?;

        self.continue_free_page()?;

        // After loading all pages, check for duplicate primitive/string entries and mark older ones as erased
        // This handles cases where deletion failed after a successful write
        self.cleanup_duplicate_entries()?;

        self.cleanup_dirty_blobs(blob_index)?;

        Ok(())
    }

    fn cleanup_dirty_blobs(&mut self, mut blob_index: BlobIndex) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("cleanup_dirty_blobs");

        while let Some(((namespace_index, chunk_start, key), (index, observed))) =
            blob_index.pop_first()
        {
            if let Some(index) = index {
                // Calculate total chunks and data size from all observed chunks
                let (chunk_count, data_size) = observed.chunks_by_page.iter().fold(
                    (0u8, 0u32),
                    |(count, size), chunk_data| {
                        (count + chunk_data.chunk_count, size + chunk_data.data_size)
                    },
                );

                if index.chunk_count != chunk_count || index.size != data_size {
                    #[cfg(feature = "debug-logs")]
                    println!(
                        "internal: load_sectors: blob index data doesn't match observed data {index:?} (expected: chunk_count={}, data_size={}, got: chunk_count={}, data_size={})",
                        index.chunk_count, index.size, chunk_count, data_size
                    );
                    self.delete_key(namespace_index.0, &key, ChunkIndex::BlobIndex)?;
                    // Also delete the orphaned data chunks for this version
                    self.delete_blob_data(namespace_index.0, &key, chunk_start)?;
                    continue;
                } else if let Some(other) =
                    blob_index.get(&(namespace_index, chunk_start.invert(), key))
                    && let Some(other_index) = &other.0
                {
                    // We have both versions - keep the newer one, delete the older one
                    // Compare by page_sequence first, then by item_index if on same page
                    let other_is_newer = other_index.page_sequence > index.page_sequence
                        || (index.page_sequence == other_index.page_sequence
                            && other_index.item_index > index.item_index);

                    if other_is_newer {
                        #[cfg(feature = "debug-logs")]
                        println!(
                            "internal: load_sectors: found two blob indices for the same key, deleting the older current one (seq: {} vs {})",
                            index.page_sequence, other_index.page_sequence
                        );
                        self.delete_key(namespace_index.0, &key, ChunkIndex::BlobIndex)?;
                    } else {
                        #[cfg(feature = "debug-logs")]
                        println!(
                            "internal: load_sectors: found two blob indices for the same key, deleting the older other one (seq: {} vs {})",
                            other_index.page_sequence, index.page_sequence
                        );
                        self.delete_key(namespace_index.0, &key, ChunkIndex::BlobIndex)?;
                    }
                }
            } else {
                // Orphaned blob data (chunks without an index) can occur when:
                // 1. Writing the blob index failed after data chunks were written
                // 2. The index was deleted but data deletion failed
                #[cfg(feature = "debug-logs")]
                println!(
                    "internal: load_sectors: found orphaned blob data. key: '{}', chunk_start: {}",
                    slice_with_nullbytes_to_str(&key.0),
                    chunk_start.clone() as u8
                );
                self.delete_blob_data(namespace_index.0, &key, chunk_start)?;
            }
        }
        Ok(())
    }

    /// The active page has to be the last page in `self.pages` as we use `pop_if` to fetch it.
    /// We also clean up any duplicate active pages that might have been created in the past
    /// due to the borked order.
    fn ensure_active_page_order(&mut self) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("ensure_active_page_order");

        let correct_active_page_stats =
            self.pages
                .iter()
                .enumerate()
                .fold(None, |acc, (idx, page)| {
                    if page.header.state != ThinPageState::Active {
                        return acc;
                    }

                    match acc {
                        None => Some((idx, page.header.sequence, 1)),
                        Some((acc_idx, acc_sequence, acc_active_page_count)) => {
                            if page.header.sequence > acc_sequence {
                                Some((idx, page.header.sequence, acc_active_page_count + 1))
                            } else {
                                Some((acc_idx, acc_sequence, acc_active_page_count + 1))
                            }
                        }
                    }
                });

        if let Some((correct_active_page_idx, _, active_page_count)) = correct_active_page_stats {
            let last_page_idx = self.pages.len() - 1;
            if correct_active_page_idx != last_page_idx {
                self.pages.swap(correct_active_page_idx, last_page_idx);
            }

            // Mark duplicate active pages as Full
            if active_page_count > 1 {
                // We actively ignore the last page as it is the correct active one
                for idx in 0..last_page_idx {
                    let page = &mut self.pages[idx];
                    if page.header.state == ThinPageState::Active {
                        #[cfg(feature = "defmt")]
                        warn!(
                            "detected duplicate active page, marking as full ({:#08x})",
                            page.address
                        );
                        page.mark_as_full(&mut self.hal)?;
                    }
                }
            }
        }

        Ok(())
    }

    fn continue_free_page(&mut self) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("continue_free_page");

        let source_page = match self
            .pages
            .iter()
            .position(|it| it.header.state == ThinPageState::Freeing)
        {
            None => return Ok(()),
            Some(idx) => self.pages.swap_remove(idx),
        };

        let target_page = match self
            .pages
            .iter()
            .position(|it| it.header.state == ThinPageState::Active)
        {
            Some(idx) => self.pages.swap_remove(idx),
            None => {
                let mut page = self.free_pages.pop().ok_or(Error::FlashFull)?;
                if page.header.state != ThinPageState::Uninitialized {
                    self.erase_page(page)?;
                    self.free_pages.pop().unwrap() // there is always a page after erasing
                } else {
                    let next_sequence = self.get_next_sequence();
                    page.initialize(&mut self.hal, next_sequence)?;
                    page
                }
            }
        };

        self.copy_items(&source_page, target_page)?;

        self.erase_page(source_page)?;

        Ok(())
    }

    /// Clean up duplicate primitive/string entries by marking older versions as erased.
    /// This handles the write-before-delete scenario where deletion failed after successful write.
    /// IMPORTANT: This does NOT touch blob entries - they have their own cleanup logic.
    fn cleanup_duplicate_entries(&mut self) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("cleanup_duplicate_entries");

        #[cfg(feature = "debug-logs")]
        println!("internal: cleanup_duplicate_entries");

        // Build a map of hash (as u32) -> Vec<(page_index, item_index, page_sequence)>
        // Use the hash as a quick filter - duplicates will have the same hash
        let mut hash_to_item: BTreeMap<u24, Vec<(PageIndex, ItemIndex, PageSequence)>> =
            BTreeMap::new();

        for (page_idx, page) in self.pages.iter().enumerate() {
            for hash_entry in &page.item_hash_list {
                hash_to_item.entry(hash_entry.hash).or_default().push((
                    PageIndex(page_idx),
                    ItemIndex(hash_entry.index),
                    PageSequence(page.header.sequence),
                ));
            }
        }

        for (_hash, entries) in hash_to_item {
            if entries.len() <= 1 {
                continue; // No duplicates for this hash
            }

            // Now we need to load items to check their full identity and type
            let mut items: Vec<_> = Vec::with_capacity(entries.len());
            for (page_idx, item_index, page_seq) in entries {
                let page = &self.pages[page_idx.0];
                let item = page.load_item(&mut self.hal, item_index.0)?;

                // Skip namespace entries (namespace_index == 0) and blob entries
                // Namespace entries are special and should not be cleaned up
                // Blob entries have their own cleanup logic
                if item.namespace_index == 0
                    || item.type_ == ItemType::BlobIndex
                    || item.type_ == ItemType::BlobData
                {
                    continue;
                }

                items.push((
                    (NamespaceIndex(item.namespace_index), item.key),
                    (page_idx, item_index, page_seq, item.span),
                ));
            }

            // Group by (namespace_index, key) to find actual duplicates
            let mut key_groups = BTreeMap::<_, Vec<_>>::new();
            for (key, val) in items {
                key_groups.entry(key).or_default().push(val);
            }

            // Erase older duplicates
            for (_key, mut group) in key_groups {
                if group.len() <= 1 {
                    continue;
                }

                // Sort by page sequence and item index (ascending = oldest first)
                group.sort_by_key(|(_, ItemIndex(idx), PageSequence(seq), _)| (*seq, *idx));

                // Keep the newest (last after sort), erase older ones
                let keep_count = group.len() - 1;
                for (PageIndex(page_index), ItemIndex(item_index), _, span) in
                    group.into_iter().take(keep_count)
                {
                    let page = self.pages.get_mut(page_index).unwrap();
                    page.erase_item::<T>(&mut self.hal, item_index, span)?;
                }
            }
        }

        Ok(())
    }

    /// Try to find and reclaim pages that can be recycled
    fn defragment(&mut self) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("defragment");

        #[cfg(feature = "debug-logs")]
        println!("internal: defragment");

        let next_sequence = self.get_next_sequence();

        // Find the next page to reclaim
        // By incorporating the sequence number, we will also reclaim older pages even if they are
        // pretty full. This helps with more even wear leveling.
        let next_page = self
            .pages
            .iter()
            .enumerate()
            .map(|(idx, page)| {
                let points = if page.erased_entry_count == 0 {
                    0
                } else {
                    page.erased_entry_count as u32 * 10 + (next_sequence - page.header.sequence)
                };
                (points, idx)
            })
            .max_by_key(|(points, _idx)| *points)
            .map(|(_, idx)| idx)
            .ok_or(Error::FlashFull)?;

        let page = self.pages.swap_remove(next_page);

        #[cfg(feature = "debug-logs")]
        println!("internal: defragment: next_page: {page:?}");

        match page.header.state {
            ThinPageState::Uninitialized => unreachable!(),
            ThinPageState::Active => unreachable!(),
            ThinPageState::Full => {
                if page.erased_entry_count != ENTRIES_PER_PAGE as _ {
                    self.free_page(&page, next_sequence)?;
                }

                self.erase_page(page)?;
            }
            ThinPageState::Freeing => unreachable!(), // TODO cleanup freeing pages on init
            ThinPageState::Corrupt => {
                self.erase_page(page)?;
            }
            ThinPageState::Invalid => {
                self.erase_page(page)?;
            }
        }

        Ok(())
    }

    /// Quickly reclaim a page that has no valid entries
    fn erase_page(&mut self, page: ThinPage) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("erase_page");

        #[cfg(feature = "debug-logs")]
        println!("internal: erase_page");

        // Erase the page and add it to free_pages
        self.hal
            .erase(page.address as _, (page.address + FLASH_SECTOR_SIZE) as _)
            .map_err(|_| Error::FlashError)?;

        self.free_pages.push(ThinPage::uninitialized(page.address));

        Ok(())
    }

    fn free_page(&mut self, source: &ThinPage, next_sequence: u32) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("free_page");

        #[cfg(feature = "debug-logs")]
        println!("internal: copy_entries_to_reserve_page");

        // Mark source page as FREEING
        let raw = (PageState::Freeing as u32).to_le_bytes();
        write_aligned(&mut self.hal, source.address as u32, &raw).map_err(|_| Error::FlashError)?;

        // TODO: Check if the active page has still some space left, e.g. this might happen if we
        //  wanted to write a string that can't be split over multiple pages or a chunk of blob_data
        //  which requires at least 2 empty entries

        // When free_page is called, we should always we have on page in reserve.
        let mut target = self.free_pages.pop().ok_or(Error::FlashFull)?;
        if target.header.state != ThinPageState::Uninitialized {
            self.hal
                .erase(
                    target.address as _,
                    (target.address + FLASH_SECTOR_SIZE) as _,
                )
                .map_err(|_| Error::FlashError)?;
        }
        target.initialize(&mut self.hal, next_sequence)?;

        self.copy_items(source, target)?;

        #[cfg(feature = "debug-logs")]
        println!("internal: copy_entries_to_reserve_page done");

        Ok(())
    }

    fn copy_items(&mut self, source: &ThinPage, mut target: ThinPage) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("copy_items");

        // in case the operation was disturbed in the middle, target might already contain some parts
        // of the source page, so we first get the last copied item so we can ignor it and everything
        // before in our copy loop
        let mut last_copied_entry = match target.item_hash_list.iter().max_by_key(|it| it.index) {
            Some(hash_entry) => Some(target.load_item(&mut self.hal, hash_entry.index)?),
            None => None,
        };

        let mut item_index = 0u8;
        while item_index < ENTRIES_PER_PAGE as u8 {
            if source.get_entry_state(item_index) != EntryMapState::Written {
                item_index += 1;
                continue;
            }

            let item = source.load_item(&mut self.hal, item_index)?;

            // in case we were disrupted while copying, we want to ignore all entries that before we
            // reached the last copied one
            if let Some(last) = last_copied_entry {
                if item == last {
                    // We found our match, everything after this still needs to be copied
                    last_copied_entry = None;
                } else {
                    // No match yet, keep searching
                }

                item_index += item.span;
                continue;
            }

            match item.type_ {
                ItemType::U8
                | ItemType::I8
                | ItemType::U16
                | ItemType::I16
                | ItemType::U32
                | ItemType::I32
                | ItemType::U64
                | ItemType::I64
                | ItemType::BlobIndex => {
                    target.write_item::<T>(
                        &mut self.hal,
                        item.namespace_index,
                        item.key,
                        item.type_,
                        if item.chunk_index == u8::MAX {
                            None
                        } else {
                            Some(item.chunk_index)
                        },
                        item.span,
                        item.data,
                    )?;
                }
                ItemType::Sized | ItemType::BlobData => {
                    let data = source.load_referenced_data(&mut self.hal, item_index, &item)?;
                    target.write_variable_sized_item::<T>(
                        &mut self.hal,
                        item.namespace_index,
                        item.key,
                        item.type_,
                        if item.chunk_index == u8::MAX {
                            None
                        } else {
                            Some(item.chunk_index)
                        },
                        &data,
                    )?;
                }
                ItemType::Blob => {
                    // Old BLOB type - not supported, skip
                }
                ItemType::Any => {
                    // Should not happen
                }
            }

            item_index += item.span;
        }

        self.pages.push(target);
        Ok(())
    }

    fn load_sector(&mut self, sector_address: usize) -> Result<LoadPageResult, Error> {
        #[cfg(feature = "defmt")]
        trace!("load_sector: @{:#08x}", sector_address);

        #[cfg(feature = "debug-logs")]
        println!("  raw: load page: 0x{sector_address:04X}");

        let mut buf = [0u8; FLASH_SECTOR_SIZE];
        self.hal
            .read(sector_address as _, &mut buf)
            .map_err(|_| Error::FlashError)?;

        if buf[..size_of::<PageHeader>()] == [0xFFu8; size_of::<PageHeader>()] {
            #[cfg(feature = "debug-logs")]
            println!("  raw: load page: 0x{sector_address:04X} -> uninitialized");

            return Ok(LoadPageResult::Empty(ThinPage::uninitialized(
                sector_address,
            )));
        }

        // Safety: either we return directly CORRUPT/INVALID/EMPTY page or we check the crc afterwards
        let raw_page: RawPage = unsafe { core::mem::transmute(buf) };

        #[cfg(feature = "debug-logs")]
        {
            let state = PageState::from(raw_page.header.state);
            println!("  raw: load page: 0x{sector_address:04X} -> {state}");
        }

        let mut page = ThinPage {
            address: sector_address,
            header: raw_page.header.into(),
            entry_state_bitmap: raw_page.entry_state_bitmap,
            erased_entry_count: 0,
            used_entry_count: 0,
            item_hash_list: vec![],
        };

        match page.header.state {
            ThinPageState::Corrupt | ThinPageState::Invalid => {
                return Ok(LoadPageResult::Empty(page));
            }
            ThinPageState::Uninitialized => {
                // validate that the page is truly empty
                if buf.iter().all(|it| *it == 0xFF).not() {
                    page.header.state = ThinPageState::Corrupt;
                };

                return Ok(LoadPageResult::Empty(page));
            }
            ThinPageState::Freeing => (),
            ThinPageState::Active => (),
            ThinPageState::Full => (),
        }

        if raw_page.header.crc != raw_page.header.calculate_crc32(T::crc32) {
            page.header.state = ThinPageState::Corrupt;
            return Ok(LoadPageResult::Empty(page));
        };

        let mut blob_index = BlobIndex::new();

        // Needed due to the desugaring below
        let mut namespaces: Vec<Namespace> = vec![];
        // This iterator desugaring is necessary to be able to skip entries, e.g. a BLOB or STR entries
        // are followed by entries containing their raw value.
        let items = &raw_page.items;
        let mut item_iter = unsafe { items.entries.iter().zip(u8::MIN..u8::MAX) };
        'item_iter: while let Some((item, item_index)) = item_iter.next() {
            let state = page.get_entry_state(item_index);
            match state {
                EntryMapState::Illegal => {
                    page.erased_entry_count += 1;
                    continue 'item_iter;
                }
                EntryMapState::Erased => {
                    page.erased_entry_count += 1;
                    continue 'item_iter;
                }
                EntryMapState::Empty => {
                    // maybe data was written but the map was not updated yet
                    let calculated_crc = item.calculate_crc32(T::crc32);
                    if item.crc == calculated_crc
                        && item.type_ != ItemType::Any
                        && item.span != u8::MAX
                    {
                        match item.type_ {
                            ItemType::U8
                            | ItemType::I8
                            | ItemType::U16
                            | ItemType::I16
                            | ItemType::U32
                            | ItemType::I32
                            | ItemType::U64
                            | ItemType::I64
                            | ItemType::BlobIndex => {
                                #[cfg(feature = "debug-logs")]
                                println!("encountered valid but empty scalar item at {item_index}");
                                page.set_entry_state(
                                    &mut self.hal,
                                    item_index as _,
                                    EntryMapState::Written,
                                )?;
                                page.used_entry_count += 1;
                            }
                            ItemType::Blob => {
                                // TODO: should we just ignore this value or mark page corrupt?
                                //  Alternatively, we could add support for BLOB_V1 and convert it here
                                page.used_entry_count += 1;
                                continue 'item_iter;
                            }
                            ItemType::Sized | ItemType::BlobData => {
                                #[cfg(feature = "debug-logs")]
                                println!(
                                    "encountered valid but EMPTY variable sized item at {item_index}"
                                );
                                let data =
                                    page.load_referenced_data(&mut self.hal, item_index, item)?;
                                let data_crc = T::crc32(u32::MAX, &data);
                                if data_crc != unsafe { item.data.sized.crc } {
                                    page.set_entry_state_range(
                                        &mut self.hal,
                                        item_index..item_index + item.span,
                                        EntryMapState::Erased,
                                    )?;
                                    page.erased_entry_count += item.span;
                                    continue 'item_iter;
                                }
                                page.set_entry_state_range(
                                    &mut self.hal,
                                    item_index..item_index + item.span,
                                    EntryMapState::Written,
                                )?;
                                page.used_entry_count += item.span;
                            }
                            ItemType::Any => {
                                continue 'item_iter;
                            }
                        }
                    } else {
                        continue 'item_iter;
                    }
                }
                EntryMapState::Written => {
                    let calculated_crc = item.calculate_crc32(T::crc32);
                    if item.crc != calculated_crc {
                        #[cfg(feature = "debug-logs")]
                        println!(
                            "CRC mismatch for item '{}', marking as erased",
                            slice_with_nullbytes_to_str(&item.key.0)
                        );
                        page.set_entry_state_range(
                            &mut self.hal,
                            item_index..(item_index + item.span),
                            EntryMapState::Erased,
                        )?;
                        page.erased_entry_count += item.span;
                        continue 'item_iter;
                    }
                    page.used_entry_count += item.span;
                }
            }

            // Continue for valid WRITTEN and formerly EMPTY entries
            #[cfg(feature = "debug-logs")]
            println!("item: {:?}", item);

            if item.namespace_index == 0 {
                namespaces.push(Namespace {
                    name: item.key,
                    index: unsafe { item.data.raw[0] },
                });
                continue 'item_iter;
            }

            if item.type_ == ItemType::BlobIndex || item.type_ == ItemType::BlobData {
                let chunk_start = if item.type_ == ItemType::BlobIndex {
                    unsafe { VersionOffset::from(item.data.blob_index.chunk_start) }
                } else {
                    VersionOffset::from(item.chunk_index)
                };

                let key = (NamespaceIndex(item.namespace_index), chunk_start, item.key);
                let existing = blob_index.get_mut(&key);
                if let Some(existing) = existing {
                    if item.type_ == ItemType::BlobIndex {
                        existing.0 = Some(BlobIndexEntryBlobIndexData {
                            item_index,
                            page_sequence: page.header.sequence,
                            size: unsafe { item.data.blob_index.size },
                            chunk_count: unsafe { item.data.blob_index.chunk_count },
                        });
                    } else {
                        // Add this chunk to the page-specific tracking
                        let chunk_size = unsafe { item.data.sized.size } as u32;
                        let page_seq = page.header.sequence;

                        // Check if we already have chunks from this page
                        if let Some(entry) = existing
                            .1
                            .chunks_by_page
                            .iter_mut()
                            .find(|chunk| chunk.page_sequence == page_seq)
                        {
                            entry.chunk_count += 1;
                            entry.data_size += chunk_size;
                        } else {
                            existing.1.chunks_by_page.push(ChunkData {
                                page_sequence: page_seq,
                                chunk_count: 1,
                                data_size: chunk_size,
                            });
                        }
                    }
                } else if item.type_ == ItemType::BlobIndex {
                    blob_index.insert(
                        key,
                        (
                            Some(BlobIndexEntryBlobIndexData {
                                item_index,
                                page_sequence: page.header.sequence,
                                size: unsafe { item.data.blob_index.size },
                                chunk_count: unsafe { item.data.blob_index.chunk_count },
                            }),
                            BlobObservedData {
                                chunks_by_page: vec![],
                            },
                        ),
                    );
                } else {
                    blob_index.insert(
                        key,
                        (
                            None,
                            BlobObservedData {
                                chunks_by_page: vec![ChunkData {
                                    page_sequence: page.header.sequence,
                                    chunk_count: 1,
                                    data_size: unsafe { item.data.sized.size } as u32,
                                }],
                            },
                        ),
                    );
                }
            }

            page.item_hash_list.push(ItemHashListEntry {
                hash: item.calculate_hash(T::crc32),
                index: item_index,
            });

            // skip following items containing raw data
            if item.span >= 2 {
                item_iter.nth((item.span - 2) as usize);
            }
        }

        #[cfg(feature = "debug-logs")]
        println!("PGE {page:?}");

        Ok(LoadPageResult::Used(page, namespaces, blob_index))
    }
}
