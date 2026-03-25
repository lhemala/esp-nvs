//! NVS initialization and sector loading.
//!
//! This module contains the logic for reading flash sectors and initializing
//! the in-memory page structures during [`Nvs`](crate::Nvs) startup.

use alloc::vec;
use alloc::vec::Vec;
use core::mem::size_of;
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
    EntryMapState,
    FLASH_SECTOR_SIZE,
    ItemType,
    PageHeader,
    RawPage,
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

        // After loading all pages, check for duplicate primitive/string entries and mark older ones
        // as erased This handles cases where deletion failed after a successful write
        self.cleanup_duplicate_entries()?;

        self.cleanup_dirty_blobs(blob_index)?;

        Ok(())
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

            return Ok(LoadPageResult::Empty(ThinPage::uninitialized(sector_address)));
        }

        // Safety: either we return directly CORRUPT/INVALID/EMPTY page or we check the crc
        // afterwards
        let raw_page: RawPage = unsafe { core::mem::transmute(buf) };

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
                    if item.crc == calculated_crc && item.type_ != ItemType::Any && item.span != u8::MAX {
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
                                page.set_entry_state(&mut self.hal, item_index as _, EntryMapState::Written)?;
                                page.used_entry_count += 1;
                            }
                            ItemType::Blob => {
                                // TODO: should we just ignore this value or mark page corrupt?
                                //  Alternatively, we could add support for BLOB_V1 and convert it
                                // here
                                page.used_entry_count += 1;
                                continue 'item_iter;
                            }
                            ItemType::Sized | ItemType::BlobData => {
                                #[cfg(feature = "debug-logs")]
                                println!("encountered valid but EMPTY variable sized item at {item_index}");
                                let data = page.load_referenced_data(&mut self.hal, item_index, item)?;
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
