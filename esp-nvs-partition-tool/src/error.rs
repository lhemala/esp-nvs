use thiserror::Error;

/// Errors that can occur during CSV parsing, binary generation, or binary
/// parsing of NVS partitions.
#[derive(Error, Debug)]
pub enum Error {
    #[error("failed to parse CSV: {0}")]
    CsvError(#[from] csv::Error),

    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("invalid entry type: {0}")]
    InvalidType(String),

    #[error("invalid encoding: {0}")]
    InvalidEncoding(String),

    #[error("invalid value: {0}")]
    InvalidValue(String),

    #[error("hex decoding error: {0}")]
    HexError(#[from] hex::FromHexError),

    #[error("base64 decoding error: {0}")]
    Base64Error(#[from] base64::DecodeError),

    #[error("missing namespace")]
    MissingNamespace,

    #[error("invalid key: {0}")]
    InvalidKey(String),

    #[error("partition size {0} is too small")]
    PartitionTooSmall(usize),

    #[error("invalid partition size {0}: must be a multiple of 4096 bytes")]
    InvalidPartitionSize(usize),

    #[error("too many namespaces (max 255)")]
    TooManyNamespaces,
}
