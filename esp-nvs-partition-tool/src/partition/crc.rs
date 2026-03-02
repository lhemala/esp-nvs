/// Compute an NVS entry CRC over all bytes except the CRC field at offset 4..8.
///
/// # Panics
/// Panics if `entry_data` is shorter than 32 bytes.
pub fn crc32_entry(entry_data: &[u8]) -> u32 {
    assert!(
        entry_data.len() >= 32,
        "crc32_entry requires at least 32 bytes, got {}",
        entry_data.len()
    );
    let mut combined = [0u8; 28];
    combined[..4].copy_from_slice(&entry_data[0..4]);
    combined[4..].copy_from_slice(&entry_data[8..32]);
    crc32(&combined)
}

/// CRC32 using the IEEE 802.3 polynomial (0xEDB88320, bit-reversed 0x04C11DB7).
///
/// This matches the CRC32 algorithm used by ESP-IDF for NVS entry and page
/// header checksums.
///
/// This function is intentionally public so that callers can verify or compute
/// CRCs over NVS data independently of the higher-level partition APIs.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;

    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }

    !crc
}
