use std::fs;

use esp_nvs_partition_tool::{
    DataValue,
    EntryContent,
    NvsPartition,
};

mod common;

#[test]
fn test_hex2bin_encoding() {
    let partition = common::read_csv_file("tests/assets/hex2bin_test.csv");
    assert_eq!(partition.entries.len(), 1);

    match &partition.entries[0].content {
        EntryContent::Data(DataValue::Binary(data)) => {
            assert_eq!(data.len(), 16);
            assert_eq!(data[0], 0x00);
            assert_eq!(data[1], 0x11);
            assert_eq!(data[15], 0xFF);
        }
        _ => panic!("Expected binary data"),
    }
}

#[test]
fn test_key_length_validation() {
    let content = fs::read_to_string("tests/assets/invalid_long_key.csv").unwrap();

    let result = NvsPartition::try_from_str(&content);
    assert!(result.is_err());
}
