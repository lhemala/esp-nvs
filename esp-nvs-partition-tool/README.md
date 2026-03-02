# esp-nvs-partition-tool

ESP-IDF compatible NVS (Non-Volatile Storage) partition table parser and generator inspired by [esp-idf-nvs-partition-gen](https://github.com/espressif/esp-idf/tree/v5.5.3/components/nvs_flash/nvs_partition_tool).

This library and CLI tool allows you to parse and generate NVS partition binary files from CSV files, following the [ESP-IDF NVS partition format specification](https://docs.espressif.com/projects/esp-idf/en/stable/esp32c6/api-reference/storage/nvs_partition_gen.html#nvs-partition-generator-utility).

## TODO

- [ ] Encryption support (planned for future release)

## CSV Format

The CSV file must have exactly four columns:

```csv
key,type,encoding,value
```

### Entry Types

1. **namespace** - Defines a namespace
   - Encoding and value must be empty
   - Example: `my_namespace,namespace,,`

2. **data** - Raw data entry
   - Valid encodings: `u8`, `i8`, `u16`, `i16`, `u32`, `i32`, `u64`, `i64`, `string`, `hex2bin`, `base64`
   - Example: `my_key,data,u32,12345`

3. **file** - Read value from a file
   - Valid encodings: `string`, `hex2bin`, `base64`, `binary`
   - Value should be the file path (relative to CSV file)
   - Example: `my_blob,file,binary,data.bin`

### Example CSV

```csv
key,type,encoding,value
namespace_one,namespace,,
example_u8,data,u8,100
example_i8,data,i8,-100
example_string,data,string,Hello World
example_blob,data,hex2bin,AABBCCDDEE
namespace_two,namespace,,
config,file,binary,config.bin
```

## CLI Usage

### Generate NVS Partition Binary

```bash
esp-nvs-partition-tool generate <input.csv> <output.bin> --size <size>
```

The size can be specified in decimal or hexadecimal (with `0x` prefix):

```bash
# Generate 16KB partition (decimal)
esp-nvs-partition-tool generate nvs_data.csv partition.bin --size 16384

# Generate 16KB partition (hexadecimal)
esp-nvs-partition-tool generate nvs_data.csv partition.bin --size 0x4000
```

### Parse NVS Partition Binary to CSV

```bash
esp-nvs-partition-tool parse <input.bin> <output.csv>
```

Example:

```bash
# Parse a partition binary back to CSV
esp-nvs-partition-tool parse partition.bin recovered_data.csv
```

## Library Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
esp-nvs-partition-tool = "0.1.0"
```

Example usage:

```rust
use esp_nvs_partition_tool::NvsPartition;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse CSV file and generate binary
    let partition = NvsPartition::from_csv_file("nvs_data.csv")?;
    partition.generate_partition_file("output.bin", 16384)?;

    // Parse binary back to CSV
    let recovered_partition = NvsPartition::parse_partition_file("output.bin")?;
    recovered_partition.to_csv_file("recovered.csv")?;

    Ok(())
}
```

## References

- [ESP-IDF NVS Partition Generator Documentation](https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/storage/nvs_partition_gen.html)
- [ESP-IDF NVS Flash Documentation](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/storage/nvs_flash.html)
- [esp-idf-part](https://github.com/esp-rs/esp-idf-part) - Reference implementation for partition tables
