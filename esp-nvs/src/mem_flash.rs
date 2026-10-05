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

    /// The range of `buf` an access of `len` bytes at `offset` covers, if it lies inside the flash
    /// and is aligned to `alignment`. A real flash reports an error for anything else, and so does
    /// this one rather than panicking on an out of bounds slice.
    fn range(&self, offset: u32, len: usize, alignment: usize) -> Result<core::ops::Range<usize>, MemFlashError> {
        let start = offset as usize;
        let end = start.checked_add(len).ok_or(MemFlashError)?;
        if end > self.buf.len() || !start.is_multiple_of(alignment) || !len.is_multiple_of(alignment) {
            return Err(MemFlashError);
        }
        Ok(start..end)
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
        let range = self.range(offset, bytes.len(), Self::READ_SIZE)?;
        bytes.copy_from_slice(&self.buf[range]);
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
        let len = (to as usize).checked_sub(from as usize).ok_or(MemFlashError)?;
        let range = self.range(from, len, Self::ERASE_SIZE)?;
        self.buf[range].fill(0xFF);
        Ok(())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        let range = self.range(offset, bytes.len(), Self::WRITE_SIZE)?;
        for (target, &val) in self.buf[range].iter_mut().zip(bytes) {
            // Real NOR flash can only flip bits from 1 to 0
            *target &= val;
        }
        Ok(())
    }
}

impl Crc for MemFlash {
    fn crc32(init: u32, data: &[u8]) -> u32 {
        software_crc32(init, data)
    }
}

#[cfg(test)]
mod tests {
    use embedded_storage::nor_flash::{
        NorFlash,
        ReadNorFlash,
    };

    use super::MemFlash;
    use crate::FLASH_SECTOR_SIZE;

    #[test]
    fn out_of_bounds_access_is_an_error_not_a_panic() {
        let mut flash = MemFlash::new(1);
        let mut buf = [0u8; 8];
        assert!(flash.read(FLASH_SECTOR_SIZE as u32 - 4, &mut buf).is_err());
        assert!(flash.read(u32::MAX - 3, &mut buf).is_err());
        assert!(flash.write(FLASH_SECTOR_SIZE as u32, &buf).is_err());
        assert!(flash.erase(0, 2 * FLASH_SECTOR_SIZE as u32).is_err());
        assert!(flash.erase(FLASH_SECTOR_SIZE as u32, 0).is_err());
        assert!(flash.read(0, &mut buf).is_ok());
    }
}
