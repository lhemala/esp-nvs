#![doc = include_str ! ("../README.md")]
#![cfg_attr(not(target_arch = "x86_64"), no_std)]

pub mod error;
mod get;
mod internal;
pub mod platform;
mod raw;
mod set;
mod u24;

/// Maximum Key length is 15 bytes + 1 byte for the null terminator.
const MAX_KEY_LENGTH: usize = 15;
const MAX_KEY_NUL_TERMINATED_LENGTH: usize = MAX_KEY_LENGTH + 1;

/// A 16-byte key used for NVS storage (15 characters + null terminator)
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Key([u8; MAX_KEY_NUL_TERMINATED_LENGTH]);

impl Key {
    /// Creates a 16 byte, null-padded byte array used as key for values and namespaces.
    ///
    /// Usage: `Key::from_array(b"my_key")`
    ///
    /// Tip: use a const context if possible to ensure that the key is transformed at compile time:
    ///   `let my_key = const { Key::from_array(b"my_key") };`
    pub const fn from_array<const M: usize>(src: &[u8; M]) -> Self {
        assert!(M <= MAX_KEY_LENGTH);
        let mut dst = [0u8; MAX_KEY_NUL_TERMINATED_LENGTH];
        let mut i = 0;
        while i < M {
            dst[i] = src[i];
            i += 1;
        }
        Self(dst)
    }

    /// Creates a 16 byte, null-padded byte array used as key for values and namespaces.
    ///
    /// Usage: `Key::from_slice(b"my_key")`
    ///
    /// Tip: use a const context if possible to ensure that the key is transformed at compile time:
    ///   `let my_key = const { Key::from_slice("my_key".as_bytes()) };`
    pub const fn from_slice(src: &[u8]) -> Self {
        assert!(src.len() <= MAX_KEY_LENGTH);
        let mut dst = [0u8; MAX_KEY_NUL_TERMINATED_LENGTH];
        let mut i = 0;
        while i < src.len() {
            dst[i] = src[i];
            i += 1;
        }
        Self(dst)
    }

    /// Creates a 16 byte, null-padded byte array used as key for values and namespaces.
    ///
    /// Usage: `Key::from_str("my_key")`
    ///
    /// Tip: use a const context if possible to ensure that the key is transformed at compile time:
    ///   `let my_key = const { Key::from_str("my_key") };`
    pub const fn from_str(s: &str) -> Self {
        let bytes = s.as_bytes();
        Self::from_slice(bytes)
    }

    /// Converts a key to a byte array.
    pub const fn as_bytes(&self) -> &[u8; MAX_KEY_NUL_TERMINATED_LENGTH] {
        &self.0
    }
}

