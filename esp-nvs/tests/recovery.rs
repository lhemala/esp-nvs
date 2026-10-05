//! Recovery from what a power loss or a flash fault leaves behind.
//!
//! Most of these cut the power by failing every flash operation past a budget, then reopen the
//! partition the way a device does on its next boot, and check that it opens, that unrelated values
//! survived, and that it can still be written.

use esp_nvs::Key;
use esp_nvs::error::{
    Error,
    ItemType,
};
use pretty_assertions::assert_eq;

mod common;

fn namespace() -> Key {
    Key::from_str("ns")
}

/// Writes `count` u8 keys `k0..` into a fresh partition of `pages` pages.
fn partition_with_keys(pages: usize, count: u8) -> common::Flash {
    let mut flash = common::Flash::new(pages);
    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    for i in 0..count {
        nvs.set(&namespace(), &Key::from_str(&format!("k{i}")), i).unwrap();
    }
    drop(nvs);
    flash
}

/// Opens the partition with faults disabled, checks the `k0..` keys and writes one more value.
fn reboot_and_check(flash: &mut common::Flash, count: u8) {
    flash.disable_faults();
    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut *flash).unwrap();
    for i in 0..count {
        assert_eq!(nvs.get::<u8>(&namespace(), &Key::from_str(&format!("k{i}"))), Ok(i));
    }
    nvs.set(&namespace(), &Key::from_str("after"), 7u8).unwrap();
    assert_eq!(nvs.get::<u8>(&namespace(), &Key::from_str("after")), Ok(7));
}

/// A string spanning 20 entries, interrupted at every flash operation it takes.
///
/// Cut between its header and its data, the header is CRC valid but the data is not. The scan
/// erased its span and counted it, then visited each payload entry again, found it erased and
/// counted it a second time, so the next free entry ran past the end of the page: the next `set`
/// wrote into the following sector and panicked indexing the entry state bitmap.
#[test]
fn power_loss_while_writing_a_string_leaves_a_writable_partition() {
    let value = "x".repeat(19 * 32 - 1);

    for budget in 0..60 {
        let mut flash = partition_with_keys(3, 99);
        {
            let nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.into_inner().arm_fault(budget);
        }
        if let Ok(mut nvs) = esp_nvs::Nvs::new(0, flash.len(), &mut flash) {
            let _ = nvs.set(&namespace(), &Key::from_str("s"), value.as_str());
        }

        reboot_and_check(&mut flash, 99);
    }
}

/// Power lost after the write that filled the active page but before the page was marked full.
///
/// The page comes back `Active` with every entry used. A `set` into a new namespace then wrote the
/// namespace entry at index 126 and the value at 127, which is the header of the next sector, so
/// the value was gone on the next boot. Had that sector held a page, it would have gone with it.
#[test]
fn a_full_page_left_active_is_not_written_past_its_end() {
    let mut flash = common::Flash::new(4);
    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        // The namespace entry and 125 values fill the first page exactly.
        for i in 0..125u8 {
            nvs.set(&namespace(), &Key::from_str(&format!("k{i}")), i).unwrap();
        }
    }

    // Set the first page's state word back from FULL to ACTIVE, as if marking it had not happened.
    // The state is not covered by the page header CRC.
    flash.buf[..4].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());

    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        nvs.set(&Key::from_str("other"), &Key::from_str("x"), 1u8).unwrap();
    }

    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    for i in 0..125u8 {
        assert_eq!(nvs.get::<u8>(&namespace(), &Key::from_str(&format!("k{i}"))), Ok(i));
    }
    assert_eq!(nvs.get::<u8>(&Key::from_str("other"), &Key::from_str("x")), Ok(1));
}

/// An empty partition is rejected. It was accepted, and the first `set` panicked looking for a free
/// page.
#[test]
fn an_empty_partition_is_rejected() {
    let flash = common::Flash::new(2);
    let result = esp_nvs::Nvs::new(0, 0, flash);
    assert!(matches!(result, Err(esp_nvs::error::Error::InvalidPartitionSize)));
}

/// A single page can be read, as a read-only image is, but has no reserve page to write with.
#[test]
fn a_single_page_partition_reports_flash_full_on_write() {
    let mut flash = common::Flash::new(1);
    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(
        nvs.set(&namespace(), &Key::from_str("a"), 1u8),
        Err(esp_nvs::error::Error::FlashFull)
    );
}

