use std::fs;

use esp_nvs_partition_tool::{
    DataValue,
    EntryContent,
    NvsEntry,
    NvsPartition,
};
use tempfile::NamedTempFile;

/// Perform a full CSV -> binary -> parse -> CSV -> parse -> binary roundtrip.
///
/// The two parsed partitions are compared entry-by-entry (data equality),
/// and binary1 is compared to a fresh generation from parsed1 to ensure
/// the CSV serialization is lossless.
fn roundtrip_csv_file(csv_path: &str, partition_size: usize) {
    let original_partition = NvsPartition::from_csv_file(csv_path).unwrap();

    let binary1 = original_partition
        .generate_partition(partition_size)
        .unwrap();
    let parsed1 = NvsPartition::parse_partition(&binary1).unwrap();

    assert_eq!(
        original_partition.entries.len(),
        parsed1.entries.len(),
        "entry count mismatch after first parse"
    );

    let csv2 = parsed1.clone().to_csv().unwrap();
    let parsed2 = NvsPartition::from_csv(&csv2).unwrap();

    assert_eq!(
        parsed1.entries.len(),
        parsed2.entries.len(),
        "entry count mismatch after CSV roundtrip"
    );
    for (i, (e1, e2)) in parsed1
        .entries
        .iter()
        .zip(parsed2.entries.iter())
        .enumerate()
    {
        assert_eq!(
            e1.namespace, e2.namespace,
            "namespace mismatch at entry {i}"
        );
        assert_eq!(e1.key, e2.key, "key mismatch at entry {i}");
        assert_eq!(
            e1.content, e2.content,
            "content mismatch at entry {i} (key '{}')",
            e1.key
        );
    }

    let bin2 = parsed2.generate_partition(partition_size).unwrap();
    let parsed3 = NvsPartition::parse_partition(&bin2).unwrap();

    assert_eq!(
        parsed2.entries.len(),
        parsed3.entries.len(),
        "entry count mismatch after second binary generation"
    );

    for (i, (e2, e3)) in parsed2
        .entries
        .iter()
        .zip(parsed3.entries.iter())
        .enumerate()
    {
        assert_eq!(
            e2.content, e3.content,
            "content mismatch at entry {i} (key '{}') after second roundtrip",
            e2.key
        );
    }
}

// Test cases derived from ESP-IDF test_nvs_gen_check.py setup functions
// https://github.com/espressif/esp-idf/blob/v5.5.3/components/nvs_flash/nvs_partition_tool/test_nvs_gen_check.py

/// Roundtrip test for the `setup_ok_primitive` case: a single namespace with
/// primitive integer types (i32, u32, i8).
#[test]
fn test_roundtrip_primitive() {
    roundtrip_csv_file("tests/assets/roundtrip_basic.csv", 0x4000);

    let partition = NvsPartition::from_csv_file("tests/assets/roundtrip_basic.csv").unwrap();
    assert_eq!(partition.entries.len(), 3);
    assert_eq!(partition.entries[0].namespace, "storage");
    assert_entry_content(&partition, 0, &EntryContent::Data(DataValue::I32(42)));
    assert_entry_content(&partition, 1, &EntryContent::Data(DataValue::U32(42)));
    assert_entry_content(&partition, 2, &EntryContent::Data(DataValue::I8(100)));
}

/// Roundtrip test for the `setup_ok_variable_len` case: variable-length
/// entries including short/long strings and single/multi-page blobs.
#[test]
fn test_roundtrip_variable_len() {
    roundtrip_csv_file("tests/assets/large_string.csv", 0x5000);

    let partition = NvsPartition::from_csv_file("tests/assets/large_string.csv").unwrap();
    assert_eq!(partition.entries.len(), 5);

    let bin = partition.generate_partition(0x5000).unwrap();
    let parsed = NvsPartition::parse_partition(&bin).unwrap();
    assert_eq!(parsed.entries.len(), 5);

    assert_entry_content(
        &parsed,
        0,
        &EntryContent::Data(DataValue::String("Hello world!".to_string())),
    );

    let blob_data = fs::read("tests/assets/sample_blob.bin").unwrap();
    assert_entry_content(
        &parsed,
        1,
        &EntryContent::Data(DataValue::Binary(blob_data)),
    );

    let lorem_string = fs::read_to_string("tests/assets/lorem_string.txt").unwrap();
    assert_entry_content(
        &parsed,
        2,
        &EntryContent::Data(DataValue::String(lorem_string)),
    );

    assert_entry_content(
        &parsed,
        3,
        &EntryContent::Data(DataValue::String("I am unique!".to_string())),
    );

    let multi_blob_data = fs::read("tests/assets/sample_multipage_blob.bin").unwrap();
    assert_entry_content(
        &parsed,
        4,
        &EntryContent::Data(DataValue::Binary(multi_blob_data)),
    );
}

