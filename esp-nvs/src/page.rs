//! Page management for NVS storage.
//!
//! This module contains the [`ThinPage`] type and related utilities for
//! managing individual flash pages within an NVS partition.

use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;
#[cfg(feature = "debug-logs")]
use core::fmt::{
    Debug,
    Formatter,
};
use core::mem::{
    offset_of,
    size_of,
};
use core::ops::Range;

#[cfg(feature = "defmt")]
use defmt::trace;

use crate::Key;
use crate::error::Error;
use crate::error::Error::{
    ItemTypeMismatch,
    KeyNotFound,
    PageFull,
};
use crate::platform::{
    AlignedOps,
    Platform,
};
use crate::raw::{
    ENTRIES_PER_PAGE,
    ENTRY_STATE_BITMAP_SIZE,
    EntryMapState,
    Item,
    ItemData,
    ItemType,
    PageHeader,
    PageHeaderRaw,
    PageState,
    RawItem,
    RawPage,
    write_aligned,
};
use crate::u24::u24;

/// In-memory representation of a flash page with minimal memory footprint.
pub(crate) struct ThinPage {
    pub(crate) address: usize,
    pub(crate) header: ThinPageHeader,
    pub(crate) entry_state_bitmap: [u8; ENTRY_STATE_BITMAP_SIZE],
    pub(crate) item_hash_list: Vec<ItemHashListEntry>,
    pub(crate) erased_entry_count: u8,
    pub(crate) used_entry_count: u8,
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

    pub(crate) fn initialize<T: Platform>(&mut self, hal: &mut T, next_sequence: u32) -> Result<(), Error> {
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

        write_aligned::<T>(hal, self.address as u32, unsafe { &raw_header.raw }).map_err(|_| Error::FlashError)?;

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

    pub(crate) fn load_item<T: Platform>(&self, hal: &mut T, item_index: u8) -> Result<Item, Error> {
        #[cfg(feature = "defmt")]
        trace!("load_item: @{:#08x}[{}]", self.address, item_index);

        let mut buf = [0u8; size_of::<Item>()];
        hal.read(
            (self.address + offset_of!(RawPage, items) + size_of::<Item>() * item_index as usize) as _,
            &mut buf,
        )
        .map_err(|_| Error::FlashError)?;

        if buf.iter().all(|&it| it == 0xFF) {
            return Err(KeyNotFound);
        }

        // Safety: we check the crc afterwards
        let item = unsafe { core::mem::transmute::<[u8; 32], Item>(buf) };

        if item.crc != item.calculate_crc32(T::crc32) {
            return Err(KeyNotFound);
        }

        Ok(item)
    }

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
        let target_addr = self.address + offset_of!(RawPage, items) + size_of::<Item>() * item_index;

        #[cfg(feature = "defmt")]
        trace!("load_item: @{:#08x}[{}]", self.address, item_index);

        #[cfg(feature = "debug-logs")]
        println!("  internal: write_item: target_addr: 0x{target_addr:0>8x}");

        let raw_item = RawItem { item };
        write_aligned(hal, target_addr as _, unsafe { &raw_item.raw }).map_err(|_| Error::FlashError)?;

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

    pub(crate) fn write_namespace<T: Platform>(&mut self, hal: &mut T, key: Key, value: u8) -> Result<(), Error> {
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
            sized: crate::raw::ItemDataSized::new(data.len() as _, T::crc32(u32::MAX, data)),
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
        let header_addr = self.address + offset_of!(RawPage, items) + size_of::<Item>() * start_index;
        let raw_item = RawItem { item };

        write_aligned(hal, header_addr as _, unsafe { &raw_item.raw }).map_err(|_| Error::FlashError)?;

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

    pub(crate) fn load_referenced_data<T: Platform>(
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
            ItemType::Sized | ItemType::BlobData | ItemType::Blob => {}
            _ => return Err(ItemTypeMismatch(item.type_)),
        }

        let size = unsafe { item.data.sized.size } as usize;
        let aligned_size = T::align_read(size);

        let mut buf = Vec::with_capacity(aligned_size);
        // Safety: we just allocated the buffer with the exact size we need and we will override it
        // the the call to hal.read()
        unsafe {
            Vec::set_len(&mut buf, aligned_size);
        }
        hal.read(
            (self.address + offset_of!(RawPage, items) + size_of::<Item>() * (item_index as usize + 1)) as _,
            &mut buf,
        )
        .map_err(|_| Error::FlashError)?;

        // Safety: we allocated aligned_size bytes which is always more than size
        unsafe {
            Vec::set_len(&mut buf, size);
        }

        Ok(buf)
    }

    pub(crate) fn set_entry_state<T: Platform>(
        &mut self,
        hal: &mut T,
        item_index: usize,
        state: EntryMapState,
    ) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("set_entry_state: @{:#08x}[{}]: {}", self.address, item_index, state);

        #[cfg(feature = "debug-logs")]
        println!("internal: set_entry_state");

        self.set_entry_state_range(hal, (item_index as u8)..(item_index as u8 + 1), state)
    }

