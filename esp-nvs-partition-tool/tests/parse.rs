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

/// CSV files written for ESP-IDF's generator have to parse here too. These all failed: comment
/// lines, namespace rows without the trailing empty fields, spaces around type and encoding, an
/// encoding in upper case, and spaces around a number.
#[test]
fn test_csv_accepted_by_esp_idf() {
    let csv = "\
# settings of the device
key,type,encoding,value
config,namespace
# the version
version, data , U8 , 5
name,data,string, spaced out
";
    let partition = NvsPartition::try_from_str(csv).unwrap();
    assert_eq!(partition.entries.len(), 2);
    assert_eq!(partition.entries[0].namespace, "config");
    assert_eq!(partition.entries[0].key, "version");
    assert_eq!(partition.entries[0].content, EntryContent::Data(DataValue::U8(5)));
    // String values are taken as written.
    assert_eq!(
        partition.entries[1].content,
        EntryContent::Data(DataValue::String(" spaced out".to_string()))
    );
}
