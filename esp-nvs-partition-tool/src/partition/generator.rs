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
    validate_key,
};
use crate::NvsPartition;
use crate::error::Error;

/// Generate an NVS partition binary in memory and return it as a `Vec<u8>`.
///
/// `size` must be a multiple of 4096 (the ESP-IDF flash sector size).
pub(crate) fn generate_partition_data(partition: &NvsPartition, size: usize) -> Result<Vec<u8>, Error> {
    // Writing takes a page to write to and one kept in reserve; with a single page every write
    // failed with `FlashFull`. ESP-IDF asks for at least three pages.
    if size < 2 * esp_nvs::FLASH_SECTOR_SIZE {
        return Err(Error::PartitionTooSmall(size));
    }
    // Checked before the in-memory flash is allocated: the library rejects more than u16::MAX
    // pages, but a size far beyond that ran out of memory first.
    if !size.is_multiple_of(esp_nvs::FLASH_SECTOR_SIZE) || size / esp_nvs::FLASH_SECTOR_SIZE > u16::MAX as usize {
        return Err(Error::InvalidPartitionSize(size));
    }

    let pages = size / esp_nvs::FLASH_SECTOR_SIZE;
    let flash = MemFlash::new(pages);
    let mut nvs = Nvs::new(0, size, flash)?;

    // Entries built through the API have not been through the CSV parser's checks.
    for entry in &partition.entries {
        validate_key(&entry.namespace)?;
        validate_key(&entry.key)?;
    }

    for entry in &partition.entries {
        let namespace = Key::from_str(&entry.namespace);
        let key = Key::from_str(&entry.key);

        // Resolve the value from the entry content.
        // For file entries, read the file and convert to a DataValue at generation
        // time.
        let resolved_value;
        let value = match &entry.content {
            EntryContent::Data(val) => val,
            EntryContent::File { encoding, file_path } => {
                // The bare I/O error does not say which of the files it is about.
                let content = read(file_path)
                    .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", file_path.display())))?;
                resolved_value = parse_file_content(&content, encoding)?;
                &resolved_value
            }
        };

        match value {
            DataValue::U8(v) => nvs.set(&namespace, &key, *v).map_err(map_nvs_error)?,
            DataValue::I8(v) => nvs.set(&namespace, &key, *v).map_err(map_nvs_error)?,
            DataValue::U16(v) => nvs.set(&namespace, &key, *v).map_err(map_nvs_error)?,
            DataValue::I16(v) => nvs.set(&namespace, &key, *v).map_err(map_nvs_error)?,
            DataValue::U32(v) => nvs.set(&namespace, &key, *v).map_err(map_nvs_error)?,
            DataValue::I32(v) => nvs.set(&namespace, &key, *v).map_err(map_nvs_error)?,
            DataValue::U64(v) => nvs.set(&namespace, &key, *v).map_err(map_nvs_error)?,
            DataValue::I64(v) => nvs.set(&namespace, &key, *v).map_err(map_nvs_error)?,
            DataValue::String(s) => nvs.set(&namespace, &key, s.as_str())?,
            DataValue::Binary(b) => nvs.set(&namespace, &key, b.as_slice())?,
        }
    }

    Ok(nvs.into_inner().into_inner())
}

/// Reports what the library says about namespaces as this crate's own error.
fn map_nvs_error(e: esp_nvs::error::Error) -> Error {
    match e {
        esp_nvs::error::Error::TooManyNamespaces => Error::TooManyNamespaces,
        e => Error::NvsError(e),
    }
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
