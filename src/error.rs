//! Crate-internal typed errors. The binary edge (`main`) uses `anyhow`; library
//! modules use [`Result`] so failures carry a precise variant.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience alias used throughout the library modules.
pub type Result<T> = std::result::Result<T, Error>;
