use std::collections::HashMap;
use std::fs::read;

use base64::Engine;

use super::{
    validate_key,
    DataValue,
    EntryContent,
    FileEncoding,
};
use crate::error::Error;
use crate::partition::consts::*;
use crate::partition::crc::{
    crc32,
    crc32_entry,
};
use crate::NvsPartition;

/// Generate an NVS partition binary in memory and return it as a `Vec<u8>`.
///
/// `size` must be a multiple of 4096 (the ESP-IDF flash sector size).
pub(crate) fn generate_partition_data(
    partition: &NvsPartition,
    size: usize,
) -> Result<Vec<u8>, Error> {
    if size < FLASH_SECTOR_SIZE {
        return Err(Error::PartitionTooSmall(size));
    } else if !size.is_multiple_of(FLASH_SECTOR_SIZE) {
        return Err(Error::InvalidPartitionSize(size));
    }

    let mut writer = PartitionWriter::new(size);
    let mut namespace_map: HashMap<String, u8> = HashMap::new();
    let mut namespace_counter: u8 = 0;

    for entry in &partition.entries {
        // Ensure the entry's namespace is registered in the binary
        let ns_index = match namespace_map.get(&entry.namespace) {
            Some(&id) => id,
            None => {
                // Register new namespace
                namespace_counter = namespace_counter
                    .checked_add(1)
                    .ok_or(Error::TooManyNamespaces)?;
                namespace_map.insert(entry.namespace.clone(), namespace_counter);

                // Write namespace entry to binary
                if writer.current_entry >= ENTRIES_PER_PAGE {
                    writer.advance_page()?;
                }

                writer.write_namespace_entry(&entry.namespace, namespace_counter)?;

                namespace_counter
            }
        };

        // Resolve the value from the entry content.
        // For file entries, read the file and convert to a DataValue at generation time.
        let resolved_value;
        let value = match &entry.content {
            EntryContent::Data(val) => val,
            EntryContent::File {
                encoding,
                file_path,
            } => {
                let content = read(file_path)?;
                resolved_value = parse_file_content(&content, encoding)?;
                &resolved_value
            }
        };

        // Compute how many entries must fit on the current page.
        // Primitives need 1; strings need header + data entries (all on one
        // page); blobs only need the BLOB_INDEX entry here — BLOB_DATA
        // handles page spanning internally.
        let page_space_needed = match value {
            DataValue::Binary(_) => 1,
            DataValue::String(s) => 1 + (s.len() + 1).div_ceil(ENTRY_SIZE),
            _ => 1, // primitives
        };

        if writer.current_entry + page_space_needed > ENTRIES_PER_PAGE {
            writer.advance_page()?;
        }

        writer.write_data_entry(ns_index, &entry.key, value)?;
    }

    // Mark the last page as full only if it has no remaining free entries
    if writer.current_entry >= ENTRIES_PER_PAGE {
        write_page_header(
            &mut writer.data,
            writer.current_page,
            page_seq(writer.current_page)?,
            PAGE_STATE_FULL,
        );
    }

    Ok(writer.data)
}

struct PartitionWriter {
    data: Vec<u8>,
    current_page: usize,
    current_entry: usize,
    num_pages: usize,
}

impl PartitionWriter {
    fn new(size: usize) -> Self {
        let num_pages = size / FLASH_SECTOR_SIZE;
        let mut data = vec![0xFF; size];

        // Initialize first page header
        write_page_header(&mut data, 0, 0, PAGE_STATE_ACTIVE);

        Self {
            data,
            current_page: 0,
            current_entry: 0,
            num_pages,
        }
    }

    fn advance_page(&mut self) -> Result<(), Error> {
        write_page_header(
            &mut self.data,
            self.current_page,
            page_seq(self.current_page)?,
            PAGE_STATE_FULL,
        );

        self.current_page += 1;
        if self.current_page >= self.num_pages {
            return Err(Error::PartitionTooSmall(self.num_pages * FLASH_SECTOR_SIZE));
        }

        write_page_header(
            &mut self.data,
            self.current_page,
            page_seq(self.current_page)?,
            PAGE_STATE_ACTIVE,
        );

        self.current_entry = 0;

        Ok(())
    }

