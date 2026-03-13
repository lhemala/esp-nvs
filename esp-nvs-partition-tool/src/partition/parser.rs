use std::fs;
use std::path::Path;

use esp_nvs::mem_flash::MemFlash;
use esp_nvs::{
    ItemType,
    Key,
    Nvs,
};

use super::{
    DataValue,
    NvsEntry,
};
use crate::NvsPartition;
use crate::error::Error;

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

    if !data.len().is_multiple_of(esp_nvs::FLASH_SECTOR_SIZE) {
        return Err(Error::InvalidValue(format!(
            "binary size {} is not a multiple of page size {}",
            data.len(),
            esp_nvs::FLASH_SECTOR_SIZE
        )));
    }

    let size = data.len();
    let flash = MemFlash::from_bytes(data.to_vec());
    let mut nvs = Nvs::new(0, size, flash)?;

    let mut entries = Vec::new();

    // Collect all typed entries first, then read values by type
    let typed: Vec<(Key, Key, ItemType)> = nvs.typed_entries().collect::<Result<Vec<_>, _>>()?;

    for (ns_key, entry_key, item_type) in typed {
        let namespace = ns_key.as_str().to_string();
        let key = entry_key.as_str().to_string();

        let value = match item_type {
            ItemType::U8 => DataValue::U8(nvs.get::<u8>(&ns_key, &entry_key)?),
            ItemType::I8 => DataValue::I8(nvs.get::<i8>(&ns_key, &entry_key)?),
            ItemType::U16 => DataValue::U16(nvs.get::<u16>(&ns_key, &entry_key)?),
            ItemType::I16 => DataValue::I16(nvs.get::<i16>(&ns_key, &entry_key)?),
            ItemType::U32 => DataValue::U32(nvs.get::<u32>(&ns_key, &entry_key)?),
            ItemType::I32 => DataValue::I32(nvs.get::<i32>(&ns_key, &entry_key)?),
            ItemType::U64 => DataValue::U64(nvs.get::<u64>(&ns_key, &entry_key)?),
            ItemType::I64 => DataValue::I64(nvs.get::<i64>(&ns_key, &entry_key)?),
            ItemType::Sized => DataValue::String(nvs.get::<String>(&ns_key, &entry_key)?),
            ItemType::BlobIndex | ItemType::BlobData | ItemType::Blob => {
                DataValue::Binary(nvs.get::<Vec<u8>>(&ns_key, &entry_key)?)
            }
            ItemType::Any => {
                return Err(Error::InvalidValue(format!(
                    "unexpected item type {:?} for key '{}'",
                    item_type, key
                )));
            }
        };

        entries.push(NvsEntry::new_data(namespace, key, value));
    }

    Ok(NvsPartition { entries })
}
