//! A minimal in-memory NOR flash implementation for host-side tooling and tests.
//!
//! [`MemFlash`] implements [`embedded_storage::nor_flash::NorFlash`] and
//! [`crate::platform::Crc`], making it a fully functional [`crate::platform::Platform`]
//! that can be used with [`crate::Nvs`] on any host platform without hardware
//! dependencies.

use alloc::vec;
use alloc::vec::Vec;

use embedded_storage::nor_flash::{
    ErrorType,
    NorFlash,
    NorFlashError,
    NorFlashErrorKind,
    ReadNorFlash,
};

use crate::FLASH_SECTOR_SIZE;
use crate::platform::{
    Crc,
    software_crc32,
};

const WORD_SIZE: usize = 4;

/// In-memory NOR flash that simulates real flash semantics:
///
/// - Erased state is all `0xFF`.
/// - Writes can only flip bits from 1 → 0 (bitwise AND).
/// - Erases restore a full sector to `0xFF`.
/// - Read/write alignment is 4 bytes (word size).
/// - Erase granularity is 4096 bytes (sector size).
pub struct MemFlash {
    buf: Vec<u8>,
}

impl MemFlash {
    /// Create a fresh flash of the given number of pages, filled with `0xFF`.
    pub fn new(pages: usize) -> Self {
        Self {
            buf: vec![0xFF; FLASH_SECTOR_SIZE * pages],
        }
    }

    /// Wrap existing binary data as a flash image.
    ///
    /// The data length must be a multiple of [`FLASH_SECTOR_SIZE`].
    ///
    /// # Panics
    /// Panics if `data.len()` is not a multiple of `FLASH_SECTOR_SIZE`.
    pub fn from_bytes(data: Vec<u8>) -> Self {
        assert!(
            data.len().is_multiple_of(FLASH_SECTOR_SIZE),
            "MemFlash data length {} is not a multiple of sector size {}",
            data.len(),
            FLASH_SECTOR_SIZE
        );
        Self { buf: data }
    }

    /// Consume the flash and return the underlying buffer.
    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }

    /// Return the total size of the flash in bytes.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Returns whether the flash is empty (zero bytes).
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

#[derive(Debug)]
pub struct MemFlashError;

impl NorFlashError for MemFlashError {
    fn kind(&self) -> NorFlashErrorKind {
        NorFlashErrorKind::Other
    }
}

impl ErrorType for MemFlash {
    type Error = MemFlashError;
}

impl ReadNorFlash for MemFlash {
    const READ_SIZE: usize = WORD_SIZE;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let offset = offset as usize;
        bytes.copy_from_slice(&self.buf[offset..offset + bytes.len()]);
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.buf.len()
    }
}

impl NorFlash for MemFlash {
    const WRITE_SIZE: usize = WORD_SIZE;
    const ERASE_SIZE: usize = FLASH_SECTOR_SIZE;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        for addr in from..to {
            self.buf[addr as usize] = 0xFF;
        }
        Ok(())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        let offset = offset as usize;
        for (i, &val) in bytes.iter().enumerate() {
            // Real NOR flash can only flip bits from 1 to 0
            self.buf[offset + i] &= val;
        }
        Ok(())
    }
}

impl Crc for MemFlash {
    fn crc32(init: u32, data: &[u8]) -> u32 {
        software_crc32(init, data)
    }
}
