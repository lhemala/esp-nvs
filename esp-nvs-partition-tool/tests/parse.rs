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

/// A CSV file starting with a UTF-8 byte order mark is still CSV. Its first byte is above 0x80, so
/// it was taken for a binary partition and rejected for its size.
#[test]
fn test_csv_with_byte_order_mark() {
    let csv = "\u{feff}key,type,encoding,value\nns,namespace,,\nk,data,u8,1\n";
    let partition = NvsPartition::try_from(csv.as_bytes()).unwrap();
    assert_eq!(partition.entries.len(), 1);
    assert_eq!(partition.entries[0].key, "k");
}

/// A key in a binary partition that is not UTF-8 is reported, rather than written to the CSV cut
/// short at its first invalid byte.
#[test]
fn test_binary_with_a_key_that_is_not_utf8() {
    let partition = NvsPartition {
        entries: vec![esp_nvs_partition_tool::NvsEntry::new_data(
            "ns".to_string(),
            "abc".to_string(),
            DataValue::U8(1),
        )],
    };
    let mut image = partition.generate_partition(0x3000).unwrap();

    // Find the item header of `abc` and put an invalid UTF-8 sequence into its key.
    let entry = (0..126)
        .map(|entry| 64 + entry * 32)
        .find(|&offset| &image[offset + 8..offset + 11] == b"abc")
        .unwrap();
    image[entry + 9] = 0xC3;
    image[entry + 10] = 0x28;
    // Fix the item CRC: it covers bytes 0..4, the key and the data.
    let mut crc_input = image[entry..entry + 4].to_vec();
    crc_input.extend_from_slice(&image[entry + 8..entry + 32]);
    let crc = esp_nvs::platform::software_crc32(u32::MAX, &crc_input);
    image[entry + 4..entry + 8].copy_from_slice(&crc.to_le_bytes());

    assert!(matches!(
        NvsPartition::try_from_bytes(image),
        Err(esp_nvs_partition_tool::Error::InvalidKey(_))
    ));
}

/// More encodings ESP-IDF's generator takes for data rows: `binary` stores the value text itself as
/// a blob, and base64 may contain whitespace, as when wrapped over lines. Both were rejected.
#[test]
fn test_binary_encoding_and_wrapped_base64() {
    let csv = "key,type,encoding,value\nns,namespace,,\nraw,data,binary,abc\nb64,data,base64,\"aGVs\nbG8=\"\n";
    let partition = NvsPartition::try_from_str(csv).unwrap();
    assert_eq!(
        partition.entries[0].content,
        EntryContent::Data(DataValue::Binary(b"abc".to_vec()))
    );
    assert_eq!(
        partition.entries[1].content,
        EntryContent::Data(DataValue::Binary(b"hello".to_vec()))
    );
}
