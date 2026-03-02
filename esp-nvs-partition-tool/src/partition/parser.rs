use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::{
    DataValue,
    NvsEntry,
    MAX_KEY_LENGTH,
};
use crate::error::Error;
use crate::partition::consts::*;
use crate::partition::crc::{
    crc32,
    crc32_entry,
};
use crate::NvsPartition;

#[derive(Clone, PartialEq, Eq, Hash)]
struct BlobKey {
    namespace_id: u8,
    key: String,
}

struct BlobChunk {
    chunk_index: u8,
    data: Vec<u8>,
}

struct BlobInfo {
    size: u32,
    chunk_count: u8,
}

/// Page-level context shared across entry-parsing helpers.
struct PageContext<'a> {
    data: &'a [u8],
    bitmap_offset: usize,
    entries_offset: usize,
    page_idx: usize,
}

/// Parse an NVS partition binary file at the given `path`.
pub(crate) fn parse_binary<P: AsRef<Path>>(path: P) -> Result<NvsPartition, Error> {
    let data = fs::read(path)?;
    parse_binary_data(&data)
}

/// Parse an NVS partition binary from an in-memory byte slice.
pub(crate) fn parse_binary_data(data: &[u8]) -> Result<NvsPartition, Error> {
    if data.is_empty() {
        return Err(Error::InvalidValue(
            "binary data is empty; an NVS partition requires at least one page (4096 bytes)"
                .to_string(),
        ));
    }

    if !data.len().is_multiple_of(FLASH_SECTOR_SIZE) {
        return Err(Error::InvalidValue(format!(
            "binary size {} is not a multiple of page size {}",
            data.len(),
            FLASH_SECTOR_SIZE
        )));
    }

    let mut partition = NvsPartition { entries: vec![] };
    let num_pages = data.len() / FLASH_SECTOR_SIZE;

    // Collect blob data: (namespace_id, key) -> Vec of (chunk_index, data)
    let mut blob_data_chunks: HashMap<BlobKey, Vec<BlobChunk>> = HashMap::new();
    let mut blob_indices: HashMap<BlobKey, BlobInfo> = HashMap::new();
    let mut blob_positions: HashMap<BlobKey, usize> = HashMap::new();
    // Map namespace binary indices to their names
    let mut namespace_names: HashMap<u8, String> = HashMap::new();

    // First pass: collect all entries
    for page_idx in 0..num_pages {
        let page_offset = page_idx * FLASH_SECTOR_SIZE;
        let page_data = &data[page_offset..page_offset + FLASH_SECTOR_SIZE];

        // Parse page header
        let state = read_u32(page_data, 0);

        // Skip uninitialized pages
        if state == 0xFFFFFFFF {
            continue;
        }

        // Skip pages that are being freed (compaction in progress)
        if state == PAGE_STATE_FREEING {
            continue;
        }

        if state == PAGE_STATE_CORRUPT {
            return Err(Error::InvalidValue(format!(
                "corrupt page detected at page {}",
                page_idx
            )));
        }

        // Only Active and Full pages contain valid data
        if state != PAGE_STATE_ACTIVE && state != PAGE_STATE_FULL {
            return Err(Error::InvalidValue(format!(
                "unknown page state 0x{:08x} at page {}",
                state, page_idx
            )));
        }

        // Validate page version byte (must be 0xFE for NVS format version 2)
        let version = page_data[8];
        if version != 0xFE {
            return Err(Error::InvalidValue(format!(
                "unsupported NVS page version 0x{:02x} at page {} (expected 0xFE)",
                version, page_idx
            )));
        }

        // Validate page header CRC (stored at offset 28, computed over bytes 4..28)
        let stored_header_crc = read_u32(page_data, 28);
        let computed_header_crc = crc32(&page_data[4..28]);
        if stored_header_crc != computed_header_crc {
            return Err(Error::InvalidValue(format!(
                "page header CRC mismatch at page {}: stored 0x{:08x}, computed 0x{:08x}",
                page_idx, stored_header_crc, computed_header_crc
            )));
        }

        // Parse entries
        let page = PageContext {
            data: page_data,
            bitmap_offset: PAGE_HEADER_SIZE,
            entries_offset: PAGE_HEADER_SIZE + ENTRY_STATE_BITMAP_SIZE,
            page_idx,
        };

        let mut entry_idx = 0;
        while entry_idx < ENTRIES_PER_PAGE {
            // Check if entry is written
            let bitmap_byte_idx = entry_idx / 4;
            let bitmap_bit_offset = (entry_idx % 4) * 2;
            let bitmap_byte = page.data[page.bitmap_offset + bitmap_byte_idx];
            let entry_state = (bitmap_byte >> bitmap_bit_offset) & 0b11;

            if entry_state != ENTRY_STATE_WRITTEN {
                entry_idx += 1;
                continue;
            }

            let entry_offset = page.entries_offset + (entry_idx * ENTRY_SIZE);
            let entry_data = &page.data[entry_offset..entry_offset + ENTRY_SIZE];

            let namespace_idx = entry_data[0];
            let item_type = entry_data[1];
            let span = entry_data[2];
            let chunk_index = entry_data[3];

            // Extract key (16 bytes, null-terminated)
            let key_bytes = &entry_data[8..24];
            let key = extract_key(key_bytes)?;

            // Extract data field (8 bytes)
            let data_field = &entry_data[24..32];

            // Validate entry CRC (stored at bytes 4..8, computed over 0..4 + 8..32)
            let stored_entry_crc = read_u32(entry_data, 4);
            let computed_entry_crc = crc32_entry(entry_data);
            if stored_entry_crc != computed_entry_crc {
                return Err(Error::InvalidValue(format!(
                    "entry CRC mismatch at page {}, entry {}: stored 0x{:08x}, computed 0x{:08x}",
                    page.page_idx, entry_idx, stored_entry_crc, computed_entry_crc
                )));
            }

            match item_type {
                ITEM_TYPE_U8 if namespace_idx == 0 => {
                    // This is a namespace entry — record the index-to-name mapping
                    let ns_id = data_field[0];
                    if let Some(existing) = namespace_names.get(&ns_id) {
                        return Err(Error::InvalidValue(format!(
                            "duplicate namespace index {} at page {}, entry {}: '{}' conflicts with '{}'",
                            ns_id, page.page_idx, entry_idx, key, existing
                        )));
                    }
                    namespace_names.insert(ns_id, key);
                    entry_idx += 1;
                }
                t @ (ITEM_TYPE_U8 | ITEM_TYPE_I8 | ITEM_TYPE_U16 | ITEM_TYPE_I16
                | ITEM_TYPE_U32 | ITEM_TYPE_I32 | ITEM_TYPE_U64 | ITEM_TYPE_I64) => {
                    let ns = resolve_namespace(&namespace_names, namespace_idx)?;
                    let value = decode_primitive(data_field, t);
                    partition.entries.push(NvsEntry::new_data(ns, key, value));
                    entry_idx += 1;
                }
                ITEM_TYPE_SIZED => {
                    // ITEM_TYPE_SIZED (0x21) is always a null-terminated string
                    // (SZ type) in the ESP-IDF NVS format.
                    let ns = resolve_namespace(&namespace_names, namespace_idx)?;
                    let data = read_span_data(&page, entry_idx, span, data_field, &key, "SIZED")?;

                    let s = std::str::from_utf8(&data).map_err(|e| {
                        Error::InvalidValue(format!(
                            "invalid UTF-8 in string entry '{}': {}",
                            key, e
                        ))
                    })?;

                    partition.entries.push(NvsEntry::new_data(
                        ns,
                        key,
                        DataValue::String(s.trim_end_matches('\0').to_string()),
                    ));

                    entry_idx += span as usize;
                }
                ITEM_TYPE_BLOB => {
                    // ITEM_TYPE_BLOB (0x41) is a legacy single-page blob
                    // (version 1 format). Same structure as SIZED but always
                    // contains binary data, not a string.
                    let ns = resolve_namespace(&namespace_names, namespace_idx)?;
                    let data =
                        read_span_data(&page, entry_idx, span, data_field, &key, "legacy BLOB")?;

                    partition
                        .entries
                        .push(NvsEntry::new_data(ns, key, DataValue::Binary(data)));

                    entry_idx += span as usize;
                }
                ITEM_TYPE_BLOB_INDEX => {
                    let ns = resolve_namespace(&namespace_names, namespace_idx)?;

                    // BLOB_INDEX entries must always have span = 1
                    if span != 1 {
                        return Err(Error::InvalidValue(format!(
                            "invalid span {} for BLOB_INDEX entry at page {}, entry {} (expected 1)",
                            span, page.page_idx, entry_idx
                        )));
                    }

                    // Record blob index information
                    let blob_size = read_u32(data_field, 0);
                    let chunk_count = data_field[4];

                    let blob_key = BlobKey {
                        namespace_id: namespace_idx,
                        key: key.clone(),
                    };
                    if blob_indices.contains_key(&blob_key) {
                        return Err(Error::InvalidValue(format!(
                            "duplicate BLOB_INDEX for key '{}' at page {}, entry {}",
                            key, page.page_idx, entry_idx
                        )));
                    }
                    blob_indices.insert(
                        blob_key.clone(),
                        BlobInfo {
                            size: blob_size,
                            chunk_count,
                        },
                    );

                    // Insert a placeholder entry at this position. The second
                    // pass will replace it with the fully assembled blob data
                    // once all chunks have been collected across pages.
                    blob_positions.insert(blob_key, partition.entries.len());
                    partition.entries.push(NvsEntry::new_data(
                        ns,
                        key,
                        DataValue::Binary(Vec::new()),
                    ));

                    entry_idx += 1;
                }
                ITEM_TYPE_BLOB_DATA => {
                    // Collect blob data chunk
                    let blob_key = BlobKey {
                        namespace_id: namespace_idx,
                        key: key.clone(),
                    };
                    let data =
                        read_span_data(&page, entry_idx, span, data_field, &key, "BLOB_DATA")?;

                    blob_data_chunks
                        .entry(blob_key)
                        .or_default()
                        .push(BlobChunk { chunk_index, data });

                    entry_idx += span as usize;
                }
                _ => {
                    return Err(Error::InvalidValue(format!(
                        "unknown item type 0x{:02x} at page {}, entry {}",
                        item_type, page.page_idx, entry_idx
                    )));
                }
            }
        }
    }

    // Second pass: assemble blob entries that have indices and replace placeholders
    for (blob_key, info) in &blob_indices {
        if let Some(mut chunks) = blob_data_chunks.remove(blob_key) {
            // Verify the number of collected chunks matches the index header
            if chunks.len() != info.chunk_count as usize {
                return Err(Error::InvalidValue(format!(
                    "BLOB_INDEX for key '{}' expects {} chunks but {} were found",
                    blob_key.key,
                    info.chunk_count,
                    chunks.len()
                )));
            }

            // Sort chunks by index
            chunks.sort_by_key(|c| c.chunk_index);

            // Concatenate chunk data
            let mut blob_data = Vec::new();
            for chunk in chunks {
                blob_data.extend_from_slice(&chunk.data);
            }

            // Trim to actual size
            blob_data.truncate(info.size as usize);

            // Replace the placeholder entry at its original position
            if let Some(&pos) = blob_positions.get(blob_key) {
                let ns = resolve_namespace(&namespace_names, blob_key.namespace_id)?;
                partition.entries[pos] =
                    NvsEntry::new_data(ns, blob_key.key.clone(), DataValue::Binary(blob_data));
            }
        } else if info.size > 0 {
            return Err(Error::InvalidValue(format!(
                "BLOB_INDEX for key '{}' references {} bytes but no BLOB_DATA chunks were found",
                blob_key.key, info.size
            )));
        }
    }

    // Check for orphaned BLOB_DATA chunks with no matching BLOB_INDEX
    if !blob_data_chunks.is_empty() {
        let orphaned_keys: Vec<String> = blob_data_chunks.keys().map(|k| k.key.clone()).collect();
        return Err(Error::InvalidValue(format!(
            "found BLOB_DATA chunks with no matching BLOB_INDEX for keys: {}",
            orphaned_keys.join(", ")
        )));
    }

    Ok(partition)
}

