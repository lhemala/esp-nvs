//! Page compaction and garbage collection for NVS.
//!
//! This module contains defragmentation, cleanup, and page reclamation logic.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

#[cfg(feature = "defmt")]
use defmt::{
    trace,
    warn,
};

use crate::Nvs;
use crate::blob::BlobIndex;
use crate::error::Error;
use crate::page::{
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
    ItemType,
    PageState,
    write_aligned,
};
use crate::types::{
    ChunkIndex,
    ItemIndex,
    NamespaceIndex,
    PageIndex,
    PageSequence,
};
use crate::u24::u24;

impl<T> Nvs<T>
where
    T: Platform,
{
    pub(crate) fn cleanup_dirty_blobs(&mut self, mut blob_index: BlobIndex) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("cleanup_dirty_blobs");

        while let Some(((namespace_index, chunk_start, key), (index, observed))) = blob_index.pop_first() {
            if let Some(index) = index {
                // Calculate total chunks and data size from all observed chunks
                let (chunk_count, data_size) = observed
                    .chunks_by_page
                    .iter()
                    .fold((0u8, 0u32), |(count, size), chunk_data| {
                        (count + chunk_data.chunk_count, size + chunk_data.data_size)
                    });

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
                } else if let Some(other) = blob_index.get(&(namespace_index, chunk_start.invert(), key))
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
    pub(crate) fn ensure_active_page_order(&mut self) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("ensure_active_page_order");

        let correct_active_page_stats = self.pages.iter().enumerate().fold(None, |acc, (idx, page)| {
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

            // Power lost between the write that filled the active page and the one marking it full
            // leaves it `Active` with no free entry. Retire it now, so the next write takes a fresh
            // page instead of finding no room on this one.
            let active_page = &mut self.pages[last_page_idx];
            if active_page.is_full() {
                active_page.mark_as_full(&mut self.hal)?;
            }
        }

        Ok(())
    }

    /// Finishes a defragmentation that was interrupted, returning whether there was one.
    ///
    /// A defragmentation only starts once no page is `Active`: it marks the source `Freeing`,
    /// copies its live items into a freshly initialized reserve page, which is `Active` from
    /// then on, and erases the source. So a `Freeing` page found here has to be finished, and
    /// an `Active` page next to it holds nothing but a partial copy of it.
    ///
    /// That partial copy is thrown away and the copy restarted into a clean page, the way ESP-IDF
    /// does it. Resuming it in place instead cannot be relied on: a write torn by the power loss
    /// leaves erased entries in the target, and with those the rest of the source may no longer
    /// fit, failing the same way on every boot.
    ///
    /// The one target worth keeping is a complete one. A copy that fills its target leaves it
    /// `Full` rather than `Active`, and when the reserve was the only free page there is none left
    /// to restart into, so that state would also fail on every boot.
    pub(crate) fn continue_free_page(&mut self) -> Result<bool, Error> {
        #[cfg(feature = "defmt")]
        trace!("continue_free_page");

        let source_page = match self
            .pages
            .iter()
            .position(|it| it.header.state == ThinPageState::Freeing)
        {
            None => return Ok(false),
            Some(idx) => self.pages.swap_remove(idx),
        };

        if let Some(idx) = self
            .pages
            .iter()
            .position(|it| it.header.state == ThinPageState::Active)
        {
            let partial_copy = self.pages.swap_remove(idx);
            self.erase_page(partial_copy)?;
        } else if self.newest_page_holds_all_items_of(&source_page)? {
            self.erase_page(source_page)?;
            return Ok(true);
        }

        let mut target = self.free_pages.pop().ok_or(Error::FlashFull)?;
        if target.header.state != ThinPageState::Uninitialized {
            self.hal
                .erase(target.address as _, (target.address + FLASH_SECTOR_SIZE) as _)
                .map_err(|_| Error::FlashError)?;
            target = ThinPage::uninitialized(target.address);
        }
        // The source is out of `self.pages` but its sequence counts all the same.
        let next_sequence = self.get_next_sequence().max(source_page.header.sequence + 1);
        target.initialize(&mut self.hal, next_sequence)?;

        self.copy_items(&source_page, target)?;

        self.erase_page(source_page)?;

        Ok(true)
    }

    /// Whether the newest page holds an item for every live item of `source`, meaning a copy of
    /// `source` into it completed.
    ///
    /// Should that page instead be an ordinary one that happens to hold newer values for each of
    /// those keys, the items on `source` are outdated duplicates, so dropping `source` is just as
    /// right.
    fn newest_page_holds_all_items_of(&mut self, source: &ThinPage) -> Result<bool, Error> {
        let Some(newest) = self.pages.iter().max_by_key(|page| page.header.sequence) else {
            return Ok(false);
        };
        if newest.header.sequence <= source.header.sequence {
            return Ok(false);
        }

        for source_entry in &source.item_hash_list {
            let Ok(item) = source.load_item(&mut self.hal, source_entry.index) else {
                continue;
            };

            let mut found = false;
            for entry in newest.item_hash_list.iter().filter(|it| it.hash == source_entry.hash) {
                if let Ok(candidate) = newest.load_item(&mut self.hal, entry.index)
                    && candidate.namespace_index == item.namespace_index
                    && candidate.key == item.key
                    && candidate.chunk_index == item.chunk_index
                {
                    found = true;
                    break;
                }
            }
            if !found {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Clean up duplicate primitive/string entries by marking older versions as erased.
    /// This handles the write-before-delete scenario where deletion failed after successful write.
    /// IMPORTANT: This does NOT touch blob entries - they have their own cleanup logic.
    pub(crate) fn cleanup_duplicate_entries(&mut self) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("cleanup_duplicate_entries");

        #[cfg(feature = "debug-logs")]
        println!("internal: cleanup_duplicate_entries");

        // Build a map of hash (as u32) -> Vec<(page_index, item_index, page_sequence)>
        // Use the hash as a quick filter - duplicates will have the same hash
        let mut hash_to_item: BTreeMap<u24, Vec<(PageIndex, ItemIndex, PageSequence)>> = BTreeMap::new();

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
                if item.namespace_index == 0 || item.type_ == ItemType::BlobIndex || item.type_ == ItemType::BlobData {
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
                for (PageIndex(page_index), ItemIndex(item_index), _, span) in group.into_iter().take(keep_count) {
                    let page = self.pages.get_mut(page_index).unwrap();
                    page.erase_item::<T>(&mut self.hal, item_index, span)?;
                }
            }
        }

        Ok(())
    }

    /// Try to find and reclaim pages that can be recycled
    /// Reclaims one page, chosen by erased entries weighted against age so that old pages are
    /// recycled too and the wear spreads.
    ///
    /// `Ok(())` does not mean the caller is better off than before, and no caller may assume it
    /// does. Reclaiming a page moves its live entries into the reserve page and pushes the erased
    /// source back, so the free page count is the same afterwards, and the copy has exactly as many
    /// free entries as the source had. Handed a page with a single erased entry, this reproduces an
    /// equally unusable page indefinitely, one sector erase per call.
    ///
    /// Two loops in `set_blob` were written on the assumption that a successful call means
    /// progress, and both spun forever, wearing a sector out in under a minute. A caller that
    /// retries after this has to carry its own proof of progress - `set_blob` counts retires
    /// against the chunks it has written. Making the progress observable here instead would
    /// suit callers better, but note that simply refusing to reclaim a page with no erased
    /// entries is not the answer: that copy is the only way the unused tail of a prematurely
    /// retired page ever becomes reachable again, and removing it costs writes that currently
    /// succeed.
    pub(crate) fn defragment(&mut self) -> Result<(), Error> {
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
                if page.erased_entry_count != ENTRIES_PER_PAGE as _
                    && let Err(e) = self.free_page(&page, next_sequence)
                {
                    // The source still holds its items, so it stays in the instance. At the front,
                    // where the oldest pages are, and away from the tail an active page belongs at.
                    self.pages.insert(0, page);
                    return Err(e);
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
    pub(crate) fn erase_page(&mut self, page: ThinPage) -> Result<(), Error> {
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

    pub(crate) fn free_page(&mut self, source: &ThinPage, next_sequence: u32) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("free_page");

        #[cfg(feature = "debug-logs")]
        println!("internal: copy_entries_to_reserve_page");

        // Mark source page as FREEING before the reserve page is touched. The other way round, a
        // power loss in between would leave the initialized reserve as an ordinary active page and
        // the partition without a reserve. `continue_free_page` picks up from either point.
        let raw = (PageState::Freeing as u32).to_le_bytes();
        write_aligned(&mut self.hal, source.address as u32, &raw).map_err(|_| Error::FlashError)?;

        // When free_page is called, we should always we have on page in reserve.
        let mut target = self.free_pages.pop().ok_or(Error::FlashFull)?;
        if target.header.state != ThinPageState::Uninitialized {
            self.hal
                .erase(target.address as _, (target.address + FLASH_SECTOR_SIZE) as _)
                .map_err(|_| Error::FlashError)?;
        }
        target.initialize(&mut self.hal, next_sequence)?;

        self.copy_items(source, target)?;

        #[cfg(feature = "debug-logs")]
        println!("internal: copy_entries_to_reserve_page done");

        Ok(())
    }

    pub(crate) fn copy_items(&mut self, source: &ThinPage, mut target: ThinPage) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("copy_items");

        let mut item_index = 0u8;
        while item_index < ENTRIES_PER_PAGE as u8 {
            if source.get_entry_state(item_index) != EntryMapState::Written {
                item_index += 1;
                continue;
            }

            let item = source.load_item(&mut self.hal, item_index)?;

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
}
