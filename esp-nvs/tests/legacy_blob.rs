//! Legacy single-page blobs, the `BLOB` (0x41) items ESP-IDF wrote before multi-page blobs.
//!
//! They are never written by this crate, but a partition coming from an older ESP-IDF version can
//! hold them, so they are read, listed, kept through defragmentation and can be deleted or
//! overwritten, which replaces them with a multi-page blob.

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
                && buf[offset + common::ITEM_KEY_OFFSET..common::ITEM_DATA_OFFSET + offset] == key.as_bytes()[..]
                && common::is_item_header(buf, offset)
        })
}

/// A partition with a legacy blob under `key()` and a u8 `other` next to it.
fn partition_with_legacy_blob(pages: usize) -> common::Flash {
    let mut flash = common::Flash::new(pages);
    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        let value = core::str::from_utf8(&VALUE[..VALUE.len() - 1]).unwrap();
        nvs.set(&namespace(), &key(), value).unwrap();
        nvs.set(&namespace(), &Key::from_str("other"), 7u8).unwrap();
    }
    let header = find_header(&flash.buf, ItemType::Sized, &key()).unwrap();
    flash.buf[header + 1] = ItemType::Blob as u8;
    let crc = common::item_crc(&flash.buf[header..header + ITEM_SIZE]);
    flash.buf[header + common::ITEM_CRC_OFFSET..header + common::ITEM_KEY_OFFSET].copy_from_slice(&crc.to_le_bytes());
    flash
}

#[test]
fn a_legacy_blob_is_read() {
    let mut flash = partition_with_legacy_blob(3);
    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Ok(VALUE.to_vec()));
    assert_eq!(
        nvs.get::<String>(&namespace(), &key()),
        Err(Error::ItemTypeMismatch(ItemType::Blob))
    );
}

#[test]
fn a_legacy_blob_is_listed() {
    let mut flash = partition_with_legacy_blob(3);
    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    let keys: Vec<_> = nvs.keys().collect::<Result<_, _>>().unwrap();
    assert_eq!(keys, vec![(namespace(), key()), (namespace(), Key::from_str("other"))]);
    let entries: Vec<_> = nvs.typed_entries().collect::<Result<_, _>>().unwrap();
    assert_eq!(
        entries,
        vec![
            (namespace(), key(), ItemType::Blob),
            (namespace(), Key::from_str("other"), ItemType::U8)
        ]
    );
}

/// Copying rebuilt each item by type and had no case for legacy blobs, so one was silently dropped
/// while its source page was erased.
#[test]
fn a_legacy_blob_survives_defragmentation() {
    let mut flash = partition_with_legacy_blob(3);
    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    // Often enough that even the oldest page goes through defragmentation, which takes a while, as
    // a page with fewer erased entries only wins once it is old enough.
    for value in 0..6_000u32 {
        nvs.set(&namespace(), &Key::from_str("counter"), value).unwrap();
    }
    assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Ok(VALUE.to_vec()));
    drop(nvs);

    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Ok(VALUE.to_vec()));
}

#[test]
fn a_legacy_blob_is_deleted() {
    let mut flash = partition_with_legacy_blob(3);
    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        nvs.delete(&namespace(), &key()).unwrap();
        assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Err(Error::KeyNotFound));
    }
    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Err(Error::KeyNotFound));
    assert_eq!(nvs.get::<u8>(&namespace(), &Key::from_str("other")), Ok(7));
}

/// Overwriting a legacy blob with a blob replaces it by a multi-page one, the only kind written.
#[test]
fn a_legacy_blob_is_overwritten_by_a_blob() {
    let mut flash = partition_with_legacy_blob(3);
    let value = vec![0x5Au8; 5000];
    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        nvs.set(&namespace(), &key(), value.as_slice()).unwrap();
        assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Ok(value.clone()));
    }
    assert_eq!(
        find_header(&flash.buf, ItemType::Blob, &key()).map(|offset| {
            let page = offset / FLASH_SECTOR_SIZE * FLASH_SECTOR_SIZE;
            common::entry_state(&flash.buf, page, (offset - page - common::ITEM_OFFSET) / ITEM_SIZE)
        }),
        Some(0b00),
        "the legacy item is erased"
    );

    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()), Ok(value));
    let entries: Vec<_> = nvs.typed_entries().collect::<Result<_, _>>().unwrap();
    assert!(entries.contains(&(namespace(), key(), ItemType::BlobIndex)));
    assert!(!entries.contains(&(namespace(), key(), ItemType::Blob)));
}

#[test]
fn a_legacy_blob_is_overwritten_by_a_primitive_or_a_string() {
    for overwrite in [
        |nvs: &mut esp_nvs::Nvs<&mut common::Flash>| nvs.set(&namespace(), &key(), 42u32),
        |nvs: &mut esp_nvs::Nvs<&mut common::Flash>| nvs.set(&namespace(), &key(), "a string"),
    ] {
        let mut flash = partition_with_legacy_blob(3);
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            overwrite(&mut nvs).unwrap();
        }
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key()).ok(), None);
        assert!(
            nvs.get::<u32>(&namespace(), &key()) == Ok(42)
                || nvs.get::<String>(&namespace(), &key()).as_deref() == Ok("a string")
        );
        assert_eq!(nvs.get::<u8>(&namespace(), &Key::from_str("other")), Ok(7));
    }
}

/// A legacy blob whose entries were written but not yet marked in the entry map is recovered like a
/// string, header and payload. It used to be counted as a single entry with its payload left
/// uncounted, so the next write landed on top of it.
#[test]
fn a_legacy_blob_not_yet_marked_written_is_recovered() {
    let mut flash = partition_with_legacy_blob(3);
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

/// Overwriting a legacy blob with a blob, cutting the power at every flash operation. After a
/// reboot the key holds either value, never neither, and a further write sticks.
#[test]
fn power_loss_while_overwriting_a_legacy_blob() {
    let image = partition_with_legacy_blob(4).buf;
    let value = vec![0x5Au8; 5000];
    for budget in 0.. {
        let mut flash = common::Flash::new(4);
        flash.buf = image.clone();
        flash.arm_fault(budget);
        let written = match esp_nvs::Nvs::new(0, flash.len(), &mut flash) {
            Ok(mut nvs) => nvs.set(&namespace(), &key(), value.as_slice()),
            Err(e) => Err(e),
        };
        flash.disable_faults();

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        let read = nvs.get::<Vec<u8>>(&namespace(), &key());
        assert!(
            read == Ok(VALUE.to_vec()) || read == Ok(value.clone()),
            "power lost at operation {budget}: {:?}",
            read.map(|v| v.len())
        );
        nvs.set(&namespace(), &key(), [1u8; 10].as_slice()).unwrap();
        drop(nvs);
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&namespace(), &key()),
            Ok(vec![1u8; 10]),
            "power lost at operation {budget}"
        );

        if written.is_ok() {
            break;
        }
    }
}
