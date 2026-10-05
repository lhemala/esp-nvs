//! NVS initialization and sector loading.
//!
//! This module contains the logic for reading flash sectors and initializing
//! the in-memory page structures during [`Nvs`](crate::Nvs) startup.

use alloc::vec;
use alloc::vec::Vec;
use core::mem::{
    offset_of,
    size_of,
};
use core::ops::Not;

#[cfg(feature = "defmt")]
use defmt::trace;

use crate::Nvs;
use crate::blob::{
    BlobIndex,
    BlobIndexEntryBlobIndexData,
    BlobObservedData,
    ChunkData,
};
use crate::error::Error;
use crate::page::{
    ItemHashListEntry,
    LoadPageResult,
    Namespace,
    ThinPage,
    ThinPageState,
};
use crate::platform::Platform;
#[cfg(feature = "debug-logs")]
use crate::raw::slice_with_nullbytes_to_str;
use crate::raw::{
    ENTRIES_PER_PAGE,
    EntryMapState,
    FLASH_SECTOR_SIZE,
    Item,
    ItemType,
    PageHeader,
    RawPage,
    sanitize_item_type,
};
use crate::types::{
    NamespaceIndex,
    VersionOffset,
};

impl<T> Nvs<T>
where
    T: Platform,
{
    pub(crate) fn load_sectors(&mut self) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("load_sectors");

        #[cfg(feature = "debug-logs")]
        println!("internal: load_sectors");

        let mut blob_index = self.scan_sectors()?;

        // Finishing an interrupted defragmentation erases pages the scan above has already
        // accounted for. The blob index in particular would count each chunk on both the source
        // and its partial copy, find twice the data its index claims, and delete a blob that is
        // perfectly intact. So start over from what is on flash once the recovery is done.
        // ESP-IDF never leaves more than one page `Freeing`, but nothing stops flash from saying
        // otherwise, so all of them are finished.
        let mut recovered = false;
        while self.continue_free_page()? {
            recovered = true;
        }
        if recovered {
            self.pages.clear();
            self.free_pages.clear();
            self.namespaces.clear();
            blob_index = self.scan_sectors()?;
        }

        // Settle the blobs first, so at most one version of each is left, then check for duplicate
        // entries and mark older ones as erased. This handles cases where deletion failed after a
        // successful write.
        self.cleanup_dirty_blobs(blob_index)?;

        self.cleanup_duplicate_entries()?;

        Ok(())
    }

    /// Loads every sector into `self.pages` and `self.free_pages`, and the namespaces into
    /// `self.namespaces`, returning what was found about blobs.
    fn scan_sectors(&mut self) -> Result<BlobIndex, Error> {
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

        Ok(blob_index)
    }

    pub(crate) fn load_sector(&mut self, sector_address: usize) -> Result<LoadPageResult, Error> {
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

            // A blank header does not make a blank page: an erase the power cut short, or a header
            // write that never happened, leaves data behind it. Handed out as uninitialized, the
            // page would be initialized without an erase and new items programmed over the old
            // bytes. Corrupt pages are erased before they are used.
            let mut page = ThinPage::uninitialized(sector_address);
            if buf.iter().all(|it| *it == 0xFF).not() {
                page.header.state = ThinPageState::Corrupt;
            }
            return Ok(LoadPageResult::Empty(page));
        }

        // The items are reinterpreted from raw flash, so their type bytes have to be valid first,
        // see `Item::from_raw`. `buf` itself keeps the bytes as they are on flash.
        let mut sanitized = buf;
        sanitized[offset_of!(RawPage, items)..]
            .as_chunks_mut::<{ size_of::<Item>() }>()
            .0
            .iter_mut()
            .for_each(sanitize_item_type);

        // Safety: either we return directly CORRUPT/INVALID/EMPTY page or we check the crc
        // afterwards
        let raw_page: RawPage = unsafe { core::mem::transmute(sanitized) };

        #[cfg(feature = "debug-logs")]
        {
            let state = crate::raw::PageState::from(raw_page.header.state);
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
        // This iterator desugaring is necessary to be able to skip entries, e.g. a BLOB or STR
        // entries are followed by entries containing their raw value.
        let items = &raw_page.items;
        let mut item_iter = unsafe { items.entries.iter().zip(u8::MIN..u8::MAX) };
        'item_iter: while let Some((item, item_index)) = item_iter.next() {
            let state = page.get_entry_state(item_index);

            // `span` is an unvalidated u8 straight from flash, and everything below uses it to walk
            // entries, mark ranges of the entry map and add to the page's u8 entry counters. All of
            // that assumes the span's entries are on this page. A span of zero, or one reaching
            // past the last entry, is corrupt and has to be recognised before it is
            // trusted: summing it into `used_entry_count` overflowed, which is a panic
            // in debug and a wrong count in release, and it happens during `Nvs::new`,
            // so a single bad byte took the partition down at startup before anything
            // could be read.
            let span_fits_page = item.span >= 1 && item_index as usize + item.span as usize <= ENTRIES_PER_PAGE;

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
                    let entry_offset = offset_of!(RawPage, items) + item_index as usize * size_of::<Item>();
                    if buf[entry_offset..entry_offset + size_of::<Item>()]
                        .iter()
                        .all(|it| *it == 0xFF)
                    {
                        continue 'item_iter;
                    }

                    // Not blank, so something was written here but the map was not updated yet.
                    // Either it is a complete item and recovered below, or it
                    // is what a torn write left behind. That has to be marked
                    // erased and counted like one: left as it is, it
                    // is where the next item would be written, programmed on top of the leftovers,
                    // and both the new value and the old one it replaces would be lost.
                    let calculated_crc = item.calculate_crc32(T::crc32);
                    if item.crc != calculated_crc || !span_fits_page {
                        page.set_entry_state(&mut self.hal, item_index as _, EntryMapState::Erased)?;
                        page.erased_entry_count += 1;
                        continue 'item_iter;
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
                        | ItemType::BlobIndex
                            if item.span == 1 =>
                        {
                            #[cfg(feature = "debug-logs")]
                            println!("encountered valid but empty scalar item at {item_index}");
                            page.set_entry_state(&mut self.hal, item_index as _, EntryMapState::Written)?;
                            page.used_entry_count += 1;
                        }
                        ItemType::Sized | ItemType::BlobData | ItemType::Blob => {
                            #[cfg(feature = "debug-logs")]
                            println!("encountered valid but EMPTY variable sized item at {item_index}");
                            let data_is_valid = match page.load_referenced_data(&mut self.hal, item_index, item) {
                                Ok(data) => T::crc32(u32::MAX, &data) == unsafe { item.data.sized.crc },
                                Err(Error::CorruptedData) => false,
                                Err(e) => return Err(e),
                            };
                            if !data_is_valid {
                                page.set_entry_state_range(
                                    &mut self.hal,
                                    item_index..item_index + item.span,
                                    EntryMapState::Erased,
                                )?;
                                page.erased_entry_count += item.span;
                                // The whole span is counted above, so its payload entries must
                                // not be visited again: they now read as erased and would each
                                // be counted a second time, pushing the next free entry past
                                // the end of the page.
                                if item.span >= 2 {
                                    item_iter.nth((item.span - 2) as usize);
                                }
                                continue 'item_iter;
                            }
                            page.set_entry_state_range(
                                &mut self.hal,
                                item_index..item_index + item.span,
                                EntryMapState::Written,
                            )?;
                            page.used_entry_count += item.span;
                        }
                        _ => {
                            page.set_entry_state(&mut self.hal, item_index as _, EntryMapState::Erased)?;
                            page.erased_entry_count += 1;
                            continue 'item_iter;
                        }
                    }
                }
                EntryMapState::Written => {
                    let calculated_crc = item.calculate_crc32(T::crc32);
                    if item.crc != calculated_crc || !span_fits_page {
                        #[cfg(feature = "debug-logs")]
                        println!(
                            "CRC mismatch for item '{}', marking as erased",
                            slice_with_nullbytes_to_str(&item.key.0)
                        );
                        // The span was read from a header whose CRC just failed, or one that cannot
                        // fit the page, so it says nothing trustworthy about how many entries this
                        // item covers. Erasing a range on its word takes out whatever happens to
                        // follow, valid items included. Erase the header alone: the entries behind
                        // it are scanned like any other and reach this same
                        // check one at a time, which ends in the same place
                        // for a genuinely half written item without reaching
                        // past it.
                        //
                        // The entry has to be marked erased in the entry map, not only counted as
                        // such. Those entries behind it are payload, and their "span" byte is as
                        // often as not out of range, so this branch is
                        // where most of them end up. Left `Written`,
                        // `copy_items` later loads each one, fails its CRC and aborts the
                        // defragmentation with the source page stuck in `Freeing`; every `Nvs::new`
                        // after that resumes the copy and fails the same way.
                        page.set_entry_state_range(&mut self.hal, item_index..(item_index + 1), EntryMapState::Erased)?;
                        page.erased_entry_count += 1;
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
                            BlobObservedData { chunks_by_page: vec![] },
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
