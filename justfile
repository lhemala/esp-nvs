fix:
    cargo fmt
    cargo clippy --fix --allow-dirty --allow-staged --release --features=defmt
    cargo fmt

lint:
    cargo clippy --release --features=defmt -- -D warnings
    cargo fmt --check

test:
    cargo test

publish-dry-run:
    cargo publish --registry crates-io --dry-run

publish:
    cargo publish --registry crates-io

[working-directory: 'tests/assets/']
generate_test_nvs_bin:
    nvs_partition_gen generate test_nvs_data.csv test_nvs_data.bin 0x4000
