//! Item-level NVS operations: get, set, and delete.
//!
//! This module contains the internal implementation for reading, writing,
//! and deleting items (primitives, strings, blobs) from NVS storage.

use alloc::string::{
    String,
    ToString,
};
use alloc::vec;
use alloc::vec::Vec;
use core::cmp;
use core::mem::size_of;

#[cfg(feature = "defmt")]
use defmt::trace;

use crate::error::Error;
use crate::error::Error::{
    ItemTypeMismatch,
    KeyNotFound,
};
use crate::page::{
    ThinPage,
    ThinPageState,
};
use crate::platform::Platform;
use crate::raw::{
    Item,
    ItemData,
    ItemDataBlobIndex,
    ItemType,
    MAX_BLOB_CHUNK_COUNT,
    MAX_BLOB_DATA_PER_PAGE,
    MAX_BLOB_SIZE,
};
use crate::types::{
    ChunkIndex,
    ItemIndex,
    PageIndex,
    VersionOffset,
};
use crate::{
    Key,
    MAX_KEY_LENGTH,
    Nvs,
    raw,
};

impl<T> Nvs<T>
where
    T: Platform,
{
    pub(crate) fn get_primitive(&mut self, namespace: &Key, key: &Key, type_: ItemType) -> Result<u64, Error> {
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

        let namespace_index = *self.namespaces.get(namespace).ok_or(Error::NamespaceNotFound)?;

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

        let namespace_index = *self.namespaces.get(namespace).ok_or(Error::NamespaceNotFound)?;

        let (page_index, item_index, item) = self.load_item(namespace_index, ChunkIndex::Any, key)?;

        if item.type_ != ItemType::Sized {
            return Err(ItemTypeMismatch(item.type_));
        }

        let page = &self.pages[page_index.0];
        let data = page.load_referenced_data(&mut self.hal, item_index.0, &item)?;

        let crc = unsafe { item.data.sized.crc };
        if crc != T::crc32(u32::MAX, &data) {
            return Err(Error::KeyNotFound);
        }

        // A stored string always carries its null terminator, so an empty payload means the size on
        // flash is corrupt. Without this the slice below underflows: a panic in debug, and in
        // release a wrapped `usize::MAX` length that panics on the slice instead. Neither is
        // something a `get` should do to the caller.
        if data.is_empty() {
            return Err(Error::CorruptedData);
        }

        let str = core::str::from_utf8(&data[..data.len() - 1]).map_err(|_| Error::CorruptedData)?; // we don't want the null terminator
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

        let namespace_index = *self.namespaces.get(namespace).ok_or(Error::NamespaceNotFound)?;

        let (page_index, item_index, item) = self.load_item(namespace_index, ChunkIndex::Any, key)?;

        if item.type_ == ItemType::BlobIndex {
            let size = unsafe { item.data.blob_index.size };

            // Checked before allocating: a corrupt size would otherwise ask for up to 4 GiB below.
            if size as usize > MAX_BLOB_SIZE {
                return Err(Error::CorruptedData);
            }

            let chunk_count = unsafe { item.data.blob_index.chunk_count };
            let chunk_start = unsafe { item.data.blob_index.chunk_start };

            // Both come straight from flash, so their sum can overflow on a corrupt index.
            let chunk_end = chunk_start.checked_add(chunk_count).ok_or(Error::CorruptedData)?;

            let mut buf = vec![0u8; size as usize];
            let mut offset = 0usize;

            for chunk in chunk_start..chunk_end {
                let (page_index, item_index, item) =
                    match self.load_item(namespace_index, ChunkIndex::BlobData(chunk), key) {
                        // The index promises this chunk, so a missing one means the stored blob is
                        // incomplete, not that the key was never written.
                        Err(Error::KeyNotFound) => return Err(Error::CorruptedData),
                        result => result?,
                    };

                if item.type_ != ItemType::BlobData {
                    return Err(ItemTypeMismatch(item.type_));
                }

                let page = &self.pages[page_index.0];
                let data = page.load_referenced_data(&mut self.hal, item_index.0, &item)?;

                let data_crc = unsafe { item.data.sized.crc };
                if data_crc != T::crc32(u32::MAX, &data) {
                    return Err(Error::CorruptedData);
                }

                // The chunks hold more data than the index claims, so index and chunks disagree.
                // Copying only the part that still fits would silently return a truncated blob.
                if offset + data.len() > buf.len() {
                    return Err(Error::CorruptedData);
                }

                buf[offset..offset + data.len()].copy_from_slice(&data);
                offset += data.len();
            }

            // The chunks hold less data than the index claims; the tail of `buf` would still be
            // zero, which would silently return a zero padded blob.
            if offset != buf.len() {
                return Err(Error::CorruptedData);
            }

            Ok(buf)
        } else if item.type_ == ItemType::Blob {
            // Legacy single-page blob (version 1 format) — same layout as Sized
            let page = &self.pages[page_index.0];
            let data = page.load_referenced_data(&mut self.hal, item_index.0, &item)?;

            let crc = unsafe { item.data.sized.crc };
            if crc != T::crc32(u32::MAX, &data) {
                return Err(Error::CorruptedData);
            }

            Ok(data)
        } else {
            Err(ItemTypeMismatch(item.type_))
        }
    }

    pub(crate) fn delete_key(&mut self, namespace_index: u8, key: &Key, chunk_index: ChunkIndex) -> Result<(), Error> {
        #[cfg(feature = "defmt")]
        trace!("delete_key");

        #[cfg(feature = "debug-logs")]
        println!("internal: delete_key");

        let (page_index, item_index, item) = self.load_item(namespace_index, chunk_index.clone(), key)?;

        let page = self.pages.get_mut(page_index.0).unwrap();

        page.erase_item::<T>(&mut self.hal, item_index.0, item.span)?;
        if self.purge {
            page.purge_entries::<T>(&mut self.hal, item_index.0, item.span)?;
        }

        // If we deleted a BLOB_IDX we need to delete all associated BLOB_DATA entries
        if item.type_ == ItemType::BlobIndex {
            self.delete_blob_data(item.namespace_index, key, unsafe {
                VersionOffset::from(item.data.blob_index.chunk_start)
            })?;
        }

        Ok(())
    }

    pub(crate) fn delete_blob_data(
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

    fn blob_is_equal(&mut self, namespace_index: u8, key: &Key, blob_item: &Item, data: &[u8]) -> Result<bool, Error> {
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
                    Some(VersionOffset::from(unsafe { item.data.blob_index.chunk_start }))
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
        let namespace_index = match self.get_or_create_namespace(namespace, &mut page) {
            Ok(namespace_index) => namespace_index,
            Err(e) => {
                // `get_active_page` popped the page out of `self.pages`, so returning without it
                // would drop every live entry on it from this instance and leak its sector, where
                // no defragmentation can reach it again.
                self.pages.push(page);
                return Err(e);
            }
        };

        // page might be full after creating a new namespace
        if page.is_full() {
            let result = page.mark_as_full(&mut self.hal);
            // Retired or not, the page belongs back in the list before another one is taken.
            self.pages.push(page);
            result?;
            page = self.get_active_page()?;
        }

        // the active page needs to be in the vec for it to be considered by load_item()
        self.pages.push(page);

        let old_entry_location =
            if let Ok((page_index, item_index, item)) = self.load_item(namespace_index, ChunkIndex::Any, &key) {
                if item.type_ == type_ && unsafe { item.data.raw } == raw_value {
                    #[cfg(feature = "debug-logs")]
                    println!("internal: set_primitive: entry already exists and matches");
                    return Ok(());
                }

                #[cfg(feature = "debug-logs")]
                println!("internal: set_primitive: entry already exists and needs to be removed");

                // The span and the type are taken from the item found here rather than assumed: the
                // key may hold a string spanning several entries, or a blob index whose chunks have
                // to go as well.
                let old_blob_start = if item.type_ == ItemType::BlobIndex {
                    Some(unsafe { VersionOffset::from(item.data.blob_index.chunk_start) })
                } else {
                    None
                };
                Some((page_index, item_index, item.span, old_blob_start))
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

        // The old item is whatever the key held before, which is not necessarily another primitive.
        // Erasing a single entry would leave the tail of a longer item behind, and erasing a blob
        // index would orphan its chunks, so both the span and the chunks come from the item that
        // was actually found.
        if let Some((page_index, item_index, span, old_blob_start)) = old_entry_location {
            // page_index might only change on defragmentation when load_active_page()
            // is called after we got it
            let old_page = self.pages.get_mut(page_index.0).unwrap();
            old_page.erase_item(&mut self.hal, item_index.0, span)?;
            if self.purge {
                old_page.purge_entries(&mut self.hal, item_index.0, span)?;
            }
            if let Some(chunk_start) = old_blob_start {
                self.delete_blob_data(namespace_index, &key, chunk_start)?;
            }
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
        let namespace_index = match self.get_or_create_namespace(namespace, &mut page) {
            Ok(namespace_index) => namespace_index,
            Err(e) => {
                self.pages.push(page);
                return Err(e);
            }
        };

        match page.write_variable_sized_item::<T>(&mut self.hal, namespace_index, key, ItemType::Sized, None, &buf) {
            Ok(_) => {}
            Err(Error::PageFull) => {
                let retired = page.mark_as_full::<T>(&mut self.hal);
                self.pages.push(page);
                retired?;

                page = self.get_active_page()?;
                let written = page.write_variable_sized_item::<T>(
                    &mut self.hal,
                    namespace_index,
                    key,
                    ItemType::Sized,
                    None,
                    &buf,
                );
                self.pages.push(page);
                match written {
                    Ok(_) => {}
                    // The page after a retire is not guaranteed to be a fresh one: with the reserve
                    // down to one, `get_active_page` goes through `defragment`, which hands back a
                    // partially filled copy. `PageFull` is an internal signal for "try another
                    // page", and there is no other page to try, so the partition has no room for
                    // this value. Reporting it verbatim leaked an error documented as internal.
                    Err(Error::PageFull) => return Err(Error::FlashFull),
                    Err(e) => return Err(e),
                }

                // The page is already back in the list, so skip the push below.
                if let Some((_page_index, _item_index)) = old_entry_location {
                    self.delete_key(namespace_index, &key, ChunkIndex::Any)?;
                }
                return Ok(());
            }
            Err(e) => {
                self.pages.push(page);
                return Err(e);
            }
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
        // `had_old_item` also covers a key that currently holds something other than a blob, which
        // still has to be deleted once the new blob is written.
        let mut had_old_item = false;
        let should_write = if let Some(&namespace_index) = self.namespaces.get(namespace) {
            match self.load_item(namespace_index, ChunkIndex::Any, &key) {
                Ok((_page_index, _item_index, item)) => {
                    had_old_item = true;
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
        // Retiring the active page is the only move this loop can make without writing a chunk, so
        // it is also the only one that can repeat forever. `get_active_page` does not only hand out
        // fresh pages: once the reserve is down to one it goes through `defragment`, which copies a
        // page's live entries into the reserve page and hands that back as the new active page,
        // pushing the erased source back into the reserve. The free page count is invariant across
        // that, and so is the copy's free entry count, so it can keep reproducing an unusable
        // active page verbatim - each turn writing a `Full` marker, erasing a sector and copying up
        // to 126 entries.
        //
        // Hence a budget of one retire per chunk. Every retire below is guarded by this flag and
        // sets it; only a chunk that was actually written clears it again. Once the budget is
        // spent, a branch that has a way to make progress takes it (the skip falls through to a
        // partial write), and one that has none reports `FlashFull`.
        //
        // That bounds the loop. Every iteration either writes a chunk, retires a page, or returns;
        // no two retires happen without a chunk written between them, so there are at most one more
        // retire than there are chunks written. Each written chunk spends one of the
        // `MAX_BLOB_CHUNK_COUNT` indices the guard below counts, which caps the chunks at 127, so
        // the loop runs at most 2 * MAX_BLOB_CHUNK_COUNT + 1 times. The cap rests on `chunk_count`
        // alone, not on a minimum `data_len` - the last chunk of a blob can be a single byte.
        //
        // Note what the bound does not rest on either: nothing about which page `get_active_page`
        // hands back, how full it is, or what `defragment` does with the free page count. Reasoning
        // about that is what produced this loop in the first place.
        let mut retired_a_page = false;

        while offset < data.len() {
            // Chunk indices are `version_base + chunk_count`, so a blob version only owns the 128
            // wide half of the index space starting at its base, and 0xFF is reserved as "no chunk
            // index". `delete_blob_data` therefore only ever cleans up `MAX_BLOB_CHUNK_COUNT`
            // chunks. Writing one more would alias the reserved index (or leak the surplus chunk on
            // the next overwrite), so refuse the blob instead of storing it in a shape we cannot
            // read or delete again.
            //
            // Whenever the skip below can retire a partially filled page this is unreachable, since
            // every chunk is then whole and 127 of them cover any blob that passed the byte guard.
            // It is still the backstop that catches the case where the skip has to give up - a
            // partition too small for the blob, where the next page is a defragmentation target
            // rather than a fresh one.
            if chunk_count >= MAX_BLOB_CHUNK_COUNT as u8 {
                return Err(Error::ValueTooLong);
            }

            let mut page = self.get_active_page()?;

            // Calculate how much data we can fit
            let free_entries = page.get_free_entry_count();

            // A chunk needs at least two entries, one for its header and one for data, so a page
            // with fewer has to be retired before anything can be written at all.
            //
            // With the retire budget already spent this is where the spin used to start: the page
            // we get after retiring one is under no obligation to be a better one, and a
            // defragmentation target reproduced verbatim never is. There is nothing left to try -
            // the retire is this branch's only move and it has no partial write to fall back on -
            // so report that the partition has no room for the chunk. The blob's own length is not
            // at fault here, that is what the byte guard above and the chunk count guard below are
            // for, which is why this is `FlashFull` rather than `ValueTooLong`. Nor does the
            // condition look at the length: a one byte blob still needs a header entry and a data
            // entry, so it spins on a page with one free entry exactly like the largest one does.
            if free_entries <= 1 {
                if retired_a_page {
                    // `get_active_page` popped this page out of `self.pages`, so bailing out
                    // without handing it back would drop the live entries on it from this instance
                    // and leak its sector out of both `pages` and `free_pages`, where `defragment`
                    // can never reclaim it again. `FlashFull` is an error the caller is expected to
                    // handle and carry on from, so the instance has to survive it intact. Pushing
                    // an `Active` page back at the tail is the order `get_active_page` and
                    // `ensure_active_page_order` expect.
                    self.pages.push(page);
                    return Err(Error::FlashFull);
                }
                retired_a_page = true;
                page.mark_as_full::<T>(&mut self.hal)?;
                self.pages.push(page);
                continue;
            }

            // A chunk only ever takes what the active page has left, so writing the rest of a blob
            // onto a partially filled page costs the same chunk index as a whole one but stores
            // less. That would make the largest storable blob depend on how full the active page
            // happened to be. Retire such a page instead - before any chunk of this blob is
            // written, so nothing is wasted on a write that then restarts - whenever the rest of
            // the blob would no longer fit into the chunk indices that are left. The next page is
            // then either a fresh one or, in the `FlashFull` region below, a defragmentation
            // target, and a fresh one makes every following chunk whole, which pins the accepted
            // size at `MAX_BLOB_SIZE - 1` for every layout.
            //
            // This branch spends the retire budget described at `retired_a_page`. Once it is spent
            // the skip is off: falling through to the partial write always advances `offset`, so
            // the write ends in the `ValueTooLong`/`FlashFull` it reported before this skip
            // existed, rather than retiring a page per turn forever.
            //
            // The skip needs the remainder to exceed
            // `(MAX_BLOB_CHUNK_COUNT - 1) * MAX_BLOB_DATA_PER_PAGE`, which in practice only blobs
            // of that order reach, so ordinary writes do not lose a page to it. Note
            // the bound is on the remainder at this point rather than on `data.len()`:
            // a shorter blob whose earlier chunks came out partial could reach it
            // algebraically, it has just never been possible to construct one.
            let remaining = data.len() - offset;
            let fits_here = (free_entries - 1) * size_of::<Item>();
            let fits_in_remaining_chunks = (MAX_BLOB_CHUNK_COUNT - 1 - chunk_count as usize) * MAX_BLOB_DATA_PER_PAGE;
            if !retired_a_page && remaining > fits_here + fits_in_remaining_chunks {
                retired_a_page = true;
                page.mark_as_full::<T>(&mut self.hal)?;
                self.pages.push(page);
                continue;
            }

            let data_len = cmp::min(fits_here, remaining);

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
                    retired_a_page = false;
                    self.pages.push(page);
                }
                // Provably unreachable: `data_len <= (free_entries - 1) * size_of::<Item>()` gives
                // `span <= free_entries`, which is exactly the check `write_variable_sized_item`
                // reports `PageFull` from. The arm is kept as a backstop, and it spends the retire
                // budget like every other retire so that the bound on the loop holds even if that
                // stops being true.
                Err(Error::PageFull) => {
                    if retired_a_page {
                        // See the same bail-out above: the page has to go back into `self.pages`.
                        self.pages.push(page);
                        return Err(Error::FlashFull);
                    }
                    retired_a_page = true;
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

        // Now that the new blob version has been successfully written, delete whatever the key held
        // before. That is gated on an item having been there at all rather than on a previous blob
        // version: a key holding a string or a primitive has no blob version, and leaving that item
        // in place used to shadow the blob just written, since `load_item` returns the older of the
        // two. The write then reported success while the value could never be read back.
        //
        // Which item is deleted is not passed in, because it is bound to be the first one found
        // anyway as newer pages appear later in self.pages. `ChunkIndex::Any` hashes the same as
        // `ChunkIndex::BlobIndex`, so this finds an old blob index just as well as a foreign item.
        if had_old_item {
            self.delete_key(namespace_index, &key, ChunkIndex::Any)?;
        }

        Ok(())
    }

    pub(crate) fn get_active_page(&mut self) -> Result<ThinPage, Error> {
        #[cfg(feature = "defmt")]
        trace!("get_active_page");

        let page = self.pages.pop_if(|page| page.header.state == ThinPageState::Active);
        if let Some(page) = page {
            return Ok(page);
        }

        // Only try reclamation if we have no free pages left
        if self.free_pages.len() == 1 {
            self.defragment()?;
        }

        let page = self.pages.pop_if(|page| page.header.state == ThinPageState::Active);
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
                .erase(page.address as _, (page.address + raw::FLASH_SECTOR_SIZE) as _)
                .map_err(|_| Error::FlashError)?;
        }

        let next_sequence = self.get_next_sequence();
        page.initialize(&mut self.hal, next_sequence)?;

        Ok(page)
    }

    pub(crate) fn get_next_sequence(&self) -> u32 {
        match self.pages.iter().map(|page| page.header.sequence).max() {
            Some(current) => current + 1,
            None => 0,
        }
    }

    pub(crate) fn get_or_create_namespace(&mut self, namespace: &Key, page: &mut ThinPage) -> Result<u8, Error> {
        #[cfg(feature = "defmt")]
        trace!("get_or_create_namespace");

        #[cfg(feature = "debug-logs")]
        println!("internal: get_or_create_namespace");

        let namespace_index = match self.namespaces.get(namespace) {
            Some(ns_idx) => *ns_idx,
            None => {
                let namespace_index = match self.namespaces.iter().max_by_key(|(_, idx)| **idx) {
                    Some((_, idx)) => idx.checked_add(1).ok_or(Error::FlashFull)?,
                    None => 1,
                };

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
}