/// A partition reaching past the end of the flash is rejected rather than read out of bounds.
#[test]
fn a_partition_larger_than_the_flash_is_rejected() {
    let flash = common::Flash::new(2);
    let result = esp_nvs::Nvs::new(0, 3 * esp_nvs::FLASH_SECTOR_SIZE, flash);
    assert!(matches!(result, Err(esp_nvs::error::Error::InvalidPartitionSize)));

    let flash = common::Flash::new(3);
    let result = esp_nvs::Nvs::new(2 * esp_nvs::FLASH_SECTOR_SIZE, 2 * esp_nvs::FLASH_SECTOR_SIZE, flash);
    assert!(matches!(result, Err(esp_nvs::error::Error::InvalidPartitionSize)));
}

/// A partition with every page in use has no reserve page. `set` panicked unwrapping a free page
/// instead of reporting that the partition is full.
#[test]
fn a_partition_without_a_free_page_reports_flash_full() {
    let mut flash = common::Flash::new(2);
    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        nvs.set(&namespace(), &Key::from_str("a"), 1u8).unwrap();
    }
    // Give the second page a valid header too, as an image filling every page would.
    let header: Vec<u8> = flash.buf[..32].to_vec();
    flash.buf[esp_nvs::FLASH_SECTOR_SIZE..esp_nvs::FLASH_SECTOR_SIZE + 32].copy_from_slice(&header);
    // FULL for both, so neither is the active page.
    flash.buf[..4].copy_from_slice(&0xFFFF_FFFCu32.to_le_bytes());
    let second = esp_nvs::FLASH_SECTOR_SIZE;
    flash.buf[second..second + 4].copy_from_slice(&0xFFFF_FFFCu32.to_le_bytes());

    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(nvs.get::<u8>(&namespace(), &Key::from_str("a")), Ok(1));
    assert_eq!(
        nvs.set(&namespace(), &Key::from_str("b"), 2u8),
        Err(esp_nvs::error::Error::FlashFull)
    );
}

fn small_blob() -> Vec<u8> {
    (0..1500u32).map(|i| (i * 7) as u8).collect()
}

fn large_blob() -> Vec<u8> {
    (0..6000u32).map(|i| (i * 13 + 1) as u8).collect()
}

/// A partition holding blobs on several pages, a string and a few primitives, with some erased
/// entries so defragmentation has something to reclaim.
fn partition_with_values(pages: usize) -> Vec<u8> {
    let flash = common::SharedFlash::new(pages);
    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap();
    nvs.set(&namespace(), &Key::from_str("blob"), small_blob().as_slice())
        .unwrap();
    for i in 0..20u8 {
        nvs.set(&namespace(), &Key::from_str(&format!("j{i}")), i).unwrap();
    }
    for i in 0..20u8 {
        nvs.delete(&namespace(), &Key::from_str(&format!("j{i}"))).unwrap();
    }
    nvs.set(&namespace(), &Key::from_str("str"), "a string that has to survive")
        .unwrap();
    for i in 0..5u8 {
        nvs.set(&namespace(), &Key::from_str(&format!("p{i}")), i).unwrap();
    }
    nvs.set(&namespace(), &Key::from_str("big"), large_blob().as_slice())
        .unwrap();
    for value in 0..60u32 {
        nvs.set(&namespace(), &Key::from_str("counter"), value).unwrap();
    }
    drop(nvs);
    flash.snapshot()
}

fn check_values(nvs: &mut esp_nvs::Nvs<common::SharedFlash>, context: &str) {
    assert!(
        nvs.get::<Vec<u8>>(&namespace(), &Key::from_str("blob")) == Ok(small_blob()),
        "blob {context}"
    );
    assert!(
        nvs.get::<Vec<u8>>(&namespace(), &Key::from_str("big")) == Ok(large_blob()),
        "big {context}"
    );
    assert_eq!(
        nvs.get::<String>(&namespace(), &Key::from_str("str")).as_deref(),
        Ok("a string that has to survive"),
        "str {context}"
    );
    for i in 0..5u8 {
        assert_eq!(
            nvs.get::<u8>(&namespace(), &Key::from_str(&format!("p{i}"))),
            Ok(i),
            "p{i} {context}"
        );
    }
}

