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

use crate::blob::{
    BlobIndex,
    BlobIndexEntryBlobIndexData,
};
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
    ItemIndex,
    NamespaceIndex,
    PageIndex,
    PageSequence,
    VersionOffset,
};
use crate::u24::u24;
use crate::{
    Key,
    Nvs,
};

impl<T> Nvs<T>
where
    T: Platform,
{
    pub(crate) fn cleanup_dirty_blobs(&mut self, blob_index: BlobIndex) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("cleanup_dirty_blobs");

        // Blob indices whose chunks add up, by (namespace, key), with the version they belong to.
        let mut consistent =
            BTreeMap::<(NamespaceIndex, Key), Vec<(VersionOffset, BlobIndexEntryBlobIndexData)>>::new();

        for ((namespace_index, chunk_start, key), (index, observed)) in blob_index {
            let Some(index) = index else {
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
                continue;
            };

            // Calculate total chunks and data size from all observed chunks. Summed wide, so that a
            // corrupt count cannot overflow.
            let (chunk_count, data_size) =
                observed
                    .chunks_by_page
                    .iter()
                    .fold((0u32, 0u64), |(count, size), chunk_data| {
                        (
                            count + chunk_data.chunk_count as u32,
                            size + chunk_data.data_size as u64,
                        )
                    });

            if index.chunk_count as u32 != chunk_count || index.size as u64 != data_size {
                #[cfg(feature = "debug-logs")]
                println!(
                    "internal: load_sectors: blob index data doesn't match observed data {index:?} (expected: chunk_count={}, data_size={}, got: chunk_count={}, data_size={})",
                    index.chunk_count, index.size, chunk_count, data_size
                );
                self.erase_blob_version(namespace_index, &key, chunk_start, &index)?;
                continue;
            }

            consistent
                .entry((namespace_index, key))
                .or_default()
                .push((chunk_start, index));
        }

        // Both versions of a blob survive a power loss between writing the new one and erasing the
        // old one. Keep the newer, by page sequence first, then by position on the same page.
        //
        // The one to go is erased where it was found. Deleting "the" blob index of the key instead
        // takes whichever one `load_item` comes across first, which follows the order of the pages
        // in flash rather than their age, and so could just as well throw away the newer version.
        for ((namespace_index, key), mut versions) in consistent {
            if versions.len() < 2 {
                continue;
            }
            versions.sort_by_key(|(_, index)| (index.page_sequence, index.item_index));
            let newest = versions.len() - 1;
            for (chunk_start, index) in versions.into_iter().take(newest) {
                #[cfg(feature = "debug-logs")]
                println!(
                    "internal: load_sectors: found two blob indices for the same key, deleting the older one (seq: {})",
                    index.page_sequence
                );
                self.erase_blob_version(namespace_index, &key, chunk_start, &index)?;
            }
        }

        Ok(())
    }

    /// Erases the blob index at the location given by `index`, and the chunks of its version.
    fn erase_blob_version(
        &mut self,
        namespace_index: NamespaceIndex,
        key: &Key,
        chunk_start: VersionOffset,
        index: &BlobIndexEntryBlobIndexData,
    ) -> Result<(), Error> {
        if let Some(page) = self
            .pages
            .iter_mut()
            .find(|page| page.header.sequence == index.page_sequence)
        {
            page.erase_item::<T>(&mut self.hal, index.item_index, 1)?;
        }
        self.delete_blob_data(namespace_index.0, key, chunk_start)
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

    /// Clean up duplicate entries by marking older versions as erased.
    /// This handles the write-before-delete scenario where deletion failed after successful write.
    /// Runs after `cleanup_dirty_blobs`, which leaves at most one version of each blob; a blob
    /// index is compared with the other items of its key here, and its chunks go with it.
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

                // Skip namespace entries (namespace_index == 0) and blob data. Namespace entries
                // are special and should not be cleaned up, and chunks go with
                // their blob index.
                //
                // A blob index takes part, though. `cleanup_dirty_blobs` has already settled which
                // version of a blob stays, but it only compares blobs with blobs, and a key
                // changing type to or from a blob leaves a blob index next to a
                // primitive or a string when the power goes before the old item is
                // erased. Left alone, reads keep returning the old value, and a
                // later write can resolve the key against the wrong one of the two.
                if item.namespace_index == 0 || item.type_ == ItemType::BlobData {
                    continue;
                }

                let blob_version = if item.type_ == ItemType::BlobIndex {
                    Some(VersionOffset::from(unsafe { item.data.blob_index.chunk_start }))
                } else {
                    None
                };
                items.push((
                    (NamespaceIndex(item.namespace_index), item.key),
                    (page_idx, item_index, page_seq, item.span, blob_version),
                ));
            }

            // Group by (namespace_index, key) to find actual duplicates
            let mut key_groups = BTreeMap::<_, Vec<_>>::new();
            for (key, val) in items {
                key_groups.entry(key).or_default().push(val);
            }

            // Erase older duplicates
            for ((namespace_index, key), mut group) in key_groups {
                if group.len() <= 1 {
                    continue;
                }

                // Sort by page sequence and item index (ascending = oldest first)
                group.sort_by_key(|(_, ItemIndex(idx), PageSequence(seq), _, _)| (*seq, *idx));

                // Keep the newest (last after sort), erase older ones
                let keep_count = group.len() - 1;
                for (PageIndex(page_index), ItemIndex(item_index), _, span, blob_version) in
                    group.into_iter().take(keep_count)
                {
                    let page = self.pages.get_mut(page_index).unwrap();
                    page.erase_item::<T>(&mut self.hal, item_index, span)?;
                    if let Some(chunk_start) = blob_version {
                        self.delete_blob_data(namespace_index.0, &key, chunk_start)?;
                    }
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
        //
        // A page whose every entry is in use is left out: copying it reproduces it exactly, so all
        // that would come of it is a sector erase. With only such pages left the partition is full,
        // and a caller retrying a write on it would otherwise wear out a sector per attempt.
        let next_page = self
            .pages
            .iter()
            .enumerate()
            .filter(|(_, page)| (page.used_entry_count as usize) < ENTRIES_PER_PAGE)
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

    /// Copies the live items of `source` into `target`, which then goes into `self.pages`.
    ///
    /// Items are copied byte for byte, header and payload, the way ESP-IDF does it. Rebuilding them
    /// from what was read instead dropped every item type it had no case for, the legacy
    /// single-page blob among them, and recomputed the data CRC of strings and chunks, so data that
    /// had already failed its CRC came out of a defragmentation as a valid value.
    ///
    /// An entry that does not read back as an item, or whose span does not fit the page, is not
    /// copied. It is not anything a read could return, and failing the copy over it would fail
    /// every defragmentation of this page from now on.
    pub(crate) fn copy_items(&mut self, source: &ThinPage, mut target: ThinPage) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("copy_items");

        let mut item_index = 0usize;
        while item_index < ENTRIES_PER_PAGE {
            if source.get_entry_state(item_index as u8) != EntryMapState::Written {
                item_index += 1;
                continue;
            }

            let item = match source.load_item(&mut self.hal, item_index as u8) {
                Ok(item) => item,
                Err(Error::FlashError) => {
                    self.pages.push(target);
                    return Err(Error::FlashError);
                }
                Err(_) => {
                    item_index += 1;
                    continue;
                }
            };
            let span = item.span as usize;
            if span == 0 || item_index + span > ENTRIES_PER_PAGE {
                item_index += 1;
                continue;
            }

            if let Err(e) = target.copy_item_from::<T>(&mut self.hal, source, item_index as u8, &item) {
                self.pages.push(target);
                return Err(e);
            }

            item_index += span;
        }

        self.pages.push(target);
        Ok(())
    }
}
