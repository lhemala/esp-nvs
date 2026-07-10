# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.0] - 2026-07-10

### Features

- Implement data physical purging

### Bug Fixes

- *(esp-nvs)* Simplify the defmt Format implementation
- Assert key characters are within the ascii range
- *(types)* Avoid problematic defmt symbol in key implementation
- *(esp-nvs: docs)* Fix path to readme

### Other

- Relax esp-hal dependency version requirement
- Relax esp-storage dependency version requirement

### Refactor

- *(esp-nvs)* Split code into more atomic modules


## [0.4.0] - 2026-03-26

### Features

- Add esp nvs partition tool
- Expose `pub mod raw` and `pub mod mem_flash` for low-level access
- Re-export raw constants and types at crate root: `ENTRIES_PER_PAGE`, `ENTRY_STATE_BITMAP_SIZE`, `FLASH_SECTOR_SIZE`, `ITEM_SIZE`, `ItemType`, `MAX_BLOB_DATA_PER_PAGE`, `MAX_BLOB_SIZE`, `PAGE_HEADER_SIZE`, `PageState`
- Make `MAX_KEY_LENGTH` a public constant
- Add `Key::as_str()` to retrieve the key as a string slice without null padding
- Add `Nvs::typed_entries()` to iterate over all data entries with their `ItemType`

### Refactor

- Introduce workspace and rustfmt configuration


## [0.3.0] - 2026-02-27

### Features

- Allow iterating over namespaces and keys

### Other

- Add default target for just

### Refactor

- [**breaking**] Display `Key` values in `defmt::Format` as binary string


## [0.2.0] - 2026-01-09

### Features

- Expose Get/Set trait to be extended by users

### Bug Fixes

- Implement error trait for nvs error
- Ensure correct active page placement in self.pages on nvs init

### Other

- *(nix)* Include riscv32{imc,imac}-unknown-none-elf rust toolchain
- Update esp-hal to v1.0.0

### Refactor

- [**breaking**] Require an owned Platform to be passed to EspNvs
- [**breaking**] Allow direct usage of flashstorage from esp-storage
- [**breaking**] Display `Key` values in `Debug` as binary string

### Documentation

- Remove unnecessary flash clone in esp-hal example

### Testing

- Cast crc32 init value as c_ulong
- Use pretty-assertions

### Miscellaneous Tasks

- Add github workflows


## [0.1.3] - 2025-12-14

### Bug Fixes

- Write_aligned fails when buf.len() < T::WRITE_SIZE


## [0.1.2] - 2025-12-10

### Bug Fixes

- Bool get returning always true

### Other

- Tune git-cliff config so there are two newlines between versions


## [0.1.1] - 2025-11-21

### Bug Fixes

- Fix broken debug logs in tests
- Fix overwriting blobs multiple times

### Miscellaneous Tasks

- Fix lint and typo in tests


## [0.1.0] - 2025-11-21

### Features

- Add trace logs to facilitate debugging on actual hardware

### Bug Fixes

- Expose key internal representation
- Ensure that the flash access is aligned
- Invalid item indices in NVS initialization

### Other

- Add defmt feature to linter recipe
- Add git-cliff config