/// Cuts the power once, at every flash operation in turn, while a counter is overwritten often
/// enough to take every page through defragmentation, then boots twice and checks that nothing
/// else was lost and that the partition can still be written.
///
/// This found that an interrupted defragmentation could fail `Nvs::new` on every boot (the resumed
/// copy no longer fitting its partly used target, or no page left to resume into), copy into an
/// erased page that never got a header, and delete intact blobs at boot because the chunks on the
/// source and on its partial copy were both counted against the blob's index.
fn power_loss_sweep(pages: usize) {
    let image = partition_with_values(pages);
    let counter = Key::from_str("counter");

    // A counter overwrite takes about three operations, so this covers several defragmentations.
    for budget in 0..1_200 {
        let flash = common::SharedFlash::from_buf(image.clone());
        let mut last_written = None;
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap();
            flash.arm_fault(budget);
            for value in 0..500u32 {
                if nvs.set(&namespace(), &counter, value).is_err() {
                    break;
                }
                last_written = Some(value);
            }
        }
        flash.disable_faults();

        let context = format!("after a power loss at operation {budget}");
        let mut nvs =
            esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap_or_else(|e| panic!("Nvs::new {context}: {e:?}"));
        check_values(&mut nvs, &context);
        if let Some(value) = last_written {
            // The write the power went out on may or may not have made it.
            let read = nvs.get::<u32>(&namespace(), &counter);
            assert!(read == Ok(value) || read == Ok(value + 1), "counter {read:?} {context}");
        }
        for value in 0..150u32 {
            nvs.set(&namespace(), &counter, value)
                .unwrap_or_else(|e| panic!("set {e:?} {context}"));
        }
        drop(nvs);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap();
        check_values(&mut nvs, &format!("{context}, one session later"));
    }
}

#[test]
fn power_loss_during_defragmentation_of_three_pages() {
    power_loss_sweep(3);
}

#[test]
fn power_loss_during_defragmentation_of_four_pages() {
    power_loss_sweep(4);
}

/// An interrupted defragmentation with no copy target yet, where the only free pages are corrupt.
///
/// Resuming erased a free page but never initialized it, copied the items into the headerless page
/// and erased the source. On the next boot the page read as blank and its items were gone.
#[test]
fn a_resumed_defragmentation_copies_into_an_initialized_page() {
    let flash = common::SharedFlash::new(3);
    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap();
        for i in 0..10u8 {
            nvs.set(&namespace(), &Key::from_str(&format!("k{i}")), i).unwrap();
        }
    }
    flash.with_buf(|buf| {
        let sector = esp_nvs::FLASH_SECTOR_SIZE;
        // ACTIVE to FREEING. The state is not covered by the page header CRC.
        buf[..4].copy_from_slice(&0xFFFF_FFF8u32.to_le_bytes());
        // The other two pages carry a state but no CRC, as an interrupted header write leaves them.
        for page in 1..3 {
            buf[page * sector..page * sector + 4].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
        }
    });

    for _boot in 0..2 {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap();
        for i in 0..10u8 {
            assert_eq!(nvs.get::<u8>(&namespace(), &Key::from_str(&format!("k{i}"))), Ok(i));
        }
    }
}

/// Power lost part way through programming an item header, before the entry map was updated.
///
/// The entry is EMPTY in the map but no longer blank, and the scan skipped it without counting it,
/// so the next write was programmed on top of the leftovers. That write reported success but its
/// CRC was broken: the new value could not be read, and the old value it replaced was erased.
#[test]
fn a_torn_entry_is_not_written_over() {
    let mut flash = common::Flash::new(3);
    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        nvs.set(&namespace(), &Key::from_str("a"), 1u32).unwrap();
    }

    // Entry 0 holds the namespace, entry 1 `a`, so entry 2 is where the next item goes.
    let entry = common::ITEM_OFFSET + 2 * esp_nvs::ITEM_SIZE;
    for (i, byte) in flash.buf[entry..entry + 12].iter_mut().enumerate() {
        *byte = (i as u8).wrapping_mul(37);
    }

    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        nvs.set(&namespace(), &Key::from_str("a"), 2u32).unwrap();
        nvs.set(&namespace(), &Key::from_str("b"), 3u32).unwrap();
        assert_eq!(nvs.get::<u32>(&namespace(), &Key::from_str("a")), Ok(2));
        assert_eq!(nvs.get::<u32>(&namespace(), &Key::from_str("b")), Ok(3));
    }

    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(nvs.get::<u32>(&namespace(), &Key::from_str("a")), Ok(2));
    assert_eq!(nvs.get::<u32>(&namespace(), &Key::from_str("b")), Ok(3));
}

