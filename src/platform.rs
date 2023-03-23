use embedded_storage::nor_flash::NorFlash;

/// See README.md for an example implementation.
pub trait Platform: Crc + NorFlash {}

impl<T: Crc + NorFlash> Platform for T {}

pub type FnCrc32 = fn(init: u32, data: &[u8]) -> u32;

pub trait Crc {
    fn crc32(init: u32, data: &[u8]) -> u32;
}

#[cfg(any(
    feature = "esp32",
    feature = "esp32s2",
    feature = "esp32s3",
    feature = "esp32c2",
    feature = "esp32c3",
    feature = "esp32c6",
    feature = "esp32h2",
))]
mod chip {
    use crate::platform::Crc;
    use embedded_storage::nor_flash::{ErrorType, NorFlash};
    use esp_storage::{FlashStorage, FlashStorageError};

    pub struct EspFlash<'d> {
        inner: FlashStorage<'d>,
    }

    impl<'d> EspFlash<'d> {
        pub fn new(inner: FlashStorage<'d>) -> Self {
            Self { inner }
        }
    }

    impl ErrorType for EspFlash<'_> {
        type Error = FlashStorageError;
    }

    impl NorFlash for EspFlash<'_> {
        const WRITE_SIZE: usize = FlashStorage::WRITE_SIZE;
        const ERASE_SIZE: usize = FlashStorage::ERASE_SIZE;

        fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
            // info!("erase: from {:x}: to {:x}", from, to);
            self.inner.erase(from, to)
        }

        fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
            // info!("write: offset {:x}: bytes {}", offset, bytes.len());
            self.inner.write(offset, bytes)
        }
    }

    impl embedded_storage::nor_flash::ReadNorFlash for EspFlash<'_> {
        const READ_SIZE: usize = FlashStorage::READ_SIZE;

        fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
            // info!("read: offset {:x}: bytes {}", offset, bytes.len());
            self.inner.read(offset, bytes)
        }

        fn capacity(&self) -> usize {
            self.inner.capacity()
        }
    }

    impl Crc for EspFlash<'_> {
        fn crc32(init: u32, data: &[u8]) -> u32 {
            esp_hal::rom::crc::crc32_le(init, data)
        }
    }
    impl Crc for &mut EspFlash<'_> {
        fn crc32(init: u32, data: &[u8]) -> u32 {
            esp_hal::rom::crc::crc32_le(init, data)
        }
    }
}

#[cfg(any(
    feature = "esp32",
    feature = "esp32s2",
    feature = "esp32s3",
    feature = "esp32c2",
    feature = "esp32c3",
    feature = "esp32c6",
    feature = "esp32h2",
))]
pub use chip::*;