impl AsRef<[u8]> for Key {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

pub use get::Get;
pub use set::Set;

extern crate alloc;

use crate::error::Error;
use crate::internal::{ChunkIndex, IterPageItems, ThinPage};
use crate::platform::Platform;
use crate::raw::{ENTRIES_PER_PAGE, FLASH_SECTOR_SIZE, Item, ItemType};
use alloc::collections::{BTreeMap, BinaryHeap};
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq)]
pub struct NvsStatistics {
    pub pages: PageStatistics,
    pub entries_per_page: Vec<EntryStatistics>,
    pub entries_overall: EntryStatistics,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageStatistics {
    pub empty: u16,
    pub active: u16,
    pub full: u16,
    pub erasing: u16,
    pub corrupted: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntryStatistics {
    pub empty: u32,
    pub written: u32,
    pub erased: u32,
    pub illegal: u32,
}

/// The Nvs struct keeps information about all pages in memory. Increases in size with
/// the numer of pages in the partition.
pub struct Nvs<T: Platform> {
    pub(crate) hal: T,
    pub(crate) base_address: usize,
    pub(crate) sectors: u16,
    pub(crate) faulted: bool,

    // set after calling self.load_sectors
    pub(crate) namespaces: BTreeMap<Key, u8>,
    pub(crate) free_pages: BinaryHeap<ThinPage>,
    pub(crate) pages: Vec<ThinPage>,
}

impl<T: Platform> Nvs<T> {
    /// Mimics the original C++ driver behavior and reads all sectors of the given partition to
    /// 1. Resolve all existing namespaces
    /// 2. Create a hashed key cache per page for quicker lookups
    /// 3. Cleanup duplicate entries
    /// 4. Cleanup of duplicated blobs or orphaned blob data
    ///
    /// Pages or entries with invalid CRC32 values are marked as corrupt and are erased when necessary
    pub fn new(partition_offset: usize, partition_size: usize, hal: T) -> Result<Nvs<T>, Error> {
        if !partition_offset.is_multiple_of(FLASH_SECTOR_SIZE) {
            return Err(Error::InvalidPartitionOffset);
        }

        if !partition_size.is_multiple_of(FLASH_SECTOR_SIZE) {
            return Err(Error::InvalidPartitionSize);
        }

        let sectors = partition_size / FLASH_SECTOR_SIZE;
        if sectors > u16::MAX as usize {
            return Err(Error::InvalidPartitionSize);
        }

        let mut nvs: Nvs<T> = Self {
            hal,
            base_address: partition_offset,
            sectors: sectors as u16,
            namespaces: BTreeMap::new(),
            free_pages: Default::default(),
            pages: Default::default(),
            faulted: false,
        };

        match nvs.load_sectors() {
            Ok(()) => Ok(nvs),
            Err(Error::FlashError) => {
                nvs.faulted = true;
                Err(Error::FlashError)
            }
            Err(e) => Err(e),
        }
    }

    /// Get a value from the flash.
    ///
    /// Supported types are bool, singed and unsigned integers up to 64-bit width, String and Vec.
    ///
    /// Both namespace and may have up to 15 characters.
    pub fn get<R>(&mut self, namespace: &Key, key: &Key) -> Result<R, Error>
    where
        Nvs<T>: Get<R>,
    {
        match Get::get(self, namespace, key) {
            Ok(val) => Ok(val),
            Err(Error::FlashError) => {
                self.faulted = true;
                Err(Error::FlashError)
            }
            Err(e) => Err(e),
        }
    }

    /// Set a value and write it to the flash
    ///
    /// Type support:
    ///  * bool, singed and unsigned integers up to 64-bit width: saved as primitive value with 32 bytes
    ///  * &str: Saved on a single page with a max size of 4000 bytes
    ///  * &[u8]: May span multiple pages, max size ~500kB
    pub fn set<R>(&mut self, namespace: &Key, key: &Key, value: R) -> Result<(), Error>
    where
        Nvs<T>: Set<R>,
    {
        if self.faulted {
            return Err(Error::FlashError);
        }

        match Set::set(self, namespace, key, value) {
            Ok(()) => Ok(()),
            Err(Error::FlashError) => {
                self.faulted = true;
                Err(Error::FlashError)
            }
            Err(e) => Err(e),
        }
    }

    /// Returns an iterator over all known namespaces.
    pub fn namespaces(&self) -> impl Iterator<Item = &Key> {
        self.namespaces.keys()
    }

    /// Returns an iterator over all keys in all namespaces.
    ///
    /// # Errors
    ///
    /// The iterator yields an error if there is a flash read error.
    pub fn keys(&mut self) -> impl Iterator<Item = Result<(Key, Key), Error>> {
        IterKeys::new(&self.pages, &mut self.hal, &self.namespaces)
    }

    /// Delete a key
    ///
    /// Ignores missing keys or the namespaces
    pub fn delete(&mut self, namespace: &Key, key: &Key) -> Result<(), Error> {
        if self.faulted {
            return Err(Error::FlashError);
        }

        if key.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::KeyMalformed);
        }
        if namespace.0[MAX_KEY_LENGTH] != b'\0' {
            return Err(Error::NamespaceMalformed);
        }

        let namespace_index = match self.namespaces.get(namespace) {
            Some(&idx) => idx,
            None => return Ok(()), // Namespace doesn't exist, that's fine
        };
        let result = self.delete_key(namespace_index, key, ChunkIndex::Any);
        match result {
            Err(Error::KeyNotFound) => Ok(()),
            Err(Error::FlashError) => {
                self.faulted = true;
                Err(Error::FlashError)
            }
            other => other,
        }
    }

