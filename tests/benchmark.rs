use crate::common::Operation::{Read, Write};
use crate::common::{
    ENTRY_STATE_MAP_ENTRY_SIZE, ENTRY_STATE_MAP_OFFSET, FLASH_SECTOR_SIZE, ITEM_OFFSET, ITEM_SIZE,
    PAGE_HEADER_SIZE,
};
use esp_nvs::Key;

mod common;

#[test]
fn single_primitve() {
    let mut flash = common::Flash::new(2);

    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    nvs.set(&Key::from_str("ns1"), &Key::from_str("value"), 0xAAu8)
        .unwrap();

    let ops_init = vec![
        Read {
            offset: 0,
            len: FLASH_SECTOR_SIZE,
        },
        Read {
            offset: FLASH_SECTOR_SIZE as _,
            len: FLASH_SECTOR_SIZE,
        },
    ];

    let ops_write = vec![
        Write {
            offset: 0,
            len: PAGE_HEADER_SIZE,
        },
        Write {
            offset: ITEM_OFFSET as _,
            len: ITEM_SIZE,
        },
        Write {
            offset: ENTRY_STATE_MAP_OFFSET as _,
            len: ENTRY_STATE_MAP_ENTRY_SIZE,
        },
        Write {
            offset: (ITEM_OFFSET + 1 * ITEM_SIZE) as _,
            len: ITEM_SIZE,
        },
        Write {
            offset: ENTRY_STATE_MAP_OFFSET as _,
            len: ENTRY_STATE_MAP_ENTRY_SIZE,
        },
    ];

    let mut ops = ops_init.clone();
    ops.extend(ops_write);

    assert_eq!(flash.operations, ops);
    flash.operations.clear();

    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    assert_eq!(
        nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("value"))
            .unwrap(),
        0xAA
    );

    // namespace is already cached, so only reading the actual value is required
    let ops_read = vec![Read {
        offset: (ITEM_OFFSET + 1 * ITEM_SIZE) as _,
        len: ITEM_SIZE,
    }];
    let mut ops = ops_init.clone();
    ops.extend(ops_read);

    assert_eq!(flash.operations, ops);
}
