#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

// filename according to https://doc.rust-lang.org/book/ch11-03-test-organization.html
use embedded_storage::nor_flash::{
    ErrorType,
    NorFlash,
    NorFlashError,
    NorFlashErrorKind,
    ReadNorFlash,
};
use esp_nvs::ENTRY_STATE_BITMAP_SIZE;
pub use esp_nvs::{
    FLASH_SECTOR_SIZE,
    PAGE_HEADER_SIZE,
};

// Taken from https://github.com/esp-rs/esp-hal/blob/main/esp-storage/src/stub.rs
pub const WORD_SIZE: usize = 4;
pub const ENTRY_STATE_MAP_OFFSET: usize = PAGE_HEADER_SIZE;
pub const ENTRY_STATE_MAP_SIZE: usize = ENTRY_STATE_BITMAP_SIZE;
pub const ENTRY_STATE_MAP_ENTRY_SIZE: usize = 1;

pub const ITEM_OFFSET: usize = PAGE_HEADER_SIZE + ENTRY_STATE_MAP_SIZE;
// 1 byte is the minimum that can be written

// Byte offsets within a 32 byte entry, mirroring `raw::Item`:
// namespace_index(1) type_(1) span(1) chunk_index(1) crc(4) key(16) data(8).
pub const ITEM_CRC_OFFSET: usize = 4;
pub const ITEM_KEY_OFFSET: usize = 8;
pub const ITEM_DATA_OFFSET: usize = 24;

/// `EntryMapState::Written` as stored in the two-bit-per-entry entry state bitmap.
pub const ENTRY_STATE_WRITTEN: u8 = 0b10;

/// Reads an entry's two-bit state out of its page's entry state bitmap.
pub fn entry_state(buf: &[u8], page_start: usize, entry: usize) -> u8 {
    let byte = buf[page_start + ENTRY_STATE_MAP_OFFSET + entry / 4];
    (byte >> ((entry % 4) * 2)) & 0b11
}

/// Recomputes the CRC an item header stores in bytes 4..8: it covers the first four header bytes,
/// the 16 byte key and the 8 byte data union, but not the CRC itself.
///
/// This is also what tells an item header apart from a payload entry, which carries no CRC of its
/// own and so is very unlikely to match: without it, a raw data byte that happens to look like an
/// item type is picked up as an item.
pub fn item_crc(entry: &[u8]) -> u32 {
    let crc = esp_nvs::platform::software_crc32(u32::MAX, &entry[0..ITEM_CRC_OFFSET]);
    let crc = esp_nvs::platform::software_crc32(crc, &entry[ITEM_KEY_OFFSET..ITEM_DATA_OFFSET]);
    esp_nvs::platform::software_crc32(crc, &entry[ITEM_DATA_OFFSET..esp_nvs::ITEM_SIZE])
}

/// Whether the entry at `offset` is a real item header rather than a payload entry.
pub fn is_item_header(buf: &[u8], offset: usize) -> bool {
    let entry = &buf[offset..offset + esp_nvs::ITEM_SIZE];
    u32::from_le_bytes(entry[ITEM_CRC_OFFSET..ITEM_KEY_OFFSET].try_into().unwrap()) == item_crc(entry)
}

#[derive(Default)]
pub struct Flash {
    pub buf: Vec<u8>,
    pub fail_after_operation: usize,
    pub operations: Vec<Operation>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Operation {
    Read { offset: u32, len: usize },
    Write { offset: u32, len: usize },
    Erase { offset: u32, len: usize },
}

impl Flash {
    pub fn new(pages: usize) -> Self {
        Self {
            buf: vec![0xffu8; FLASH_SECTOR_SIZE * pages],
            fail_after_operation: usize::MAX,
            ..Default::default()
        }
    }

    pub fn new_with_fault(pages: usize, fail_after_operation: usize) -> Self {
        Self {
            buf: vec![0xffu8; FLASH_SECTOR_SIZE * pages],
            fail_after_operation,
            ..Default::default()
        }
    }

