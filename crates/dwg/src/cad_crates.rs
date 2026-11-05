//! DWG through WithShalom's open DWG and DXF crates (`cad-dwg`, `cad-dxf`, MIT OR Apache-2.0,
//! <https://github.com/withshalom/blackcad>): the engine this crate tries first. They read DWG from R13
//! to 2018 and later except 2007 (AC1021), write AutoCAD 2018 (AC1032) DWG, and count what a
//! conversion leaves out. They use the standard library alone, so they also run in the web build.
//!
//! A conversion here is [`Outcome::Whole`] only when nothing was left out and nothing failed to
//! decode; anything else is [`Outcome::Partial`], with a note for each loss, and the caller tries
//! acadrust before settling for it.

use std::panic::{AssertUnwindSafe, catch_unwind};

use cad_dwg::cad_model::model::{Diagnostic, Drawing, Severity};
use cad_dxf::DxfFormat;

use super::{Limits, mb};

/// What a conversion produced.
pub(crate) enum Outcome {
    /// The file, with everything carried over.
    Whole(Vec<u8>),
    /// The file, and what it left out (one note each).
    Partial(Vec<u8>, Vec<String>),
}

/// Why a conversion produced nothing.
pub(crate) enum Failure {
    /// The DXF rendition passed `Limits::max_dxf_bytes`: the same for any engine.
    Limit(String),
    /// the open engine can't convert this file (a version it doesn't read, damage, a fault).
    Failed(String),
}

/// The program name and version the open engine's writer records in the DWG files CADCraft saves.
pub(crate) const APP_NAME: &str = "CADCraft";
pub(crate) const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Read DWG bytes into an ASCII DXF (AutoCAD 2018 format).
pub(crate) fn dwg_to_dxf(bytes: &[u8], limits: Limits) -> Result<Outcome, Failure> {
    let d = catch_unwind(|| cad_dwg::read_bytes(bytes))
        .map_err(|_| Failure::Failed("The open engine's DWG reader failed on this file".into()))?
        .map_err(|e| Failure::Failed(format!("DWG: {e}")))?;
    let out = catch_unwind(AssertUnwindSafe(|| cad_dxf::to_bytes(&d, DxfFormat::Ascii)))
        .map_err(|_| Failure::Failed("The open engine's DWG→DXF conversion failed on this file".into()))?;
    if out.bytes.len() > limits.max_dxf_bytes {
        return Err(Failure::Limit(format!(
            "DWG: converting this {} file produced more than {} of drawing data (the limit); \
             the file is probably damaged, or uses objects the DWG reader misreads",
            mb(bytes.len() as u64),
            mb(limits.max_dxf_bytes as u64)
        )));
    }
    let mut losses = warnings(&d, "DWG");
    if let Some(note) = left_out_note(&out.left_out, "DWG", "couldn't be converted to DXF") {
        losses.push(note);
    }
    Ok(outcome(out.bytes, losses))
}

/// Convert DXF bytes into an AutoCAD 2018 (AC1032) DWG file.
pub(crate) fn dxf_to_dwg(dxf: &[u8]) -> Result<Outcome, Failure> {
    let d = catch_unwind(|| cad_dxf::read_bytes(dxf))
        .map_err(|_| Failure::Failed("The open engine's DXF reader failed on this drawing".into()))?
        .map_err(|e| Failure::Failed(format!("DXF: {e}")))?;
    let opts = cad_dwg::write::WriteOptions { app_name: APP_NAME, app_version: APP_VERSION, ..Default::default() };
    let out = catch_unwind(AssertUnwindSafe(|| cad_dwg::write::to_bytes_with(&d, &opts)))
        .map_err(|_| Failure::Failed("The open engine's DWG writer failed on this drawing".into()))?
        .map_err(|e| Failure::Failed(format!("DWG write: {e}")))?;
    let mut losses = warnings(&d, "DXF");
    if let Some(note) = left_out_note(&out.left_out, "DWG", "couldn't be written to the DWG") {
        losses.push(note);
        // Why, object by object: what the writer said about the first few.
        losses.extend(out.notes.iter().take(WRITER_NOTES).map(|n| format!("DWG writer: {n}")));
    }
    Ok(outcome(out.bytes, losses))
}

/// Most of the DWG writer's own notes passed on when it leaves objects out.
const WRITER_NOTES: usize = 8;

fn outcome(bytes: Vec<u8>, losses: Vec<String>) -> Outcome {
    if losses.is_empty() { Outcome::Whole(bytes) } else { Outcome::Partial(bytes, losses) }
}

/// The reader's warnings and errors: objects that failed to decode, records left out, damage it
/// read around. Its information notes (what it filled in) are not losses.
pub(crate) fn warnings(d: &Drawing, format: &str) -> Vec<String> {
    d.diagnostics
        .iter()
        .filter(|x: &&Diagnostic| matches!(x.severity, Severity::Warning | Severity::Error))
        .map(|x| format!("{format}: {}", x.message))
        .collect()
}

/// Most object types named in a note.
const NOTE_TYPES: usize = 6;

/// The note on the objects a conversion of a `format` file left out, by type (most first), or
/// nothing.
pub(crate) fn left_out_note(left_out: &std::collections::BTreeMap<String, usize>, format: &str, what: &str) -> Option<String> {
    let total: usize = left_out.values().sum();
    if total == 0 {
        return None;
    }
    let mut types: Vec<(&String, &usize)> = left_out.iter().collect();
    types.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let mut list: Vec<String> = types.iter().take(NOTE_TYPES).map(|(t, n)| format!("{t} × {n}")).collect();
    if types.len() > NOTE_TYPES {
        list.push(format!("{} more types", types.len() - NOTE_TYPES));
    }
    let (objects, were) = if total == 1 { ("object", "was") } else { ("objects", "were") };
    Some(format!("{format}: {total} {objects} {what} and {were} left out ({})", list.join(", ")))
}
