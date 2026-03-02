use std::fs;

use esp_nvs_partition_tool::{
    DataValue,
    NvsEntry,
    NvsPartition,
};
use tempfile::NamedTempFile;

#[test]
fn test_csv_to_binary() {
    let csv_path = "tests/assets/roundtrip_basic.csv";

    let partition = NvsPartition::from_csv_file(csv_path).unwrap();
    assert_eq!(partition.entries.len(), 3);
    assert_eq!(partition.entries[0].namespace, "test_namespace");
    assert_eq!(partition.entries[0].key, "u8_val");

    let bin_file = NamedTempFile::new().unwrap();
    partition
        .generate_partition_file(bin_file.path(), 16384)
        .unwrap();

    let metadata = fs::metadata(bin_file.path()).unwrap();
    assert_eq!(metadata.len(), 16384);
}

#[test]
fn test_generate_from_api() {
    let mut partition = NvsPartition { entries: vec![] };

    partition.entries.push(NvsEntry::new_data(
        "config".to_string(),
        "version".to_string(),
        DataValue::U8(1),
    ));
    partition.entries.push(NvsEntry::new_data(
        "config".to_string(),
        "count".to_string(),
        DataValue::U32(12345),
    ));
    partition.entries.push(NvsEntry::new_data(
        "config".to_string(),
        "name".to_string(),
        DataValue::String("Test Device".to_string()),
    ));

    let bin_file = NamedTempFile::new().unwrap();
    let result = partition.generate_partition_file(bin_file.path(), 8192);
    assert!(result.is_ok());

    let metadata = fs::metadata(bin_file.path()).unwrap();
    assert_eq!(metadata.len(), 8192);
}

#[test]
fn test_multiple_namespaces() {
    let csv_path = "tests/assets/multiple_namespaces.csv";

    let partition = NvsPartition::from_csv_file(csv_path).unwrap();
    assert_eq!(partition.entries.len(), 3);

    let bin_file = NamedTempFile::new().unwrap();
    let result = partition.generate_partition_file(bin_file.path(), 16384);
    assert!(result.is_ok());
}

#[test]
fn test_large_string() {
    let csv_path = "tests/assets/large_string.csv";

    let partition = NvsPartition::from_csv_file(csv_path).unwrap();

    let bin_file = NamedTempFile::new().unwrap();
    let result = partition.generate_partition_file(bin_file.path(), 16384);
    assert!(result.is_ok());
}

#[test]
fn test_invalid_partition_size() {
    let mut partition = NvsPartition { entries: vec![] };
    partition.entries.push(NvsEntry::new_data(
        "test".to_string(),
        "dummy".to_string(),
        DataValue::U8(0),
    ));

    let bin_file = NamedTempFile::new().unwrap();

    let result = partition.generate_partition_file(bin_file.path(), 1024);
    assert!(result.is_err());
}