/// A sector whose header is blank but whose body is not, as an interrupted erase leaves it.
///
/// It was taken for a clean page and initialized without an erase, so the items written to it were
/// programmed over the old bytes and could not be read back after the next boot.
#[test]
fn a_page_with_a_blank_header_and_leftover_data_is_erased_before_use() {
    let mut flash = common::Flash::new(3);
    for page in 0..3 {
        let body = page * esp_nvs::FLASH_SECTOR_SIZE + common::ITEM_OFFSET;
        flash.buf[body..body + 8].fill(0x5A);
    }

    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        nvs.set(&namespace(), &Key::from_str("a"), 42u32).unwrap();
    }

    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(nvs.get::<u32>(&namespace(), &Key::from_str("a")), Ok(42));
}

/// Sets the entry map state of every entry the item at `entry` spans back to WRITTEN.
fn unerase(buf: &mut [u8], page: usize, entry: usize) {
    let span = buf[page + common::ITEM_OFFSET + entry * esp_nvs::ITEM_SIZE + 2] as usize;
    for e in entry..entry + span {
        let byte = page + common::ENTRY_STATE_MAP_OFFSET + e / 4;
        let shift = (e % 4) * 2;
        buf[byte] = (buf[byte] & !(0b11 << shift)) | (common::ENTRY_STATE_WRITTEN << shift);
    }
}

/// Finds the item headers on flash with the given type byte and key, as (page start, entry).
fn find_headers(buf: &[u8], type_: u8, key: &Key) -> Vec<(usize, usize)> {
    let mut found = vec![];
    for page in (0..buf.len()).step_by(esp_nvs::FLASH_SECTOR_SIZE) {
        for entry in 0..esp_nvs::ENTRIES_PER_PAGE {
            let offset = page + common::ITEM_OFFSET + entry * esp_nvs::ITEM_SIZE;
            let candidate = &buf[offset..offset + esp_nvs::ITEM_SIZE];
            if candidate[1] == type_ && candidate[8..24] == key.as_bytes()[..] && common::is_item_header(buf, offset) {
                found.push((page, entry));
            }
        }
    }
    found
}

