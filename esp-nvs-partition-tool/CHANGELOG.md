# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2026-10-06


## [0.2.0] - 2026-10-05

### Bug Fixes

- *(esp-nvs-partition-tool)* Build against the esp-nvs of this workspace
- *(esp-nvs-partition-tool)* Validate every key before generating
- *(esp-nvs-partition-tool)* Report more than 254 namespaces as such
- *(esp-nvs-partition-tool)* Parse CSV files the way ESP-IDF does
- *(esp-nvs-partition-tool)* Reject partition sizes the library cannot use
- *(esp-nvs-partition-tool)* Print errors for people, with the file at fault
- *(esp-nvs-partition-tool)* Take input with a byte order mark for CSV
- *(esp-nvs-partition-tool)* Reject a key that is not UTF-8 when parsing
- *(esp-nvs-partition-tool)* Accept binary data rows and wrapped base64


## [0.1.1] - 2026-07-10


## [0.1.0] - 2026-03-26

### Features

- Initial release
