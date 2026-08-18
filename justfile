mod nvs 'esp-nvs/justfile'
mod partition_tool 'esp-nvs-partition-tool/justfile'

_default:
    @just --list

fix: nvs::fix partition_tool::fix
    cargo fmt

fmt-all: fmt
    just --unstable --format
    nixfmt devenv.nix
    nixfmt .nix/esp-nvs-partition-tool.nix

fmt: _nightly-fmt

lint: _nightly-fmt-check nvs::lint partition_tool::lint

test:
    cargo test --all
    cargo test -p esp-nvs --features encryption
    cargo test --doc

update-changelog: nvs::update-changelog partition_tool::update-changelog

# Bump the esp-nvs library: prepend changelog, set version, refresh lock.
bump-lib version:
    git-cliff --unreleased --tag {{version}} --include-path "esp-nvs/**" --prepend esp-nvs/CHANGELOG.md
    sed -i '0,/^version = ".*"/s//version = "{{version}}"/' esp-nvs/Cargo.toml
    cargo check -p esp-nvs

# Bump the esp-nvs-partition-tool: prepend changelog, set version, refresh lock.
bump-tool version:
    git-cliff --unreleased --tag {{version}} --include-path "esp-nvs-partition-tool/**" --prepend esp-nvs-partition-tool/CHANGELOG.md
    sed -i '0,/^version = ".*"/s//version = "{{version}}"/' esp-nvs-partition-tool/Cargo.toml
    cargo check -p esp-nvs-partition-tool

_nightly-fmt:
    devenv shell \
        --option languages.rust.version:string 2026-02-18 \
        --option languages.rust.channel:string nightly \
        cargo fmt --all

_nightly-fmt-check:
    devenv shell \
        --option languages.rust.version:string 2026-02-18 \
        --option languages.rust.channel:string nightly \
        cargo fmt --all --check
