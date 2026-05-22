//! Crate-internal typed errors. The binary edge (`main`) uses `anyhow`; library
//! modules use [`Result`] so failures carry a precise variant.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("git error: {0}")]
    Git(#[from] git2::Error),

    #[error("could not parse review state: {0}")]
    ReviewDecode(#[from] toml::de::Error),

    #[error("could not encode review state: {0}")]
    ReviewEncode(#[from] toml::ser::Error),

    #[error("{0}")]
    Other(String),
}

/// Convenience alias used throughout the library modules.
pub type Result<T> = std::result::Result<T, Error>;
