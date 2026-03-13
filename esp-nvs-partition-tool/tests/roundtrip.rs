use std::fs;

use base64::Engine;
use esp_nvs_partition_tool::{
    DataValue,
    EntryContent,
    NvsEntry,
    NvsPartition,
};
use similar::TextDiff;
use tempfile::NamedTempFile;

macro_rules! entry {
    ($key:expr, $variant:ident, $val:expr) => {
        NvsEntry::new_data(
            "ns".to_string(),
            $key.to_string(),
            DataValue::$variant($val),
        )
    };
}

/// Assert that the entry at `index` has the expected content.
fn assert_entry_content(partition: &NvsPartition, index: usize, expected: &EntryContent) {
    assert_eq!(
        &partition.entries[index].content, expected,
        "entry {} ('{}') content mismatch",
        index, partition.entries[index].key
    );
}

/// Compare two CSV strings and, on mismatch, panic with a unified diff.
fn assert_csv_eq(expected: &str, actual: &str) {
    if expected == actual {
        return;
    }
    let diff = TextDiff::from_lines(expected, actual)
        .unified_diff()
        .header("expected", "actual")
        .to_string();
    panic!("CSV content mismatch:\n{diff}");
}

/// Full end-to-end: CSV → binary → parse → CSV → parse → binary.
/// Verifies binary identity across the complete roundtrip.
#[test]
fn test_csv_binary_csv_roundtrip() {
    let original_partition =
        NvsPartition::from_csv_file("../esp-nvs/tests/assets/test_nvs_data.csv").unwrap();

    // Generate binary
    let bin_file = NamedTempFile::new().unwrap();
    original_partition
        .generate_partition_file(bin_file.path(), 16384)
        .unwrap();

    // Parse binary back
    let parsed_partition = NvsPartition::parse_partition_file(bin_file.path()).unwrap();

    // Write to CSV
    let csv_file = NamedTempFile::new().unwrap();
    parsed_partition
        .clone()
        .to_csv_file(csv_file.path())
        .unwrap();

    // Parse the generated CSV and regenerate the binary
    let reparsed_partition = NvsPartition::from_csv_file(csv_file.path()).unwrap();
    let bin_file2 = NamedTempFile::new().unwrap();
    reparsed_partition
        .generate_partition_file(bin_file2.path(), 16384)
        .unwrap();

    // Verify we got all entries back
    assert_eq!(
        original_partition.entries.len(),
        parsed_partition.entries.len()
    );

    // Verify that the binaries are identical
    let bin1 = fs::read(bin_file.path()).unwrap();
    let bin2 = fs::read(bin_file2.path()).unwrap();
    assert_eq!(
        bin1, bin2,
        "CSV-binary-CSV-binary roundtrip should preserve the partition exactly"
    );
}

/// Verify in-memory APIs (`from_csv` / `to_csv`, `generate_partition` /
/// `parse_partition`) produce the same results as their file-based
/// counterparts.
#[test]
fn test_in_memory_api_parity() {
    // from_csv parses correctly
    let csv_content = "key,type,encoding,value\ntest_ns,namespace,,\nval,data,u8,42\n";
    let partition = NvsPartition::from_csv(csv_content).unwrap();
    assert_eq!(partition.entries.len(), 1);
    assert_eq!(partition.entries[0].namespace, "test_ns");
    assert_eq!(partition.entries[0].key, "val");

    // to_csv produces valid re-parseable output
    let csv_out = partition.clone().to_csv().unwrap();
    assert_csv_eq(csv_content, &csv_out);

    // generate_partition matches generate_partition_file
    let data = partition.clone().generate_partition(8192).unwrap();
    assert_eq!(data.len(), 8192);

    let bin_file = NamedTempFile::new().unwrap();
    partition
        .generate_partition_file(bin_file.path(), 8192)
        .unwrap();
    let file_data = fs::read(bin_file.path()).unwrap();
    assert_eq!(data, file_data);

    // parse_partition matches parse_partition_file
    let from_memory = NvsPartition::parse_partition(&data).unwrap();
    let from_file = NvsPartition::parse_partition_file(bin_file.path()).unwrap();
    assert_eq!(from_memory, from_file);
}

