use std::path::Path;

use csv::WriterBuilder;

use crate::NvsPartition;
use crate::csv::row::PartitionRow;
use crate::error::Error;

/// Serialize an NVS partition to a CSV file at the given `output_path`.
///
/// Entries are written in their original insertion order. A namespace header
/// row is emitted whenever the namespace changes between consecutive entries.
///
/// `Binary` data values are serialized as base64, matching the ESP-IDF
/// `nvs_partition_tool` convention.
pub(crate) fn write_csv<P: AsRef<Path>>(
    partition: NvsPartition,
    output_path: P,
) -> Result<(), Error> {
    let mut wtr = WriterBuilder::new()
        .has_headers(false)
        .from_path(output_path)?;
    write_records(&mut wtr, partition)
}

/// Serialize an NVS partition to CSV and return the content as a `String`.
///
/// See [`write_csv`] for details on ordering and encoding behavior.
pub(crate) fn write_csv_content(partition: NvsPartition) -> Result<String, Error> {
    let mut wtr = WriterBuilder::new()
        .has_headers(false)
        .from_writer(Vec::new());

    write_records(&mut wtr, partition)?;

    let bytes = wtr
        .into_inner()
        .map_err(|e| Error::IoError(e.into_error()))?;
    String::from_utf8(bytes)
        .map_err(|e| Error::InvalidValue(format!("CSV output is not valid UTF-8: {}", e)))
}

fn write_records<W: std::io::Write>(
    wtr: &mut csv::Writer<W>,
    partition: NvsPartition,
) -> Result<(), Error> {
    wtr.write_record(["key", "type", "encoding", "value"])?;

    // Emit namespace rows on demand, preserving the original entry order.
    let mut current_namespace: Option<String> = None;

    for entry in partition.entries {
        // Emit a namespace row when the namespace changes
        if current_namespace.as_deref() != Some(&entry.namespace) {
            let namespace = entry.namespace.clone();
            wtr.write_record([&namespace, "namespace", "", ""])?;
            current_namespace = Some(namespace);
        }

        wtr.serialize(PartitionRow::from(entry))?;
    }

    wtr.flush()?;
    Ok(())
}
