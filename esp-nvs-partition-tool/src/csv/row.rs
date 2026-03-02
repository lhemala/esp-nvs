#[derive(Debug, serde::Serialize)]
pub(crate) struct PartitionRow {
    key: String,
    r#type: Type,
    encoding: String,
    value: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum Type {
    Data,
    File,
}

impl PartitionRow {
    fn new(key: String, r#type: Type, encoding: String, value: String) -> Self {
        Self {
            key,
            r#type,
            encoding,
            value,
        }
    }
}

impl From<crate::NvsEntry> for PartitionRow {
    fn from(entry: crate::NvsEntry) -> Self {
        let r#type = Type::from(&entry.content);

        match entry.content {
            crate::EntryContent::Data(value) => PartitionRow::new(
                entry.key.to_owned(),
                r#type,
                value.encoding_str().to_string(),
                value.to_string(),
            ),
            crate::EntryContent::File {
                encoding,
                file_path,
            } => PartitionRow::new(
                entry.key.to_owned(),
                r#type,
                encoding.as_str().to_owned(),
                file_path.to_string_lossy().to_string(),
            ),
        }
    }
}

impl From<&crate::EntryContent> for Type {
    fn from(content: &crate::EntryContent) -> Self {
        match content {
            crate::EntryContent::Data(_) => Self::Data,
            crate::EntryContent::File { .. } => Self::File,
        }
    }
}
