//! Getting hold of the NVS encryption keys.
//!
//! ESP-IDF supports two schemes, both of which end up with the same 64 byte `eky || tky`:
//!
//! * an `nvs_keys` partition, protected by flash encryption - see [`from_key_partition`]
//! * derivation from a key burnt into eFuse with purpose `HMAC_UP` - see [`derive_keys`]

use crate::encryption::NVS_KEY_SIZE;
use crate::error::Error;
use crate::platform::FnCrc32;

/// Size of an `nvs_keys` partition.
pub const KEY_PARTITION_SIZE: usize = 4096;


/// See `EKEY_SEED` in [nvs_sec_provider_private.h](https://github.com/espressif/esp-idf/blob/08e0d30a74ad0bfd5a34933142b80f45619ee410/components/nvs_sec_provider/include/private/nvs_sec_provider_private.h).
const EKEY_SEED: u32 = 0xAEBE_5A5A;
/// See `TKEY_SEED` in [nvs_sec_provider_private.h](https://github.com/espressif/esp-idf/blob/08e0d30a74ad0bfd5a34933142b80f45619ee410/components/nvs_sec_provider/include/private/nvs_sec_provider_private.h).
const TKEY_SEED: u32 = 0xCEDE_A5A5;

fn seed(value: u32) -> [u8; 32] {
    let mut seed = [0u8; 32];
    for word in seed.chunks_exact_mut(4) {
        word.copy_from_slice(&value.to_le_bytes());
    }
    seed
}

/// Derives the NVS keys from an eFuse key, the way ESP-IDF's HMAC based scheme does.
///
/// `hmac_sha256` has to compute HMAC-SHA256 of the given message under the eFuse key, which is
/// what the HMAC peripheral in upstream mode does. On esp-hal, use
/// [`derive_keys_from_efuse`](crate::keys::derive_keys_from_efuse) instead of wiring it up
/// yourself.
pub fn derive_keys<E>(
    mut hmac_sha256: impl FnMut(&[u8; 32], &mut [u8; 32]) -> Result<(), E>,
) -> Result<[u8; NVS_KEY_SIZE], E> {
    let mut keys = [0u8; NVS_KEY_SIZE];
    let (encryption_key, tweak_key) = keys.split_at_mut(NVS_KEY_SIZE / 2);

    hmac_sha256(&seed(EKEY_SEED), encryption_key.try_into().unwrap())?;
    hmac_sha256(&seed(TKEY_SEED), tweak_key.try_into().unwrap())?;

    Ok(keys)
}

/// Takes the keys out of the contents of an `nvs_keys` partition and checks their CRC32.
///
/// Note that on a device with flash encryption enabled - which is the whole point of that
/// partition - reading it through [`ReadNorFlash`](embedded_storage::nor_flash::ReadNorFlash)
/// yields cipher text, because the transparent decryption happens below that layer. Pass in the
/// decrypted contents.
pub fn from_key_partition(partition: &[u8], crc32: FnCrc32) -> Result<[u8; NVS_KEY_SIZE], Error> {
    let (keys, crc) = partition
        .split_at_checked(NVS_KEY_SIZE)
        .and_then(|(keys, rest)| Some((keys, rest.get(..4)?)))
        .ok_or(Error::InvalidKeys)?;

    if crc32(u32::MAX, keys) != u32::from_le_bytes(crc.try_into().unwrap()) {
        return Err(Error::InvalidKeys);
    }

    Ok(keys.try_into().unwrap())
}

/// Derives the NVS keys with the HMAC peripheral, from an eFuse key burnt with purpose `HMAC_UP`.
///
/// This is the counterpart of ESP-IDF's `nvs_sec_provider_register_hmac()`, so a partition written
/// by either side can be read by the other.
#[cfg(feature = "hmac-keys")]
pub fn derive_keys_from_efuse(
    hmac: &mut esp_hal::hmac::Hmac<'_>,
    key_id: esp_hal::hmac::KeyId,
) -> Result<[u8; NVS_KEY_SIZE], esp_hal::hmac::Error> {
    use esp_hal::hmac::HmacPurpose;

    derive_keys(|message, output| {
        hmac.init();
        nb::block!(hmac.configure(HmacPurpose::ToUser, key_id))?;

        let mut remaining = &message[..];
        while !remaining.is_empty() {
            remaining = nb::block!(hmac.update(remaining)).unwrap();
        }
        nb::block!(hmac.finalize(output)).unwrap();

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seeds are `uint32_t seed[8]` in ESP-IDF, so they end up as little endian words.
    #[test]
    fn seeds_match_esp_idf() {
        assert_eq!(seed(EKEY_SEED), [0x5A, 0x5A, 0xBE, 0xAE].repeat(8)[..]);
        assert_eq!(seed(TKEY_SEED), [0xA5, 0xA5, 0xDE, 0xCE].repeat(8)[..]);
    }

    /// The encryption key is derived first and takes the lower half of the key material.
    #[test]
    fn keys_are_derived_in_order() {
        let mut messages = alloc::vec![];

        let keys = derive_keys::<()>(|message, output| {
            messages.push(*message);
            output.fill(messages.len() as u8);
            Ok(())
        })
        .unwrap();

        assert_eq!(messages, [seed(EKEY_SEED), seed(TKEY_SEED)]);
        assert_eq!(keys[..32], [1u8; 32]);
        assert_eq!(keys[32..], [2u8; 32]);
    }

    #[test]
    fn key_partition_crc_is_checked() {
        let crc32 = crate::platform::software_crc32;
        let mut partition = alloc::vec![0xAAu8; KEY_PARTITION_SIZE];
        let crc = crc32(u32::MAX, &partition[..NVS_KEY_SIZE]);
        partition[NVS_KEY_SIZE..NVS_KEY_SIZE + 4].copy_from_slice(&crc.to_le_bytes());

        assert_eq!(from_key_partition(&partition, crc32).unwrap(), [0xAAu8; NVS_KEY_SIZE]);

        partition[0] ^= 1;
        assert_eq!(from_key_partition(&partition, crc32), Err(Error::InvalidKeys));

        assert_eq!(from_key_partition(&partition[..8], crc32), Err(Error::InvalidKeys));
    }
}