/// Roundtrip test for the `setup_ok_mixed` case: three namespaces (storage,
/// etc, abcd) each with 20 cycling primitive entries plus strings and blobs.
#[test]
fn test_roundtrip_mixed() {
    roundtrip_csv_file("tests/assets/multiple_namespaces.csv", 0x6000);

    // Verify parsed structure matches the original test data
    let partition = NvsPartition::from_csv_file("tests/assets/multiple_namespaces.csv").unwrap();

    // storage: 20 primitives + 1 blob = 21
    // etc:     20 primitives + 1 string = 21
    // abcd:    20 primitives + 1 string + 1 blob = 22
    assert_eq!(partition.entries.len(), 64);

    let bin = partition.generate_partition(0x6000).unwrap();
    let parsed = NvsPartition::parse_partition(&bin).unwrap();
    assert_eq!(parsed.entries.len(), 64);

    // Verify primitive cycling pattern [i8, u8, i16, u16, i32, u32] for
    // each namespace
    let prim_types = ["i8", "u8", "i16", "u16", "i32", "u32"];
    let namespaces = ["storage", "etc", "abcd"];

    let mut idx = 0;
    for ns in &namespaces {
        for i in 0..20_usize {
            let entry = &parsed.entries[idx];
            assert_eq!(entry.namespace, *ns);
            assert_eq!(entry.key, format!("test_{i}"));
            let expected = match prim_types[i % 6] {
                "i8" => DataValue::I8(i as i8),
                "u8" => DataValue::U8(i as u8),
                "i16" => DataValue::I16(i as i16),
                "u16" => DataValue::U16(i as u16),
                "i32" => DataValue::I32(i as i32),
                "u32" => DataValue::U32(i as u32),
                _ => unreachable!(),
            };
            assert_entry_content(&parsed, idx, &EntryContent::Data(expected));
            idx += 1;
        }

        // Each namespace has additional entries after the 20 primitives
        match *ns {
            "storage" => {
                // blob_key -> sample_singlepage_blob.bin
                let blob = fs::read("tests/assets/sample_singlepage_blob.bin").unwrap();
                assert_entry_content(&parsed, idx, &EntryContent::Data(DataValue::Binary(blob)));
                idx += 1;
            }
            "etc" => {
                // lorem_str_key -> lorem_string.txt
                let lorem = fs::read_to_string("tests/assets/lorem_string.txt").unwrap();
                assert_entry_content(&parsed, idx, &EntryContent::Data(DataValue::String(lorem)));
                idx += 1;
            }
            "abcd" => {
                // uniq_string_key
                assert_entry_content(
                    &parsed,
                    idx,
                    &EntryContent::Data(DataValue::String("I am unique!".to_string())),
                );
                idx += 1;
                // blob_key -> sample_multipage_blob.bin
                let blob = fs::read("tests/assets/sample_multipage_blob.bin").unwrap();
                assert_entry_content(&parsed, idx, &EntryContent::Data(DataValue::Binary(blob)));
                idx += 1;
            }
            _ => unreachable!(),
        }
    }
}

/// Invalid inputs are properly rejected: non-aligned partition size, bad
/// binary length, and namespace overflow.
#[test]
fn test_validation_errors() {
    // Non-4096-aligned partition size
    let partition = NvsPartition { entries: vec![] };
    let bin_file = NamedTempFile::new().unwrap();
    assert!(
        partition
            .generate_partition_file(bin_file.path(), 5000)
            .is_err(),
        "non-4096-aligned size should be rejected"
    );

    // Binary data whose length isn't a multiple of 4096
    let bad_data = vec![0xFF; 1000];
    assert!(NvsPartition::parse_partition(&bad_data).is_err());

    // Too many namespaces (256 > 255 limit)
    let mut partition = NvsPartition { entries: vec![] };
    for i in 0..256_u16 {
        partition.entries.push(NvsEntry::new_data(
            format!("ns_{i:03}"),
            "val".to_string(),
            DataValue::U8(0),
        ));
    }
    assert!(
        partition.generate_partition(32768).is_err(),
        "256 namespaces should overflow"
    );
}

/// Assert that the entry at `index` has the expected content.
fn assert_entry_content(partition: &NvsPartition, index: usize, expected: &EntryContent) {
    assert_eq!(
        &partition.entries[index].content, expected,
        "entry {} ('{}') content mismatch",
        index, partition.entries[index].key
    );
}