/// Roundtrip blobs of various sizes (empty, small, exact chunk boundary,
/// multi-chunk) and a near-max-size string, all in the same namespace.
#[test]
fn test_blob_and_string_roundtrip() {
    let exact_boundary: Vec<u8> = (0..4000).map(|i| (i % 256) as u8).collect();
    let large_multi_chunk: Vec<u8> = (0..5000).map(|i| (i % 256) as u8).collect();
    let big_string = "x".repeat(3998); // 3998 chars + null terminator < 4000

    let mut partition = NvsPartition { entries: vec![] };
    partition.entries.push(NvsEntry::new_data(
        "ns".to_string(),
        "empty".to_string(),
        DataValue::Binary(vec![]),
    ));
    partition.entries.push(NvsEntry::new_data(
        "ns".to_string(),
        "small_a".to_string(),
        DataValue::Binary(vec![1, 2, 3]),
    ));
    partition.entries.push(NvsEntry::new_data(
        "ns".to_string(),
        "small_b".to_string(),
        DataValue::Binary(vec![4, 5, 6, 7]),
    ));
    partition.entries.push(NvsEntry::new_data(
        "ns".to_string(),
        "exact".to_string(),
        DataValue::Binary(exact_boundary.clone()),
    ));
    partition.entries.push(NvsEntry::new_data(
        "ns".to_string(),
        "large".to_string(),
        DataValue::Binary(large_multi_chunk.clone()),
    ));
    partition.entries.push(NvsEntry::new_data(
        "ns".to_string(),
        "big_str".to_string(),
        DataValue::String(big_string.clone()),
    ));

    let bin = partition.generate_partition(32768).unwrap();
    let parsed = NvsPartition::parse_partition(&bin).unwrap();
    assert_eq!(parsed.entries.len(), 6);

    assert_entry_content(&parsed, 0, &EntryContent::Data(DataValue::Binary(vec![])));
    assert_entry_content(
        &parsed,
        1,
        &EntryContent::Data(DataValue::Binary(vec![1, 2, 3])),
    );
    assert_entry_content(
        &parsed,
        2,
        &EntryContent::Data(DataValue::Binary(vec![4, 5, 6, 7])),
    );
    assert_entry_content(
        &parsed,
        3,
        &EntryContent::Data(DataValue::Binary(exact_boundary)),
    );
    assert_entry_content(
        &parsed,
        4,
        &EntryContent::Data(DataValue::Binary(large_multi_chunk)),
    );
    assert_entry_content(
        &parsed,
        5,
        &EntryContent::Data(DataValue::String(big_string)),
    );
}

/// File entries (hex2bin, base64, string) resolve and roundtrip through binary.
#[test]
fn test_file_entry_roundtrip() {
    use std::io::Write;

    let mut hex_file = NamedTempFile::new().unwrap();
    hex_file.write_all(b"DEADBEEF").unwrap();
    hex_file.flush().unwrap();

    let mut b64_file = NamedTempFile::new().unwrap();
    let b64_content = base64::engine::general_purpose::STANDARD.encode(&[0xCA, 0xFE]);
    b64_file.write_all(b64_content.as_bytes()).unwrap();
    b64_file.flush().unwrap();

    let mut str_file = NamedTempFile::new().unwrap();
    str_file.write_all(b"hello from file").unwrap();
    str_file.flush().unwrap();

    let csv = format!(
        "key,type,encoding,value\ntest_ns,namespace,,\n\
         blob_hex,file,hex2bin,{}\n\
         blob_b64,file,base64,{}\n\
         greeting,file,string,{}\n",
        hex_file.path().display(),
        b64_file.path().display(),
        str_file.path().display(),
    );

    let partition = NvsPartition::from_csv(&csv).unwrap();
    assert_eq!(partition.entries.len(), 3);

    let bin = partition.generate_partition(8192).unwrap();
    let parsed = NvsPartition::parse_partition(&bin).unwrap();
    assert_eq!(parsed.entries.len(), 3);

    assert_entry_content(
        &parsed,
        0,
        &EntryContent::Data(DataValue::Binary(vec![0xDE, 0xAD, 0xBE, 0xEF])),
    );
    assert_entry_content(
        &parsed,
        1,
        &EntryContent::Data(DataValue::Binary(vec![0xCA, 0xFE])),
    );
    assert_entry_content(
        &parsed,
        2,
        &EntryContent::Data(DataValue::String("hello from file".to_string())),
    );
}