fn resolve_namespace(
    namespace_names: &HashMap<u8, String>,
    namespace_idx: u8,
) -> Result<String, Error> {
    namespace_names
        .get(&namespace_idx)
        .cloned()
        .ok_or_else(|| Error::InvalidValue(format!("unknown namespace index {}", namespace_idx)))
}

fn extract_key(key_bytes: &[u8]) -> Result<String, Error> {
    // Find the null terminator
    let key_len = key_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(key_bytes.len());

    // ESP-IDF NVS keys must be 1-15 characters; an empty or overlong key indicates corruption.
    // Reject keys that are empty, exceed MAX_KEY_LENGTH, or occupy the full field without a
    // null terminator (the latter is covered by key_len > MAX_KEY_LENGTH, since the field
    // size is MAX_KEY_LENGTH + 1).
    if key_len == 0 || key_len > MAX_KEY_LENGTH {
        return Err(Error::InvalidKey(
            "entry has an invalid key length".to_string(),
        ));
    }

    let key_str = std::str::from_utf8(&key_bytes[..key_len]).map_err(|e| {
        Error::InvalidValue(format!(
            "invalid UTF-8 in key (bytes: {:?}): {}",
            &key_bytes[..key_len.min(16)],
            e
        ))
    })?;

    Ok(key_str.to_string())
}