    fn write_namespace_entry(&mut self, key: &str, namespace_index: u8) -> Result<(), Error> {
        let mut data = [0xFF_u8; 8];
        data[0] = namespace_index;
        self.write_entry_header(0, ITEM_TYPE_U8, 1, 0xFF, key, &data)
    }

    /// Write a single 32-byte NVS entry. The caller provides the 8-byte data
    /// field; this method handles the entry state bitmap, header bytes, key,
    /// CRC, and entry-index advance.
    fn write_entry_header(
        &mut self,
        namespace_index: u8,
        item_type: u8,
        span: u8,
        chunk_index: u8,
        key: &str,
        data: &[u8; 8],
    ) -> Result<(), Error> {
        let offset = calc_entry_offset(self.current_page, self.current_entry);

        set_entry_state(
            &mut self.data,
            self.current_page,
            self.current_entry,
            ENTRY_STATE_WRITTEN,
        );

        self.data[offset] = namespace_index;
        self.data[offset + 1] = item_type;
        self.data[offset + 2] = span;
        self.data[offset + 3] = chunk_index;

        write_key(&mut self.data[offset + 8..offset + 24], key)?;
        self.data[offset + 24..offset + 32].copy_from_slice(data);

        let entry_crc = crc32_entry(&self.data[offset..offset + ENTRY_SIZE]);
        self.data[offset + 4..offset + 8].copy_from_slice(&entry_crc.to_le_bytes());

        self.current_entry += 1;
        Ok(())
    }

    /// Write raw bytes across consecutive entry slots, marking each as written.
    fn write_data_entries(&mut self, bytes: &[u8]) {
        for (i, chunk) in bytes.chunks(ENTRY_SIZE).enumerate() {
            let entry_idx = self.current_entry + i;
            set_entry_state(
                &mut self.data,
                self.current_page,
                entry_idx,
                ENTRY_STATE_WRITTEN,
            );
            let offset = calc_entry_offset(self.current_page, entry_idx);
            self.data[offset..offset + chunk.len()].copy_from_slice(chunk);
        }
        self.current_entry += bytes.len().div_ceil(ENTRY_SIZE);
    }

    fn write_data_entry(
        &mut self,
        namespace_index: u8,
        key: &str,
        value: &DataValue,
    ) -> Result<(), Error> {
        match value {
            DataValue::U8(_)
            | DataValue::I8(_)
            | DataValue::U16(_)
            | DataValue::I16(_)
            | DataValue::U32(_)
            | DataValue::I32(_)
            | DataValue::U64(_)
            | DataValue::I64(_) => {
                self.write_primitive_entry(namespace_index, key, value)?;
            }
            DataValue::String(s) => {
                let mut bytes = s.as_bytes().to_vec();

                // ESP-IDF stores strings with a null terminator included in the size
                bytes.push(0);

                // Strings always use SIZED type (0x21) and must fit on a single page
                const MAX_STRING_SIZE: usize = (ENTRIES_PER_PAGE - 1) * ENTRY_SIZE; // 4000 bytes
                if bytes.len() > MAX_STRING_SIZE {
                    return Err(Error::InvalidValue(format!(
                        "string for key '{}' is too large ({} bytes, max {})",
                        key,
                        bytes.len(),
                        MAX_STRING_SIZE
                    )));
                }

                self.write_sized_entry(namespace_index, key, &bytes)?;
            }
            DataValue::Binary(b) => {
                self.write_blob_entries(namespace_index, key, b)?;
            }
        }

        Ok(())
    }