/// Power lost after a blob was overwritten but before its old version was erased, with the newer
/// version on a page at a lower flash address than the older one.
///
/// The boot cleanup decided correctly which version was older, then deleted "the" blob index of the
/// key, which is the first one found in page order - here the newer one. The blob rolled back.
#[test]
fn of_two_blob_versions_the_newer_one_is_kept() {
    let key = Key::from_str("b");
    let flash = common::SharedFlash::new(4);
    {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap();
        nvs.set(&namespace(), &key, [1u8; 100].as_slice()).unwrap();
        // Move on to the next page, write the new version there, and fill that page as well.
        for value in 0..130u32 {
            nvs.set(&namespace(), &Key::from_str("c"), value).unwrap();
        }
        nvs.set(&namespace(), &key, [2u8; 100].as_slice()).unwrap();
        for value in 0..130u32 {
            nvs.set(&namespace(), &Key::from_str("d"), value).unwrap();
        }
    }

    flash.with_buf(|buf| {
        // Bring the old version's index and chunk back, as if they had never been erased.
        let old_page = find_headers(buf, ItemType::BlobIndex as u8, &key)[0].0;
        for (page, entry) in find_headers(buf, ItemType::BlobIndex as u8, &key)
            .into_iter()
            .chain(find_headers(buf, ItemType::BlobData as u8, &key))
        {
            if page == old_page {
                unerase(buf, page, entry);
            }
        }

        // Swap the two pages in flash, so the newer one comes first by address.
        let sector = esp_nvs::FLASH_SECTOR_SIZE;
        let new_page = old_page + sector;
        let (low, high) = buf.split_at_mut(new_page);
        low[old_page..old_page + sector].swap_with_slice(&mut high[..sector]);
    });

    for _boot in 0..2 {
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap();
        assert_eq!(nvs.get::<Vec<u8>>(&namespace(), &key), Ok(vec![2u8; 100]));
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Primitive(u32),
    Str(String),
    Blob(Vec<u8>),
}

impl Value {
    fn set<T: esp_nvs::platform::Platform>(&self, nvs: &mut esp_nvs::Nvs<T>, key: &Key) -> Result<(), Error> {
        match self {
            Value::Primitive(value) => nvs.set(&namespace(), key, *value),
            Value::Str(value) => nvs.set(&namespace(), key, value.as_str()),
            Value::Blob(value) => nvs.set(&namespace(), key, value.as_slice()),
        }
    }

    /// Whether `key` currently reads back as this value.
    fn is_stored<T: esp_nvs::platform::Platform>(&self, nvs: &mut esp_nvs::Nvs<T>, key: &Key) -> bool {
        match self {
            Value::Primitive(value) => nvs.get::<u32>(&namespace(), key) == Ok(*value),
            Value::Str(value) => nvs.get::<String>(&namespace(), key).as_ref() == Ok(value),
            Value::Blob(value) => nvs.get::<Vec<u8>>(&namespace(), key).as_ref() == Ok(value),
        }
    }

    fn describe(&self) -> String {
        match self {
            Value::Primitive(value) => format!("u32 {value}"),
            Value::Str(value) => format!("a string of {} bytes", value.len()),
            Value::Blob(value) => format!("a blob of {} bytes", value.len()),
        }
    }

    /// Another value of the same type.
    fn another(&self) -> Value {
        match self {
            Value::Primitive(_) => Value::Primitive(33),
            Value::Str(_) => Value::Str("c".repeat(20)),
            Value::Blob(_) => Value::Blob(vec![3u8; 200]),
        }
    }
}

/// Overwrites a key with a value of each type in turn, cutting the power at every flash operation
/// of the overwrite. After a reboot the key has to read as either the old or the new value, and
/// writing it once more has to stick.
///
/// Changing to or from a blob left both items on flash when the power went before the old one was
/// erased, and nothing at boot compared a blob index with a primitive or a string. Reads kept
/// returning the old value, and the next overwrite resolved the key against the wrong item: it
/// reported success and was gone after the following reboot, or read back stale until then.
#[test]
fn power_loss_while_changing_the_type_of_a_value() {
    let values = [
        Value::Primitive(1),
        Value::Str("a".repeat(40)),
        Value::Blob(vec![1u8; 5000]),
    ];
    let replacements = [
        Value::Primitive(2),
        Value::Str("b".repeat(70)),
        Value::Blob(vec![2u8; 300]),
    ];
    let key = Key::from_str("k");
    let other = Key::from_str("other");

    for old in &values {
        for new in &replacements {
            for budget in 0.. {
                let mut flash = common::Flash::new(4);
                {
                    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
                    nvs.set(&namespace(), &other, 7u32).unwrap();
                    old.set(&mut nvs, &key).unwrap();
                }
                flash.arm_fault(budget);
                let written = match esp_nvs::Nvs::new(0, flash.len(), &mut flash) {
                    Ok(mut nvs) => new.set(&mut nvs, &key),
                    Err(e) => Err(e),
                };
                flash.disable_faults();

                let context = format!(
                    "{} to {}, power lost at operation {budget}",
                    old.describe(),
                    new.describe()
                );
                let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
                assert!(
                    old.is_stored(&mut nvs, &key) != new.is_stored(&mut nvs, &key),
                    "{context}"
                );
                assert_eq!(nvs.get::<u32>(&namespace(), &other), Ok(7), "{context}");

                let next = new.another();
                next.set(&mut nvs, &key).unwrap();
                assert!(next.is_stored(&mut nvs, &key), "{context}, then overwritten");
                drop(nvs);
                let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
                assert!(
                    next.is_stored(&mut nvs, &key),
                    "{context}, then overwritten and rebooted"
                );

                if written.is_ok() {
                    break;
                }
            }
        }
    }
}

/// A write that fails must not take the active page out of the instance.
///
/// `set` pops the active page out of the instance's page list to write to it, and several error
/// paths returned without putting it back. Everything on it then read as missing until a reboot.
#[test]
fn a_failed_write_keeps_the_values_it_did_not_touch() {
    let writes: [(&str, fn(&mut esp_nvs::Nvs<common::SharedFlash>) -> Result<(), Error>); 4] = [
        ("u8", |nvs| nvs.set(&namespace(), &Key::from_str("b"), 2u8)),
        ("blob", |nvs| {
            nvs.set(&namespace(), &Key::from_str("b"), [1u8; 10].as_slice())
        }),
        ("blob in a new namespace", |nvs| {
            nvs.set(&Key::from_str("other"), &Key::from_str("b"), [1u8; 10].as_slice())
        }),
        ("string", |nvs| nvs.set(&namespace(), &Key::from_str("b"), "x")),
    ];

    for (what, write) in writes {
        for budget in 0..4 {
            let flash = common::SharedFlash::new(4);
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap();
            nvs.set(&namespace(), &Key::from_str("a"), 1u8).unwrap();

            flash.arm_fault(budget);
            let _ = write(&mut nvs);
            flash.disable_faults();

            assert_eq!(
                nvs.get::<u8>(&namespace(), &Key::from_str("a")),
                Ok(1),
                "after a {what} write failing at operation {budget}"
            );
        }
    }
}
