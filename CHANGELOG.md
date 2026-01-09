# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
