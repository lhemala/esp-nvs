# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.0] - 2026-10-05

### Features

- *(esp-nvs)* Migrate legacy single-page blobs when opening a partition

### Bug Fixes

- *(esp-nvs)* Reject inconsistent multi-page blobs
- *(esp-nvs)* Refuse blobs needing a 128th chunk
- *(esp-nvs)* Pin the largest storable blob to MAX_BLOB_SIZE - 1
- *(esp-nvs)* Bound the retries when writing a blob
- *(esp-nvs)* Replace a value whatever type the key held
- *(esp-nvs)* Report a zero sized string instead of panicking
- *(esp-nvs)* Bound a referenced data read to its own page
- *(esp-nvs)* Survive an impossible span while scanning a page
- *(esp-nvs)* Keep the active page when a write gives up
- *(esp-nvs)* Mark an entry with an impossible span as erased
- *(esp-nvs)* Count a torn variable sized item once while scanning
- *(esp-nvs)* Never write an item past the end of its page
- *(esp-nvs)* Report a partition without a reserve page instead of panicking
- *(esp-nvs)* Skip items of an unknown namespace while iterating
- *(esp-nvs)* Restart an interrupted defragmentation from scratch
- *(esp-nvs)* Erase what a torn write leaves behind while scanning
- *(esp-nvs)* Never reinterpret an unknown type byte as an item type
- *(esp-nvs)* Keep the newer of two blob versions at boot
- *(esp-nvs)* Resolve a key left holding a blob and another value
- *(esp-nvs)* Remove the chunks of a blob write that failed
- *(esp-nvs)* Put the active page back when a write fails
- *(esp-nvs)* Let a corrupt blob be replaced by writing it again
- *(esp-nvs)* Copy items byte for byte when defragmenting
- *(esp-nvs)* Do not defragment a page that has nothing to reclaim
- *(esp-nvs)* Stop at 254 namespaces
- *(esp-nvs)* List every blob in keys()
- *(esp-nvs)* Stop Key::as_str from building invalid UTF-8
- *(esp-nvs)* Write a blob index without uninitialized bytes
- *(esp-nvs)* Report an out of range MemFlash access as an error
- *(esp-nvs)* Align by the alignment, not by the size
- *(esp-nvs)* Erase an active page at boot only when it is a partial copy
- *(esp-nvs)* Erase an item whose payload was never marked written

### Other

- Run the partition tool through cargo instead of devenv

### Documentation

- *(esp-nvs)* Record what a successful defragment does not promise
- *(esp-nvs)* Update what a failed blob write leaves behind

### Testing

- *(esp-nvs)* Cover CorruptedData branches for blobs
- *(esp-nvs)* Cover purging a blob that spans pages
- *(esp-nvs)* Pin the chunk base alternation between blob versions
- *(esp-nvs)* Cover the string length limit
- *(esp-nvs)* Pin the support for legacy single-page blobs
- *(esp-nvs)* Regenerate test_nvs_data.bin with the current generator


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
