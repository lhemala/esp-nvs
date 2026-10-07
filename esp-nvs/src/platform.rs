//! Platform abstraction traits for NVS.
//!
//! This module defines the [`Platform`] trait that combines flash storage
//! and CRC capabilities required by the NVS driver.

use embedded_storage::nor_flash::NorFlash;

/// See README.md for an example implementation.
pub trait Platform: Crc + NorFlash {}

impl<T: Crc + NorFlash> Platform for T {}

pub type FnCrc32 = fn(init: u32, data: &[u8]) -> u32;

pub trait Crc {
    fn crc32(init: u32, data: &[u8]) -> u32;
}

impl<T: Crc> Crc for &mut T {
    fn crc32(init: u32, data: &[u8]) -> u32 {
        T::crc32(init, data)
    }
}

/// Software CRC32 using the IEEE 802.3 polynomial (0xEDB88320).
///
/// This follows the zlib convention: the internal state is obtained by XOR-ing
/// `init` with `0xFFFFFFFF` before processing, and the returned value is the
/// internal state XOR-ed with `0xFFFFFFFF` after processing.
///
/// For a standard one-shot CRC32, pass `init = 0`.
///
/// For incremental/streaming use, pass the previous return value as `init`.
///
/// This function is compatible with `libz_sys::crc32` and the ESP-IDF ROM
/// `crc32_le` function, and can be used to implement the [`Crc`] trait on
/// host platforms without a C library dependency.
pub fn software_crc32(init: u32, data: &[u8]) -> u32 {
    let mut crc = init ^ 0xFFFFFFFF;

    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }

    crc ^ 0xFFFFFFFF
}

pub trait AlignedOps: Platform {
    fn align_read(size: usize) -> usize {
        align_ceil(size, Self::READ_SIZE)
    }

    fn align_write_ceil(size: usize) -> usize {
        align_ceil(size, Self::WRITE_SIZE)
    }

    fn align_write_floor(size: usize) -> usize {
        align_floor(size, Self::WRITE_SIZE)
    }
}

#[inline(always)]
const fn align_ceil(size: usize, alignment: usize) -> usize {
    // The mask only works for an alignment that is a power of two, whatever `size` is.
    if alignment.is_power_of_two() {
        size.saturating_add(alignment - 1) & !(alignment - 1)
    } else {
        size.saturating_add(alignment - 1) / alignment * alignment
    }
}

#[inline(always)]
const fn align_floor(size: usize, alignment: usize) -> usize {
    if alignment.is_power_of_two() {
        size & !(alignment - 1)
    } else {
        size / alignment * alignment
    }
}

impl<T: Platform> AlignedOps for T {}

#[cfg(any(
    feature = "esp32",
    feature = "esp32s2",
    feature = "esp32s3",
    feature = "esp32c2",
    feature = "esp32c3",
    feature = "esp32c5",
    feature = "esp32c6",
    feature = "esp32h2",
))]
mod chip {
    use esp_storage::FlashStorage;

    use crate::platform::Crc;

    impl Crc for FlashStorage<'_> {
        fn crc32(init: u32, data: &[u8]) -> u32 {
            esp_hal::rom::crc::crc32_le(init, data)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        align_ceil,
        align_floor,
    };

    /// The power of two shortcut was taken by looking at `size` instead of `alignment`, which
    /// masks with `alignment - 1` and is wrong for any other alignment: 8 aligned up to 3 came
    /// out as 8 and down to 3 as 8.
    #[test]
    fn aligns_to_an_alignment_that_is_not_a_power_of_two() {
        assert_eq!(align_ceil(8, 3), 9);
        assert_eq!(align_floor(8, 3), 6);
        assert_eq!(align_ceil(7, 3), 9);
        assert_eq!(align_floor(7, 3), 6);
    }

    #[test]
    fn aligns_to_a_power_of_two() {
        assert_eq!(align_ceil(5, 4), 8);
        assert_eq!(align_floor(5, 4), 4);
        assert_eq!(align_ceil(8, 4), 8);
        assert_eq!(align_floor(8, 4), 8);
    }
}
