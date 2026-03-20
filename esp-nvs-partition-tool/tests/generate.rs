use std::path::PathBuf;

use esp_nvs_partition_tool::{
    DataValue,
    EntryContent,
    FileEncoding,
    NvsEntry,
    NvsPartition,
};

mod common;

#[test]
fn test_csv_to_binary() {
    let partition = common::read_csv_file("tests/assets/roundtrip_basic.csv");
    assert_eq!(partition.entries.len(), 3);
    assert_eq!(partition.entries[0].namespace, "storage");
    assert_eq!(partition.entries[0].key, "int32_test");

    let data = partition.generate_partition(16384).unwrap();
    assert_eq!(data.len(), 16384);
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

    let data = partition.generate_partition(8192).unwrap();
    assert_eq!(data.len(), 8192);
}

#[test]
fn test_multiple_namespaces() {
    let partition = common::read_csv_file("tests/assets/multiple_namespaces.csv");
    assert_eq!(partition.entries.len(), 64);

    let result = partition.generate_partition(0x6000);
    assert!(result.is_ok());
}

#[test]
fn test_large_string() {
    let partition = common::read_csv_file("tests/assets/large_string.csv");

    let result = partition.generate_partition(0x5000);
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

    let result = partition.generate_partition(1024);
    assert!(result.is_err());
}

#[test]
fn test_entry_edit_methods() {
    let mut entry = NvsEntry::new_data("ns".into(), "key".into(), DataValue::U8(1));

    entry.set_data(DataValue::U32(100));
    assert!(matches!(entry.content, EntryContent::Data(DataValue::U32(100))));

    entry.set_file(FileEncoding::Binary, PathBuf::from("cert.pem"));
    assert!(matches!(entry.content, EntryContent::File { .. }));

    entry.set_content(EntryContent::Data(DataValue::String("test".into())));
    assert!(matches!(entry.content, EntryContent::Data(DataValue::String(_))));
}

#[test]
fn test_find_mut_and_generate() {
    let mut partition = NvsPartition {
        entries: vec![NvsEntry::new_data("config".into(), "value".into(), DataValue::U32(1))],
    };

    partition.find_mut("value").unwrap().set_data(DataValue::U32(42));
    assert!(partition.find_mut("nonexistent").is_none());

    let parsed = NvsPartition::try_from_bytes(partition.generate_partition(8192).unwrap()).unwrap();
    assert!(matches!(
        parsed.find("value").unwrap().content,
        EntryContent::Data(DataValue::U32(42))
    ));
}