    pub(crate) fn get_entry_state(&self, item_index: u8) -> EntryMapState {
        let idx = item_index / 4;
        let byte = self.entry_state_bitmap[idx as usize];
        let two_bits = (byte >> ((item_index % 4) * 2)) & 0b11;

        let state = EntryMapState::from_repr(two_bits).unwrap();

        #[cfg(feature = "defmt")]
        trace!("get_entry_state: @{:#08x}[{}]: {}", self.address, item_index, state);

        #[cfg(feature = "debug-logs")]
        println!(
            "internal: get_item_state @{:#08x}[{item_index}]: {state:?}",
            self.address,
        );

        state
    }

    pub(crate) fn set_entry_state_range<T: Platform>(
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

        let offset_in_raw_flash = self.address + offset_of!(RawPage, entry_state_bitmap) + start_byte;
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

    pub(crate) fn get_next_free_entry(&self) -> usize {
        self.used_entry_count as usize + self.erased_entry_count as usize
    }

    pub(crate) fn get_free_entry_count(&self) -> usize {
        ENTRIES_PER_PAGE - self.get_next_free_entry()
    }

    pub(crate) fn is_full(&self) -> bool {
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

    pub(crate) fn erase_item<T: Platform>(&mut self, hal: &mut T, item_index: u8, span: u8) -> Result<(), Error> {
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
        self.item_hash_list.retain(|entry| entry.index != item_index);

        Ok(())
    }

    /// Returns an iterator over all items in this page.
    ///
    /// # Errors
    ///
    /// This functions iterator may return a `FlashError` if there is
    /// an error reading from flash.
    pub(crate) fn items<'a, T: Platform>(&'a self, hal: &'a mut T) -> IterPageItems<'a, T> {
        IterPageItems {
            page: self,
            hal,
            iter: self.item_hash_list.iter(),
        }
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
            (ThinPageState::Uninitialized, ThinPageState::Uninitialized) => other.address.cmp(&self.address),
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
        f.write_fmt(format_args!("Page {{ address: 0x{address:0>8x} {header:?} "))?;
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

/// Iterator over items in a single page.
pub(crate) struct IterPageItems<'a, T: Platform> {
    page: &'a ThinPage,
    hal: &'a mut T,
    iter: core::slice::Iter<'a, ItemHashListEntry>,
}

impl<'a, T: Platform> IterPageItems<'a, T> {
    pub(crate) fn switch_to_page(&mut self, new_page: &'a ThinPage) {
        self.page = new_page;
        self.iter = self.page.item_hash_list.iter();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.iter.as_slice().is_empty()
    }
}

impl<'a, T: Platform> Iterator for IterPageItems<'a, T> {
    type Item = Result<Item, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.iter.next()?;

        Some(self.page.load_item(self.hal, entry.index))
    }
}

/// Compact page header stored in memory.
pub(crate) struct ThinPageHeader {
    pub(crate) state: ThinPageState,
    pub(crate) sequence: u32,
    pub(crate) version: u8,
    pub(crate) crc: u32,
}

impl ThinPageHeader {
    pub(crate) fn uninitialzed() -> Self {
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

/// State of a page in memory.
#[derive(strum::Display, PartialEq)]
pub(crate) enum ThinPageState {
    Uninitialized,
    Active,
    Full,
    Freeing,
    Corrupt,
    Invalid,
}

/// Entry in the item hash list for quick lookups.
pub(crate) struct ItemHashListEntry {
    pub(crate) hash: u24,
    pub(crate) index: u8,
}

/// Namespace definition loaded from flash.
pub(crate) struct Namespace {
    pub(crate) name: Key,
    pub(crate) index: u8,
}

/// Result of loading a page from flash.
pub(crate) enum LoadPageResult {
    Empty(ThinPage),
    Used(ThinPage, Vec<Namespace>, crate::blob::BlobIndex),
}
