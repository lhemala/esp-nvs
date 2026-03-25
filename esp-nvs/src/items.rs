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
                    return Err(Error::CorruptedData);
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
        let namespace_index = self.get_or_create_namespace(namespace, &mut page)?;

        // page might be full after creating a new namespace
        if page.is_full() {
            page.mark_as_full(&mut self.hal)?;
            page = self.get_active_page()?;
        }

        // the active page needs to be in the vec for it to be considered by load_item()
        self.pages.push(page);

        let old_entry_location =
            if let Ok((page_index, item_index, item)) = self.load_item(namespace_index, ChunkIndex::Any, &key) {
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

        match page.write_variable_sized_item::<T>(&mut self.hal, namespace_index, key, ItemType::Sized, None, &buf) {
            Ok(_) => {}
            Err(Error::PageFull) => {
                page.mark_as_full::<T>(&mut self.hal)?;
                self.pages.push(page);

                page = self.get_active_page()?;
                page.write_variable_sized_item::<T>(&mut self.hal, namespace_index, key, ItemType::Sized, None, &buf)?;
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

        // Now that the new blob version has been successfully written, delete the old version if it
        // exists _old_version is unused since it will be the first one that is bound to be
        // found anyway as newer pages appear later in self.pages
        if let Some(_old_version) = old_blob_version {
            self.delete_key(namespace_index, &key, ChunkIndex::BlobIndex)?;
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
