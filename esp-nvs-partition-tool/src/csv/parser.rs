use std::path::Path;

use base64::Engine;

use crate::NvsPartition;
use crate::error::Error;
use crate::partition::{
    DataValue,
    FileEncoding,
    NvsEntry,
    validate_key,
};

#[derive(Debug)]
struct CsvRow {
    key: String,
    entry_type: String,
    encoding: String,
    value: String,
}

/// Parse NVS CSV content from a string into an [`NvsPartition`].
pub(crate) fn parse_csv(content: &str) -> Result<NvsPartition, Error> {
    let mut partition = NvsPartition { entries: vec![] };
    // Like ESP-IDF's generator: lines starting with `#` are comments, and rows may be shorter than
    // the header.
    let mut reader = csv::ReaderBuilder::new()
        .comment(Some(b'#'))
        .flexible(true)
        .from_reader(content.as_bytes());
    let mut current_namespace: Option<String> = None;

    // Columns are looked up by name, as ESP-IDF does, and a field a row stops short of is empty: a
    // namespace row may end after its type, as `ns,namespace`.
    let headers = reader.headers()?.clone();
    let column = |name: &str| headers.iter().position(|header| header == name);
    let (key, entry_type, encoding, value) = (column("key"), column("type"), column("encoding"), column("value"));
    if key.is_none() || entry_type.is_none() {
        return Err(Error::InvalidValue(
            "the CSV header has to name at least the columns key and type".to_string(),
        ));
    }

    for result in reader.records() {
        let record = result?;
        let field = |index: Option<usize>| index.and_then(|index| record.get(index)).unwrap_or("").to_string();
        let mut row = CsvRow {
            key: field(key),
            entry_type: field(entry_type),
            encoding: field(encoding),
            value: field(value),
        };
        // ESP-IDF tolerates spaces around the type and encoding, and encodings in any case. Keys
        // and values are taken as written, spaces included, apart from numbers below.
        row.entry_type = row.entry_type.trim().to_string();
        row.encoding = row.encoding.trim().to_ascii_lowercase();

        if row.entry_type == "namespace" {
            validate_key(&row.key)?;
            if !row.encoding.is_empty() || !row.value.is_empty() {
                return Err(Error::InvalidValue(
                    "namespace entries must have empty encoding and value".to_string(),
                ));
            }
            current_namespace = Some(row.key);
            continue;
        }

        let namespace = current_namespace.clone().ok_or(Error::MissingNamespace)?;
        let entry = parse_row(row, namespace)?;
        partition.entries.push(entry);
    }

    Ok(partition)
}

fn parse_row(row: CsvRow, namespace: String) -> Result<NvsEntry, Error> {
    validate_key(&row.key)?;

    match row.entry_type.as_str() {
        "data" => {
            if row.encoding.is_empty() {
                return Err(Error::InvalidEncoding("data entries must have an encoding".to_string()));
            }
            let value = parse_value(&row.value, &row.encoding)?;
            Ok(NvsEntry::new_data(namespace, row.key, value))
        }
        "file" => {
            if row.value.is_empty() {
                return Err(Error::InvalidValue("file entries must have a file path".to_string()));
            }
            let encoding: FileEncoding = row.encoding.parse()?;
            let file_path = Path::new(&row.value).to_path_buf();
            Ok(NvsEntry::new_file(namespace, row.key, encoding, file_path))
        }
        _ => Err(Error::InvalidType(row.entry_type)),
    }
}

macro_rules! parse_numeric {
    ($value:expr, $ty:ty, $variant:ident) => {
        $value
            .trim()
            .parse::<$ty>()
            .map(DataValue::$variant)
            .map_err(|e| Error::InvalidValue(format!("invalid {} value: {}", stringify!($ty), e)))
    };
}

fn parse_value(value: &str, encoding: &str) -> Result<DataValue, Error> {
    match encoding {
        "u8" => parse_numeric!(value, u8, U8),
        "i8" => parse_numeric!(value, i8, I8),
        "u16" => parse_numeric!(value, u16, U16),
        "i16" => parse_numeric!(value, i16, I16),
        "u32" => parse_numeric!(value, u32, U32),
        "i32" => parse_numeric!(value, i32, I32),
        "u64" => parse_numeric!(value, u64, U64),
        "i64" => parse_numeric!(value, i64, I64),
        "string" => Ok(DataValue::String(value.to_string())),
        "hex2bin" => {
            let bytes = hex::decode(value.trim())?;
            Ok(DataValue::Binary(bytes))
        }
        "base64" => {
            let bytes = base64::engine::general_purpose::STANDARD.decode(value.trim())?;
            Ok(DataValue::Binary(bytes))
        }
        _ => Err(Error::InvalidEncoding(encoding.to_string())),
    }
}
