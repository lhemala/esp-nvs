//! Legacy single-page blobs, the `BLOB` (0x41) items ESP-IDF wrote before multi-page blobs.
//!
//! They are never written by this crate, but a partition coming from an older ESP-IDF version can
//! hold them. `Nvs::new` rewrites them as multi-page blobs; one it has no room for stays readable
//! as it is.

use esp_nvs::error::Error;
use esp_nvs::{
    ENTRIES_PER_PAGE,
    FLASH_SECTOR_SIZE,
    ITEM_SIZE,
    ItemType,
    Key,
};
use pretty_assertions::assert_eq;

mod common;

fn namespace() -> Key {
    Key::from_str("ns")
}

fn key() -> Key {
    Key::from_str("legacy")
}

/// The value the legacy blob holds. It is written as a string and retyped, as both share a layout,
/// so it ends in the string's NUL terminator.
const VALUE: &[u8] = b"legacy blob\0";

/// Finds the header of the item with `key` and type `type_`, as a byte offset.
fn find_header(buf: &[u8], type_: ItemType, key: &Key) -> Option<usize> {
    (0..buf.len() / FLASH_SECTOR_SIZE)
        .flat_map(|page| {
            (0..ENTRIES_PER_PAGE).map(move |entry| page * FLASH_SECTOR_SIZE + common::ITEM_OFFSET + entry * ITEM_SIZE)
        })
        .find(|&offset| {
            buf[offset + 1] == type_ as u8
                && buf[offset + common::ITEM_KEY_OFFSET..offset + common::ITEM_DATA_OFFSET] == key.as_bytes()[..]
                && common::is_item_header(buf, offset)
        })
}

/// The entry map state of the entry whose header is at byte offset `header`.
fn state_of(buf: &[u8], header: usize) -> u8 {
    let page = header / FLASH_SECTOR_SIZE * FLASH_SECTOR_SIZE;
    common::entry_state(buf, page, (header - page - common::ITEM_OFFSET) / ITEM_SIZE)
}

/// A partition of `pages` pages with a legacy blob under `key()`, a u8 `other` next to it, and then
/// `filler` more u8 keys `f0..`.
fn partition_with_legacy_blob(pages: usize, filler: usize) -> common::Flash {
    let mut flash = common::Flash::new(pages);
    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        let value = core::str::from_utf8(&VALUE[..VALUE.len() - 1]).unwrap();
        nvs.set(&namespace(), &key(), value).unwrap();
        nvs.set(&namespace(), &Key::from_str("other"), 7u8).unwrap();
        for i in 0..filler {
            nvs.set(&namespace(), &Key::from_str(&format!("f{i}")), 1u8).unwrap();
        }
    }
    let header = find_header(&flash.buf, ItemType::Sized, &key()).unwrap();
    flash.buf[header + 1] = ItemType::Blob as u8;
    let crc = common::item_crc(&flash.buf[header..header + ITEM_SIZE]);
    flash.buf[header + common::ITEM_CRC_OFFSET..header + common::ITEM_KEY_OFFSET].copy_from_slice(&crc.to_le_bytes());
    flash
}

#[test]
fn a_legacy_blob_is_migrated_at_boot() {
    let mut flash = partition_with_legacy_blob(3, 0);
    let legacy = find_header(&flash.buf, ItemType::Blob, &key()).unwrap();
    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Ok(VALUE.to_vec()));
        let entries: Vec<_> = nvs.typed_entries().collect::<Result<_, _>>().unwrap();
        assert_eq!(
            entries,
            vec![
                (namespace(), Key::from_str("other"), ItemType::U8),
                (namespace(), key(), ItemType::BlobIndex)
            ]
        );
    }
    assert_eq!(state_of(&flash.buf, legacy), 0b00, "the legacy item is erased");

    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Ok(VALUE.to_vec()));
    assert_eq!(nvs.get::<u8>(&namespace(), &Key::from_str("other")), Ok(7));
}

/// Without room for the multi-page copy, the legacy blob stays as it is, and is read, listed and
/// deleted like any other value.
#[test]
fn a_legacy_blob_without_room_to_migrate_stays_readable() {
    // The namespace entry, the legacy blob (one header and one data entry), `other` and the filler
    // fill both pages of a three page partition; the third is the reserve.
    let mut flash = partition_with_legacy_blob(3, 2 * ENTRIES_PER_PAGE - 4);
    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Ok(VALUE.to_vec()));
    let entries: Vec<_> = nvs.typed_entries().collect::<Result<_, _>>().unwrap();
    assert!(entries.contains(&(namespace(), key(), ItemType::Blob)));
    let keys: Vec<_> = nvs.keys().collect::<Result<_, _>>().unwrap();
    assert!(keys.contains(&(namespace(), key())));

    nvs.delete(&namespace(), &key()).unwrap();
    assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Err(Error::KeyNotFound));
    drop(nvs);
    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Err(Error::KeyNotFound));
    assert_eq!(nvs.get::<u8>(&namespace(), &Key::from_str("other")), Ok(7));
}

/// A legacy blob whose entries were written but not yet marked in the entry map is recovered like a
/// string, header and payload, and then migrated. It used to be counted as a single entry with its
/// payload left uncounted, so the next write landed on top of it.
#[test]
fn a_legacy_blob_not_yet_marked_written_is_recovered() {
    let mut flash = partition_with_legacy_blob(3, 0);
    let header = find_header(&flash.buf, ItemType::Blob, &key()).unwrap();
    let entry = (header - common::ITEM_OFFSET) / ITEM_SIZE;
    let span = flash.buf[header + 2] as usize;
    for e in entry..entry + span {
        flash.buf[common::ENTRY_STATE_MAP_OFFSET + e / 4] |= 0b11 << ((e % 4) * 2);
    }

    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Ok(VALUE.to_vec()));
        nvs.set(&namespace(), &Key::from_str("next"), 1u32).unwrap();
    }
    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Ok(VALUE.to_vec()));
    assert_eq!(nvs.get::<u32>(&namespace(), &Key::from_str("next")), Ok(1));
}

/// The migration cut short by a power loss at every flash operation it takes. The value has to
/// survive, and end up migrated after the next boot.
#[test]
fn power_loss_while_migrating_a_legacy_blob() {
    let image = partition_with_legacy_blob(4, 0).buf;
    for budget in 0.. {
        let mut flash = common::Flash::new(4);
        flash.buf = image.clone();
        flash.arm_fault(budget);
        let migrated = esp_nvs::Nvs::new(0, flash.len(), &mut flash).is_ok();
        flash.disable_faults();

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            assert_eq!(
                nvs.get::<Vec<u8>>(&namespace(), &key()),
                Ok(VALUE.to_vec()),
                "power lost at operation {budget}"
            );
            assert_eq!(nvs.get::<u8>(&namespace(), &Key::from_str("other")), Ok(7));
        }
        let legacy = find_header(&flash.buf, ItemType::Blob, &key()).unwrap();
        assert_eq!(state_of(&flash.buf, legacy), 0b00, "power lost at operation {budget}");

        if migrated {
            break;
        }
    }
}
