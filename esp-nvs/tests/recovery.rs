//! Recovery from what a power loss or a flash fault leaves behind.
//!
//! Most of these cut the power by failing every flash operation past a budget, then reopen the
//! partition the way a device does on its next boot, and check that it opens, that unrelated values
//! survived, and that it can still be written.

use esp_nvs::Key;
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