    fn write_primitive_entry(
        &mut self,
        namespace_index: u8,
        key: &str,
        value: &DataValue,
    ) -> Result<(), Error> {
        let item_type = match value {
            DataValue::U8(_) => ITEM_TYPE_U8,
            DataValue::I8(_) => ITEM_TYPE_I8,
            DataValue::U16(_) => ITEM_TYPE_U16,
            DataValue::I16(_) => ITEM_TYPE_I16,
            DataValue::U32(_) => ITEM_TYPE_U32,
            DataValue::I32(_) => ITEM_TYPE_I32,
            DataValue::U64(_) => ITEM_TYPE_U64,
            DataValue::I64(_) => ITEM_TYPE_I64,
            _ => unreachable!("write_primitive_entry called with non-primitive DataValue"),
        };

        let mut data = [0xFF_u8; 8];
        match value {
            DataValue::U8(v) => data[0] = *v,
            DataValue::I8(v) => data[0] = *v as u8,
            DataValue::U16(v) => data[..2].copy_from_slice(&v.to_le_bytes()),
            DataValue::I16(v) => data[..2].copy_from_slice(&v.to_le_bytes()),
            DataValue::U32(v) => data[..4].copy_from_slice(&v.to_le_bytes()),
            DataValue::I32(v) => data[..4].copy_from_slice(&v.to_le_bytes()),
            DataValue::U64(v) => data.copy_from_slice(&v.to_le_bytes()),
            DataValue::I64(v) => data.copy_from_slice(&v.to_le_bytes()),
            _ => unreachable!("write_primitive_entry called with non-primitive DataValue"),
        }

        self.write_entry_header(namespace_index, item_type, 1, 0xFF, key, &data)
    }

    fn write_sized_entry(
        &mut self,
        namespace_index: u8,
        key: &str,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let num_data_entries = bytes.len().div_ceil(ENTRY_SIZE);
        let span = u8::try_from(1 + num_data_entries).map_err(|_| {
            Error::InvalidValue(format!(
                "SIZED entry span {} exceeds u8 maximum",
                1 + num_data_entries
            ))
        })?;

        let data = build_sized_data_field(bytes)?;
        self.write_entry_header(namespace_index, ITEM_TYPE_SIZED, span, 0xFF, key, &data)?;
        self.write_data_entries(bytes);
        Ok(())
    }

    fn write_blob_entries(
        &mut self,
        namespace_index: u8,
        key: &str,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let chunk_count = bytes.len().div_ceil(MAX_DATA_PER_CHUNK);
        let chunk_count_u8 = u8::try_from(chunk_count).map_err(|_| {
            Error::InvalidValue(format!(
                "blob for key '{}' requires {} chunks, exceeding the maximum of 255",
                key, chunk_count
            ))
        })?;

        // Ensure BLOB_INDEX entry fits on current page
        if self.current_entry >= ENTRIES_PER_PAGE {
            self.advance_page()?;
        }

        // Write BLOB_INDEX entry
        let blob_size_u32 = u32::try_from(bytes.len()).map_err(|_| {
            Error::InvalidValue(format!(
                "blob for key '{}' is too large ({} bytes, max {})",
                key,
                bytes.len(),
                u32::MAX
            ))
        })?;
        let mut index_data = [0xFF_u8; 8];
        index_data[..4].copy_from_slice(&blob_size_u32.to_le_bytes());
        index_data[4] = chunk_count_u8;
        index_data[5] = 0; // chunk_start
        self.write_entry_header(
            namespace_index,
            ITEM_TYPE_BLOB_INDEX,
            1,
            0,
            key,
            &index_data,
        )?;

        // Write BLOB_DATA chunks, spanning pages as needed
        for (chunk_idx, chunk_data) in bytes.chunks(MAX_DATA_PER_CHUNK).enumerate() {
            let num_data_entries = chunk_data.len().div_ceil(ENTRY_SIZE);
            let chunk_span = 1 + num_data_entries;

            if self.current_entry + chunk_span > ENTRIES_PER_PAGE {
                self.advance_page()?;
            }

            let span = u8::try_from(chunk_span).map_err(|_| {
                Error::InvalidValue(format!(
                    "BLOB_DATA chunk span {} for key '{}' exceeds u8 maximum",
                    chunk_span, key
                ))
            })?;

            let chunk_idx_u8 = u8::try_from(chunk_idx).map_err(|_| {
                Error::InvalidValue(format!(
                    "BLOB_DATA chunk index {} for key '{}' exceeds u8 maximum",
                    chunk_idx, key
                ))
            })?;

            let data = build_sized_data_field(chunk_data)?;
            self.write_entry_header(
                namespace_index,
                ITEM_TYPE_BLOB_DATA,
                span,
                chunk_idx_u8,
                key,
                &data,
            )?;
            self.write_data_entries(chunk_data);
        }

        Ok(())
    }
}