/// Decode a primitive value from the 8-byte data field of an entry.
fn decode_primitive(data_field: &[u8], item_type: u8) -> DataValue {
    match item_type {
        ITEM_TYPE_U8 => DataValue::U8(data_field[0]),
        ITEM_TYPE_I8 => DataValue::I8(data_field[0] as i8),
        ITEM_TYPE_U16 => DataValue::U16(read_u16(data_field, 0)),
        ITEM_TYPE_I16 => DataValue::I16(read_u16(data_field, 0) as i16),
        ITEM_TYPE_U32 => DataValue::U32(read_u32(data_field, 0)),
        ITEM_TYPE_I32 => DataValue::I32(read_u32(data_field, 0) as i32),
        ITEM_TYPE_U64 => DataValue::U64(read_u64(data_field, 0)),
        ITEM_TYPE_I64 => DataValue::I64(read_u64(data_field, 0) as i64),
        _ => unreachable!("decode_primitive called with non-primitive item type 0x{item_type:02x}"),
    }
}

/// Read variable-length data from the sub-entries that follow a span header.
///
/// SIZED, legacy BLOB, and BLOB_DATA entries all share the same on-disk
/// layout: a header entry whose 8-byte data field contains
/// `[size: u16, reserved: u16, crc32: u32]`, followed by `span - 1`
/// consecutive 32-byte entries holding the actual payload.  This helper
/// validates the reserved field, span, bitmap states, CRC, and returns the
/// collected payload trimmed to `size`.
fn read_span_data(
    page: &PageContext,
    entry_idx: usize,
    span: u8,
    data_field: &[u8],
    key: &str,
    label: &str,
) -> Result<Vec<u8>, Error> {
    let size = read_u16(data_field, 0) as usize;

    let reserved = read_u16(data_field, 2);
    if reserved != RESERVED_U16 {
        return Err(Error::InvalidValue(format!(
            "unexpected reserved field 0x{:04x} in {} entry '{}' at page {}, entry {}",
            reserved, label, key, page.page_idx, entry_idx
        )));
    }

    if span == 0 {
        return Err(Error::InvalidValue(format!(
            "invalid span value 0 for {} entry at page {}, entry {}",
            label, page.page_idx, entry_idx
        )));
    }

    if entry_idx + span as usize > ENTRIES_PER_PAGE {
        return Err(Error::InvalidValue(format!(
            "{} entry span {} at page {}, entry {} exceeds page boundary",
            label, span, page.page_idx, entry_idx
        )));
    }

    validate_data_sub_entries(page, entry_idx, span, key, label)?;

    let num_data_entries = (span - 1) as usize;
    let mut collected = Vec::with_capacity(num_data_entries * ENTRY_SIZE);
    for i in 0..num_data_entries {
        let data_entry_idx = entry_idx + 1 + i;
        if data_entry_idx >= ENTRIES_PER_PAGE {
            break;
        }
        let data_entry_offset = page.entries_offset + (data_entry_idx * ENTRY_SIZE);
        collected.extend_from_slice(&page.data[data_entry_offset..data_entry_offset + ENTRY_SIZE]);
    }
    collected.truncate(size);

    let stored_crc = read_u32(data_field, 4);
    let computed_crc = crc32(&collected);
    if stored_crc != computed_crc {
        return Err(Error::InvalidValue(format!(
            "{} data CRC mismatch for key '{}': stored 0x{:08x}, computed 0x{:08x}",
            label, key, stored_crc, computed_crc
        )));
    }

    Ok(collected)
}

/// Verify that every data sub-entry covered by `span` is marked Written in the
/// entry-state bitmap.  Returns an error naming `label` and `key` on mismatch.
fn validate_data_sub_entries(
    page: &PageContext,
    entry_idx: usize,
    span: u8,
    key: &str,
    label: &str,
) -> Result<(), Error> {
    let num_data_entries = (span - 1) as usize;
    for i in 0..num_data_entries {
        let data_entry_idx = entry_idx + 1 + i;
        if data_entry_idx >= ENTRIES_PER_PAGE {
            break;
        }
        let sub_bitmap_byte_idx = data_entry_idx / 4;
        let sub_bitmap_bit_offset = (data_entry_idx % 4) * 2;
        let sub_bitmap_byte = page.data[page.bitmap_offset + sub_bitmap_byte_idx];
        let sub_entry_state = (sub_bitmap_byte >> sub_bitmap_bit_offset) & 0b11;
        if sub_entry_state != ENTRY_STATE_WRITTEN {
            return Err(Error::InvalidValue(format!(
                "{} entry '{}' data sub-entry {} at page {} is not marked Written (state {})",
                label, key, data_entry_idx, page.page_idx, sub_entry_state
            )));
        }
    }
    Ok(())
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}
