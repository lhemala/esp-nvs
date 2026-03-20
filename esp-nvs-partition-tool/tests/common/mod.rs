use std::fs;

use esp_nvs_partition_tool::{
    EntryContent,
    NvsPartition,
};

/// Read a CSV file and parse it into an `NvsPartition`, resolving relative
/// file paths against the CSV file's parent directory.
pub fn read_csv_file(csv_path: &str) -> NvsPartition {
    let content = fs::read_to_string(csv_path).unwrap();
    let mut partition = NvsPartition::try_from_str(&content).unwrap();

    if let Some(base) = std::path::Path::new(csv_path).parent() {
        for entry in &mut partition.entries {
            if let EntryContent::File { file_path, .. } = &mut entry.content {
                if file_path.is_relative() {
                    *file_path = base.join(&file_path);
                }
            }
        }
    }

    partition
}
