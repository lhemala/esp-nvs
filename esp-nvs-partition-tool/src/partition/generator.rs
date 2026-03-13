use std::fs::read;

use base64::Engine;
use esp_nvs::mem_flash::MemFlash;
use esp_nvs::{
    Key,
    Nvs,
};

use super::{
    DataValue,
    EntryContent,
    FileEncoding,
};
use crate::NvsPartition;
use crate::error::Error;

/// Generate an NVS partition binary in memory and return it as a `Vec<u8>`.
///
/// `size` must be a multiple of 4096 (the ESP-IDF flash sector size).
pub(crate) fn generate_partition_data(
    partition: &NvsPartition,
    size: usize,
) -> Result<Vec<u8>, Error> {
    if size < esp_nvs::FLASH_SECTOR_SIZE {
        return Err(Error::PartitionTooSmall(size));
    } else if !size.is_multiple_of(esp_nvs::FLASH_SECTOR_SIZE) {
        return Err(Error::InvalidPartitionSize(size));
    }

    let pages = size / esp_nvs::FLASH_SECTOR_SIZE;
    let flash = MemFlash::new(pages);
    let mut nvs = Nvs::new(0, size, flash)?;

    for entry in &partition.entries {
        let namespace = Key::from_str(&entry.namespace);
        let key = Key::from_str(&entry.key);

        // Resolve the value from the entry content.
        // For file entries, read the file and convert to a DataValue at generation
        // time.
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

        match value {
            DataValue::U8(v) => nvs.set(&namespace, &key, *v)?,
            DataValue::I8(v) => nvs.set(&namespace, &key, *v)?,
            DataValue::U16(v) => nvs.set(&namespace, &key, *v)?,
            DataValue::I16(v) => nvs.set(&namespace, &key, *v)?,
            DataValue::U32(v) => nvs.set(&namespace, &key, *v)?,
            DataValue::I32(v) => nvs.set(&namespace, &key, *v)?,
            DataValue::U64(v) => nvs.set(&namespace, &key, *v)?,
            DataValue::I64(v) => nvs.set(&namespace, &key, *v)?,
            DataValue::String(s) => nvs.set(&namespace, &key, s.as_str())?,
            DataValue::Binary(b) => nvs.set(&namespace, &key, b.as_slice())?,
        }
    }

    Ok(nvs.into_inner().into_inner())
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
