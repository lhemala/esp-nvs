use esp_nvs_partition_tool::{
    DataValue,
    EntryContent,
    NvsPartition,
};

#[test]
fn test_hex2bin_encoding() {
    let csv_path = "tests/assets/hex2bin_test.csv";

    let partition = NvsPartition::from_csv_file(csv_path).unwrap();
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
    let csv_path = "tests/assets/invalid_long_key.csv";

    let result = NvsPartition::from_csv_file(csv_path);
    assert!(result.is_err());
}