/// Roundtrip all primitive integer types at their boundary values, with enough
/// extra entries to exercise multi-page generation (>126 entries per page).
#[test]
fn test_primitive_roundtrip() {
    let mut partition = NvsPartition { entries: vec![] };

    // Boundary values for every integer type
    partition.entries.push(entry!("u8_max", U8, u8::MAX));
    partition.entries.push(entry!("u8_min", U8, u8::MIN));
    partition.entries.push(entry!("i8_max", I8, i8::MAX));
    partition.entries.push(entry!("i8_min", I8, i8::MIN));
    partition.entries.push(entry!("u16_max", U16, u16::MAX));
    partition.entries.push(entry!("i16_min", I16, i16::MIN));
    partition.entries.push(entry!("u32_max", U32, u32::MAX));
    partition.entries.push(entry!("i32_min", I32, i32::MIN));
    partition.entries.push(entry!("u64_max", U64, u64::MAX));
    partition.entries.push(entry!("i64_min", I64, i64::MIN));

    // Pad to >125 entries to force multi-page layout
    for i in 0..120_u8 {
        partition.entries.push(entry!(format!("k{i:03}"), U8, i));
    }

    let data = partition.generate_partition(16384).unwrap();
    let parsed = NvsPartition::parse_partition(&data).unwrap();
    assert_eq!(parsed.entries.len(), partition.entries.len());

    for (orig, parsed_entry) in partition.entries.iter().zip(parsed.entries.iter()) {
        assert_eq!(
            orig.content, parsed_entry.content,
            "mismatch for key '{}'",
            orig.key
        );
    }
}

/// Max namespaces (255) roundtrips successfully, and interleaved namespaces
/// preserve entry order through CSV serialization.
#[test]
fn test_namespace_handling() {
    // 255 namespaces is the maximum
    let mut partition = NvsPartition { entries: vec![] };
    for i in 0..255_u8 {
        partition.entries.push(NvsEntry::new_data(
            format!("ns_{i:03}"),
            "val".to_string(),
            DataValue::U8(i),
        ));
    }

    let bin = partition.generate_partition(24576).unwrap();
    let parsed = NvsPartition::parse_partition(&bin).unwrap();
    assert_eq!(parsed.entries.len(), 255);

    // Interleaved namespaces preserve entry order through CSV
    let mut interleaved = NvsPartition { entries: vec![] };
    interleaved.entries.push(NvsEntry::new_data(
        "ns_a".to_string(),
        "first".to_string(),
        DataValue::U8(1),
    ));
    interleaved.entries.push(NvsEntry::new_data(
        "ns_b".to_string(),
        "second".to_string(),
        DataValue::U8(2),
    ));
    interleaved.entries.push(NvsEntry::new_data(
        "ns_a".to_string(),
        "third".to_string(),
        DataValue::U8(3),
    ));

    let csv = interleaved.to_csv().unwrap();
    let reparsed = NvsPartition::from_csv(&csv).unwrap();

    assert_eq!(reparsed.entries.len(), 3);
    assert_eq!(reparsed.entries[0].key, "first");
    assert_eq!(reparsed.entries[0].namespace, "ns_a");
    assert_eq!(reparsed.entries[1].key, "second");
    assert_eq!(reparsed.entries[1].namespace, "ns_b");
    assert_eq!(reparsed.entries[2].key, "third");
    assert_eq!(reparsed.entries[2].namespace, "ns_a");
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
