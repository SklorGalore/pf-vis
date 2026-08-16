//! Reader for PSS/E raw (`.RAW`) power flow exchange files.
//!
//! [`scan`] handles the file-level mechanics — decoding, field splitting, section tracking —
//! and [`raw`] turns the resulting records into typed data.

pub mod raw;
pub mod scan;

pub use raw::RawCase;
pub use scan::PsseError;

/// Parse a raw file from disk.
pub fn parse_file(path: &std::path::Path) -> Result<RawCase, PsseError> {
    let bytes = std::fs::read(path)?;
    raw::parse(&scan::decode_latin1(&bytes))
}