    pub fn new_from_file(path: &str) -> Self {
        let partition = std::fs::read(path).unwrap();
        Self {
            buf: partition,
            fail_after_operation: usize::MAX,
            ..Default::default()
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn disable_faults(&mut self) {
        self.fail_after_operation = usize::MAX;
    }

    /// Fails every operation past the next `budget` ones.
    ///
    /// Same mechanism as [`Flash::new_with_fault`], only counted from here rather than from the
    /// first operation, so a test can set up a large partition and still hand the operation under
    /// test a budget of its own.
    pub fn arm_fault(&mut self, budget: usize) {
        self.fail_after_operation = self.operations.len() + budget;
    }

    pub fn erases(&mut self) -> usize {
        self.operations
            .iter()
            .filter(|op| match op {
                Operation::Erase { .. } => true,
                _ => false,
            })
            .count()
    }

    pub fn dump_operations(&self) {
        println!("Operations:");
        for op in &self.operations {
            println!("  {:?}", op);
        }
    }
}

#[derive(Debug)]
pub struct FlashError;

impl NorFlashError for FlashError {
    fn kind(&self) -> NorFlashErrorKind {
        NorFlashErrorKind::Other
    }
}

impl ErrorType for Flash {
    type Error = FlashError;
}

impl ReadNorFlash for Flash {
    const READ_SIZE: usize = WORD_SIZE;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        assert!(offset.is_multiple_of(Self::READ_SIZE as _));

        println!(
            "    flash: read:  0x{offset:04X}[0x{:04X}] #{:>2}",
            bytes.len(),
            self.operations.len()
        );
        if self.operations.len() >= self.fail_after_operation {
            println!("    flash: FAULT");
            return Err(FlashError);
        }
        self.operations.push(Operation::Read {
            offset,
            len: bytes.len(),
        });

        let offset = offset as usize;
        bytes.copy_from_slice(&self.buf[offset..offset + bytes.len()]);
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.buf.len()
    }
}

impl NorFlash for Flash {
    const WRITE_SIZE: usize = WORD_SIZE;

    const ERASE_SIZE: usize = FLASH_SECTOR_SIZE;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        assert!(from.is_multiple_of(Self::ERASE_SIZE as _));
        assert!(to.is_multiple_of(Self::ERASE_SIZE as _));

        println!("    flash: erase: {from:04X} - {to:04X} #{:>2}", self.operations.len());

        if self.operations.len() >= self.fail_after_operation {
            println!("    flash: FAULT");
            return Err(FlashError);
        }

        assert!((to - from).is_multiple_of(FLASH_SECTOR_SIZE as u32));
        assert!(from.is_multiple_of(FLASH_SECTOR_SIZE as u32));

        self.operations.push(Operation::Erase {
            offset: from,
            len: (to - from) as usize,
        });

        for addr in from..to {
            self.buf[addr as usize] = 0xff;
        }
        Ok(())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        assert!(offset.is_multiple_of(Self::WRITE_SIZE as _));
        assert!(bytes.len().is_multiple_of(Self::WRITE_SIZE as _));

        println!(
            "    flash: write: 0x{offset:04X}[0x{:04X}] #{:>2}",
            bytes.len(),
            self.operations.len()
        );

        if self.operations.len() >= self.fail_after_operation {
            println!("    flash: FAULT");
            return Err(FlashError);
        }
        assert!(bytes.len() > 0);

        self.operations.push(Operation::Write {
            offset,
            len: bytes.len(),
        });

        let offset = offset as usize;
        for (i, &val) in bytes.iter().enumerate() {
            // the esp flash we can only flip bits from 1 to 0
            // println!("0x[{:04x}] {} &= {val} = {}",  offset+i,self.buf[offset + i],
            // self.buf[offset + i] & val);
            self.buf[offset + i] &= val;
        }
        Ok(())
    }
}

impl esp_nvs::platform::Crc for Flash {
    fn crc32(init: u32, data: &[u8]) -> u32 {
        esp_nvs::platform::software_crc32(init, data)
    }
}

/// A [`Flash`] behind a shared handle.
///
/// `Nvs::new` takes ownership of the HAL, so with a plain [`Flash`] the backing buffer is
/// unreachable for as long as the `Nvs` instance lives. Cloning a `SharedFlash` hands the
/// `Nvs` one handle while the test keeps another, which allows simulating corruption that
/// appears *after* the partition has been scanned and cached.
#[derive(Clone, Default)]
pub struct SharedFlash(Rc<RefCell<Flash>>);

#[allow(clippy::len_without_is_empty)]
impl SharedFlash {
    pub fn new(pages: usize) -> Self {
        Self(Rc::new(RefCell::new(Flash::new(pages))))
    }

    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    /// Gives temporary mutable access to the raw partition image.
    pub fn with_buf<R>(&self, f: impl FnOnce(&mut Vec<u8>) -> R) -> R {
        f(&mut self.0.borrow_mut().buf)
    }
}

impl ErrorType for SharedFlash {
    type Error = FlashError;
}

impl ReadNorFlash for SharedFlash {
    const READ_SIZE: usize = WORD_SIZE;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        self.0.borrow_mut().read(offset, bytes)
    }

    fn capacity(&self) -> usize {
        self.0.borrow().buf.len()
    }
}

impl NorFlash for SharedFlash {
    const WRITE_SIZE: usize = WORD_SIZE;

    const ERASE_SIZE: usize = FLASH_SECTOR_SIZE;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        self.0.borrow_mut().erase(from, to)
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        self.0.borrow_mut().write(offset, bytes)
    }
}

impl esp_nvs::platform::Crc for SharedFlash {
    fn crc32(init: u32, data: &[u8]) -> u32 {
        esp_nvs::platform::software_crc32(init, data)
    }
}