/// Build the 8-byte data field for SIZED and BLOB_DATA entries:
/// `[size:u16, reserved:u16, crc32:u32]`.
fn build_sized_data_field(bytes: &[u8]) -> Result<[u8; 8], Error> {
    let size = u16::try_from(bytes.len()).map_err(|_| {
        Error::InvalidValue(format!("data size {} exceeds u16 maximum", bytes.len()))
    })?;
    let mut data = [0u8; 8];
    data[..2].copy_from_slice(&size.to_le_bytes());
    data[2..4].copy_from_slice(&RESERVED_U16.to_le_bytes());
    let crc = crc32(bytes);
    data[4..].copy_from_slice(&crc.to_le_bytes());
    Ok(data)
}

fn page_seq(page_index: usize) -> Result<u32, Error> {
    u32::try_from(page_index)
        .map_err(|_| Error::InvalidValue(format!("page index {} exceeds u32 range", page_index)))
}

fn calc_entry_offset(page_index: usize, entry_index: usize) -> usize {
    page_index * FLASH_SECTOR_SIZE
        + PAGE_HEADER_SIZE
        + ENTRY_STATE_BITMAP_SIZE
        + (entry_index * ENTRY_SIZE)
}

fn write_page_header(data: &mut [u8], page_index: usize, sequence: u32, state: u32) {
    let offset = page_index * FLASH_SECTOR_SIZE;

    // Write state
    data[offset..offset + 4].copy_from_slice(&state.to_le_bytes());

    // Write sequence number
    data[offset + 4..offset + 8].copy_from_slice(&sequence.to_le_bytes());

    // Write version (0xFE for NVS format - used by ESP-IDF)
    data[offset + 8] = 0xFE;

    // Reserved bytes (19 bytes) are already 0xFF

    // Calculate and write CRC32
    let crc = crc32(&data[offset + 4..offset + 28]);
    data[offset + 28..offset + 32].copy_from_slice(&crc.to_le_bytes());
}

fn write_key(dest: &mut [u8], key: &str) -> Result<(), Error> {
    validate_key(key)?;

    let key_bytes = key.as_bytes();
    dest[..key_bytes.len()].copy_from_slice(key_bytes);
    // Null-terminate and zero-fill the rest (ESP-IDF format uses zeros, not 0xFF)
    dest[key_bytes.len()..16].fill(0);

    Ok(())
}

fn set_entry_state(data: &mut [u8], page_index: usize, entry_index: usize, state: u8) {
    let page_offset = page_index * FLASH_SECTOR_SIZE;
    let bitmap_offset = page_offset + PAGE_HEADER_SIZE;

    let byte_index = entry_index / 4;
    let bit_offset = (entry_index % 4) * 2;

    let mut byte = data[bitmap_offset + byte_index];
    byte &= !(0b11 << bit_offset); // Clear the 2 bits
    byte |= state << bit_offset; // Set the state
    data[bitmap_offset + byte_index] = byte;
}

fn parse_file_content(content: &[u8], encoding: &FileEncoding) -> Result<DataValue, Error> {
    match encoding {
        FileEncoding::String => {
            let s = std::str::from_utf8(content)
                .map_err(|e| Error::InvalidValue(format!("invalid UTF-8 in file: {}", e)))?;
            Ok(DataValue::String(s.to_string()))
        }
        FileEncoding::Hex2Bin => {
            let hex_str = std::str::from_utf8(content)
                .map_err(|e| Error::InvalidValue(format!("invalid UTF-8 in hex file: {}", e)))?;
            let bytes = hex::decode(hex_str.trim())?;
            Ok(DataValue::Binary(bytes))
        }
        FileEncoding::Base64 => {
            let b64_str = std::str::from_utf8(content)
                .map_err(|e| Error::InvalidValue(format!("invalid UTF-8 in base64 file: {}", e)))?;
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64_str.trim())?;
            Ok(DataValue::Binary(bytes))
        }
        FileEncoding::Binary => Ok(DataValue::Binary(content.to_vec())),
    }
}