    /// Returns detailed statistics about the NVS partition usage
    pub fn statistics(&mut self) -> Result<NvsStatistics, Error> {
        if self.faulted {
            return Err(Error::FlashError);
        }

        let mut page_stats = PageStatistics {
            empty: 0,
            active: 0,
            full: 0,
            erasing: 0,
            corrupted: 0,
        };

        let mut all_pages: Vec<&ThinPage> = Vec::with_capacity(self.sectors as _);
        all_pages.extend(self.pages.iter());
        all_pages.extend(self.free_pages.iter());
        // sorted for stable output as this is also used in tests
        all_pages.sort_by_key(|page| page.address);

        let entries_per_page = all_pages
            .into_iter()
            .map(|page| {
                match page.get_state() {
                    internal::ThinPageState::Active => page_stats.active += 1,
                    internal::ThinPageState::Full => page_stats.full += 1,
                    internal::ThinPageState::Freeing => page_stats.erasing += 1,
                    internal::ThinPageState::Corrupt => page_stats.corrupted += 1,
                    internal::ThinPageState::Invalid => page_stats.corrupted += 1,
                    internal::ThinPageState::Uninitialized => page_stats.empty += 1,
                }

                if *page.get_state() == internal::ThinPageState::Corrupt {
                    EntryStatistics {
                        empty: 0,
                        written: 0,
                        erased: 0,
                        illegal: ENTRIES_PER_PAGE as _,
                    }
                } else {
                    let (empty, written, erased, illegal) = page.get_entry_statistics();
                    EntryStatistics {
                        empty,
                        written,
                        erased,
                        illegal,
                    }
                }
            })
            .collect::<Vec<_>>();

        let entries_overall = entries_per_page.iter().fold(
            EntryStatistics {
                empty: 0,
                written: 0,
                erased: 0,
                illegal: 0,
            },
            |acc, x| EntryStatistics {
                empty: acc.empty + x.empty,
                written: acc.written + x.written,
                erased: acc.erased + x.erased,
                illegal: acc.illegal + x.illegal,
            },
        );

        Ok(NvsStatistics {
            pages: page_stats,
            entries_per_page,
            entries_overall,
        })
    }
}

struct IterLoadedItems<'a, T: Platform> {
    pages: &'a [ThinPage],
    current: Option<IterPageItems<'a, T>>,
}

// TODO: Skip broken pages?

impl<'a, T: Platform> IterLoadedItems<'a, T> {
    fn new(mut pages: &'a [ThinPage], hal: &'a mut T) -> Self {
        let first = pages.split_off_first();

        Self {
            pages,
            current: first.map(|page| page.items(hal)),
        }
    }
}

impl<'a, T: Platform> Iterator for IterLoadedItems<'a, T> {
    type Item = Result<Item, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        // self.current is only None if there are no pages at all
        let current = self.current.as_mut()?;

        // if the current page is exhausted, move to next page that has items (or until we run out of pages)
        while current.is_empty() {
            let next_page = self.pages.split_off_first()?;

            current.switch_to_page(next_page);
        }

        current.next()
    }
}

struct IterKeys<'a, T: Platform> {
    items: IterLoadedItems<'a, T>,
    namespaces: &'a BTreeMap<Key, u8>,
}

impl<'a, T: Platform> IterKeys<'a, T> {
    fn new(pages: &'a [ThinPage], hal: &'a mut T, namespaces: &'a BTreeMap<Key, u8>) -> Self {
        Self {
            items: IterLoadedItems::new(pages, hal),
            namespaces,
        }
    }

    fn item_to_keys(&self, item: Item) -> (Key, Key) {
        let (namespace_key, _) = self
            .namespaces
            .iter()
            .find(|(_, idx)| **idx == item.namespace_index)
            // a key should always have a namespace
            .unwrap();

        (*namespace_key, item.key)
    }
}

impl<'a, T: Platform> Iterator for IterKeys<'a, T> {
    type Item = Result<(Key, Key), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.items.next()? {
                Ok(item) => {
                    // TODO: The blobs are not handled correctly yet, when should they be skipped?

                    // Skip namespace entries (namespace_index == 0)
                    if item.namespace_index == 0 || item.type_ == ItemType::BlobData {
                        continue;
                    }

                    return Some(Ok(self.item_to_keys(item)));
                }
                Err(err) => return Some(Err(err)),
            }
        }
    }
}
