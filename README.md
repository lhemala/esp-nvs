# esp-nvs

This is an ESP-IDF compatible, bare metal, non-volatile storage (NVS) library.

## Motivation

The motivation to write this library was
1. to have a backwards compatible NVS driver in case someone is upgrading existing systems from ESP-IDF.
2. to take advantage of the many people working at espressif, who probably spent a minute or two when designing
   the underlying, flash-friendly data structure.

Therefore, the code is based on the [NVS documentation from espressif](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/storage/nvs_flash.html)
and inspired by the [actual C++ implementation](https://github.com/espressif/esp-idf/tree/master/components/nvs_flash).

## Compatibility

Blobs are written in the multi-page format ESP-IDF uses since v4.0. Blobs in the older single-page format, as found on
partitions written by earlier ESP-IDF versions, are read, listed and kept, and are replaced by the multi-page format
when overwritten.

## Safety

Since the data structure on the flash is based on C structs, some `unsafe` blocks are used to transmute the read memory
to `#[repr(C,packed)]` structs and unions. Nevertheless, all data structures include CRC32 checksums which are checked and handled.
The use of unions requires unsafe as well.

## Status

This library is used in production, but nevertheless, there might be some kinks here and there.

## Example

esp-nvs requires an implementation of the `Platform` trait to provide access to flash storage and CRC32 (typically the
ESP32 ROM CRC32 peripheral).

When using `esp_storage::FlashStorage`, enable the matching chip feature on **both** `esp-storage` and `esp-nvs`. The
`esp-nvs` chip feature activates the `Crc` trait implementation for `FlashStorage` using the ROM CRC32 peripheral:

```toml
[dependencies]
esp-storage = { version = "0.8.1", features = ["esp32c6"] }
esp-nvs     = { version = "0.5.0", features = ["esp32c6"] }
```

If you prefer a pure software CRC32 (no ROM dependency), provide your own `impl Crc for FlashStorage<'_>` using
`esp_nvs::platform::software_crc32`.

```rust,ignore
// TODO: Update the offsets according to your partition table definition.
let partition_offset = 0x390000;
let partition_size = 0x32000;

let storage = esp_storage::FlashStorage::new(peripherals.FLASH);

let mut nvs =
    esp_nvs::Nvs::new(partition_offset, partition_size, storage).expect("failed to create nvs");
```
