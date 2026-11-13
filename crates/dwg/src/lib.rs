//! DraftCraft's DWG codec.
//!
//! - [`read`]: DWG files from R13 to AutoCAD 2018 and later into the [`records`] of
//!   their objects, which `draftcraft-io` maps straight into the drawing.
//! - [`write`]: AutoCAD 2018 (AC1032) DWG files from records.
//! - [`file`]: a DWG file opened to its parts (version, sections, the object list), for a reader
//!   that builds the drawing itself.
//! - [`bits`], [`crc`], [`lz77`]: the format's primitives, shared by both.
//! - [`acadrust`]: DWG through the acadrust crate (MPL-2.0, used unmodified), converted to and from
//!   DXF bytes: the fallback for what the reader and writer don't cover yet.
//!
//! The reader, writer and records come from BlackCAD (<https://github.com/withshalom/blackcad>,
//! MIT OR Apache-2.0), written from the Open Design Alliance's public DWG specification.
//! This crate depends on no other workspace crate.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod acadrust;
pub mod bits;
pub mod crc;
pub mod file;
pub mod lz77;
pub mod read;
pub mod records;
pub mod write;

pub use read::{DwgVersion, read_bytes};

/// DWG version codes we can write.
pub const VERSIONS: &[(&str, &str)] =
    &[("AC1015", "2000"), ("AC1018", "2004"), ("AC1021", "2007"), ("AC1024", "2010"), ("AC1027", "2013"), ("AC1032", "2018")];

/// True when the bytes look like a DWG file (`AC10xx` magic).
pub fn is_dwg(bytes: &[u8]) -> bool {
    bytes.len() > 6 && bytes.starts_with(b"AC10") && bytes.get(4..6).is_some_and(|v| v.iter().all(u8::is_ascii_digit))
}

/// The DWG version code of a file (e.g. "AC1032").
pub fn version(bytes: &[u8]) -> Option<String> {
    is_dwg(bytes).then(|| String::from_utf8_lossy(bytes.get(0..6).unwrap_or_default()).to_string())
}

/// Size limits for reading a DWG file. DWG is compressed and the DXF text it converts to is
/// several times larger (about 6x in our tests: a 10 MB DWG becomes about 60 MB of DXF), so
/// the limits are generous for real drawings. They exist because a damaged file, or one the reader
/// misreads, can make the DWG reader produce a drawing thousands of times larger than the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest DWG file read at all (checked before reading starts).
    pub max_dwg_bytes: usize,
    /// Largest DXF rendition of a DWG (checked while it is written).
    pub max_dxf_bytes: usize,
}

impl Limits {
    /// 512 MB of DWG, 1 GB of DXF (more than the DXF reader's group-code limit fills).
    pub const DEFAULT: Limits = Limits { max_dwg_bytes: 512 << 20, max_dxf_bytes: 1 << 30 };
}

/// A byte count in megabytes for messages ("9.9 MB", "1024 MB").
#[cfg_attr(no_acadrust, allow(dead_code))]
pub(crate) fn mb(n: u64) -> String {
    let m = n as f64 / f64::from(1u32 << 20);
    if m >= 100.0 { format!("{m:.0} MB") } else { format!("{m:.1} MB") }
}
