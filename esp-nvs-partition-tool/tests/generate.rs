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

/// Keys and namespaces built through the API are checked like those from a CSV file. A key that is
/// too long or not ASCII made `generate_partition` panic, and one with a NUL byte was cut short.
#[test]
fn test_generate_rejects_invalid_keys() {
    for (namespace, key) in [
        ("config", "a_key_that_is_too_long"),
        ("config", "ééééééé"),
        ("config", "a\0b"),
        ("config", ""),
        ("", "key"),
        ("a_namespace_too_long", "key"),
    ] {
        let partition = NvsPartition {
            entries: vec![NvsEntry::new_data(
                namespace.to_string(),
                key.to_string(),
                DataValue::U8(1),
            )],
        };
        assert!(
            matches!(
                partition.generate_partition(0x3000),
                Err(esp_nvs_partition_tool::Error::InvalidKey(_))
            ),
            "{namespace:?}/{key:?}"
        );
    }
}

/// The CSV parser rejects a key that is not ASCII instead of handing it on to panic later.
#[test]
fn test_csv_rejects_non_ascii_keys() {
    let csv = "key,type,encoding,value\nns,namespace,,\nééééééa,data,u8,1\n";
    assert!(matches!(
        NvsPartition::try_from_str(csv),
        Err(esp_nvs_partition_tool::Error::InvalidKey(_))
    ));
}

/// ESP-IDF reserves namespace index 255 to match any namespace, so a partition holds at most 254.
/// The 255th was accepted and given that index.
#[test]
fn test_generate_rejects_a_255th_namespace() {
    let entries = (0..255)
        .map(|i| NvsEntry::new_data(format!("n{i}"), "k".to_string(), DataValue::U8(1)))
        .collect();
    let partition = NvsPartition { entries };
    assert!(matches!(
        partition.generate_partition(0x6000),
        Err(esp_nvs_partition_tool::Error::TooManyNamespaces)
    ));
}

/// A single page cannot be written to, as the library keeps one page in reserve. That used to fail
/// with an opaque `FlashFull` from the library.
#[test]
fn test_generate_rejects_a_single_page() {
    let partition = NvsPartition {
        entries: vec![NvsEntry::new_data("ns".to_string(), "k".to_string(), DataValue::U8(1))],
    };
    assert!(matches!(
        partition.generate_partition(0x1000),
        Err(esp_nvs_partition_tool::Error::PartitionTooSmall(0x1000))
    ));
}

/// A size beyond what the library accepts is rejected before memory for it is allocated, which
/// ran out of memory for a large enough size.
#[test]
fn test_generate_rejects_a_huge_size() {
    let partition = NvsPartition { entries: vec![] };
    let size = 0x100_0000_0000;
    assert!(matches!(
        partition.generate_partition(size),
        Err(esp_nvs_partition_tool::Error::InvalidPartitionSize(s)) if s == size
    ));
}

/// A file entry that cannot be read reports which file it is.
#[test]
fn test_generate_reports_the_path_of_a_missing_file() {
    let partition = NvsPartition {
        entries: vec![NvsEntry::new_file(
            "ns".to_string(),
            "k".to_string(),
            FileEncoding::Binary,
            PathBuf::from("does/not/exist.bin"),
        )],
    };
    let error = partition.generate_partition(0x3000).unwrap_err();
    assert!(error.to_string().contains("does/not/exist.bin"), "{error}");
}
