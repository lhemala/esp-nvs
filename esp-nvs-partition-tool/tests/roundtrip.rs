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

/// Hand-crafted binary containing a legacy blob (type 0x41) parses correctly.
#[test]
fn test_parse_legacy_blob() {
    use esp_nvs_partition_tool::partition::crc::crc32;

    let mut page = vec![0xFF_u8; 4096];

    // Page header (32 bytes): state = ACTIVE
    page[0..4].copy_from_slice(&0xFFFFFFFE_u32.to_le_bytes());
    page[4..8].copy_from_slice(&0_u32.to_le_bytes());
    page[8] = 0xFE; // version
    let hdr_crc = crc32(&page[4..28]);
    page[28..32].copy_from_slice(&hdr_crc.to_le_bytes());

    // Entry-state bitmap: entries 0,1,2 = Written; rest = Empty
    page[32] = 0xEA;

    let entries_base = 64;

    // Entry 0: Namespace "test_ns" → index 1
    let e0 = entries_base;
    page[e0] = 0;
    page[e0 + 1] = 0x01;
    page[e0 + 2] = 1;
    page[e0 + 3] = 0xFF;
    let ns_key = b"test_ns\0\0\0\0\0\0\0\0\0";
    page[e0 + 8..e0 + 24].copy_from_slice(ns_key);
    page[e0 + 24] = 1;
    let e0_crc = esp_nvs_partition_tool::partition::crc::crc32_entry(&page[e0..e0 + 32]);
    page[e0 + 4..e0 + 8].copy_from_slice(&e0_crc.to_le_bytes());

    // Entry 1: Legacy blob header (type 0x41, span=2)
    let e1 = entries_base + 32;
    page[e1] = 1;
    page[e1 + 1] = 0x41;
    page[e1 + 2] = 2;
    page[e1 + 3] = 0xFF;
    let blob_key = b"my_blob\0\0\0\0\0\0\0\0\0";
    page[e1 + 8..e1 + 24].copy_from_slice(blob_key);
    let payload: &[u8] = &[0xCA, 0xFE, 0xBA, 0xBE];
    let payload_size = payload.len() as u16;
    page[e1 + 24..e1 + 26].copy_from_slice(&payload_size.to_le_bytes());
    page[e1 + 26..e1 + 28].copy_from_slice(&0xFFFF_u16.to_le_bytes());
    let payload_crc = crc32(payload);
    page[e1 + 28..e1 + 32].copy_from_slice(&payload_crc.to_le_bytes());
    let e1_crc = esp_nvs_partition_tool::partition::crc::crc32_entry(&page[e1..e1 + 32]);
    page[e1 + 4..e1 + 8].copy_from_slice(&e1_crc.to_le_bytes());

    // Entry 2: Blob data payload
    let e2 = entries_base + 64;
    page[e2..e2 + payload.len()].copy_from_slice(payload);

    let parsed = NvsPartition::parse_partition(&page).unwrap();
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].namespace, "test_ns");
    assert_eq!(parsed.entries[0].key, "my_blob");

    match &parsed.entries[0].content {
        EntryContent::Data(DataValue::Binary(data)) => {
            assert_eq!(data, &[0xCA, 0xFE, 0xBA, 0xBE]);
        }
        other => panic!("expected legacy binary blob, got {:?}", other),
    }
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

    let data = partition.generate_partition(8192).unwrap();
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
