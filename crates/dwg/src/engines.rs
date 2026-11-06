//! Two DWG engines, tried in order, around the acadrust bridge in this crate's root:
//!
//! 1. **WithShalom's open DWG and DXF crates** (`cad-dwg`, `cad-dxf`, MIT OR Apache-2.0,
//!    [`crate::cad_crates`]), tried first. They read DWG from R13 to 2018 and later except 2007
//!    (AC1021), write AutoCAD 2018 (AC1032) DWG, and count what a conversion leaves out. They use the
//!    standard library alone, so the web build reads and writes DWG with them too.
//! 2. **acadrust**, through this crate's own functions ([`crate::dwg_to_dxf_noted`],
//!    [`crate::dxf_to_dwg_as`]), unchanged, for whatever the open engine doesn't convert whole.
//!
//! So a conversion never loses what acadrust would have kept: the open engine's result is used
//! when it is complete, acadrust's otherwise, and the open engine's partial one (with notes on what
//! it left out) only when acadrust can't convert the file at all.

use crate::cad_crates::{self, Failure, Outcome};
use crate::{Limits, VERSIONS, mb, unsupported};

/// The version the open engine's writer writes; the others are acadrust's.
const CAD_CRATES_WRITES: &str = "AC1032";

/// The DWG engines, in the order they're tried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// WithShalom's open DWG and DXF crates: tried first, on every target.
    CadCrates,
    /// acadrust: for what the open engine doesn't convert whole (native builds).
    Acadrust,
}

/// A DWG file converted to DXF.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Converted {
    /// The drawing as ASCII DXF.
    pub dxf: Vec<u8>,
    /// What the conversion left out, one note each; empty when it carried everything over.
    pub notes: Vec<String>,
    /// The engine that converted it.
    pub engine: Engine,
}

/// Read DWG bytes into DXF, saying which engine converted the file: the open engine when it
/// converts it whole, otherwise acadrust (native builds), otherwise the open engine's partial
/// conversion with its notes. The error, when neither converts the file, is acadrust's (the open
/// engine's in the web build).
pub fn convert_dwg(bytes: &[u8], limits: Limits) -> Result<Converted, String> {
    if bytes.len() > limits.max_dwg_bytes {
        return Err(too_large(bytes, limits));
    }
    let (partial, cad_crates_error) = match cad_crates::dwg_to_dxf(bytes, limits) {
        Ok(Outcome::Whole(dxf)) => return Ok(Converted { dxf, notes: Vec::new(), engine: Engine::CadCrates }),
        Err(Failure::Limit(e)) => return Err(e),
        Ok(Outcome::Partial(dxf, notes)) => (Some((dxf, notes)), None),
        Err(Failure::Failed(e)) => (None, Some(e)),
    };
    match (crate::decode(bytes, limits), partial) {
        (Ok((dxf, notes)), _) => Ok(Converted { dxf, notes, engine: Engine::Acadrust }),
        (Err(_), Some((dxf, notes))) => Ok(Converted { dxf, notes, engine: Engine::CadCrates }),
        (Err(e), None) => Err(if cfg!(no_dwg_bridge) { cad_crates_error.unwrap_or(e) } else { e }),
    }
}

/// [`convert_dwg`] with one engine only, whatever it leaves out.
pub fn convert_dwg_with(engine: Engine, bytes: &[u8], limits: Limits) -> Result<Converted, String> {
    if bytes.len() > limits.max_dwg_bytes {
        return Err(too_large(bytes, limits));
    }
    match engine {
        Engine::CadCrates => match cad_crates::dwg_to_dxf(bytes, limits) {
            Ok(Outcome::Whole(dxf)) => Ok(Converted { dxf, notes: Vec::new(), engine }),
            Ok(Outcome::Partial(dxf, notes)) => Ok(Converted { dxf, notes, engine }),
            Err(Failure::Limit(e) | Failure::Failed(e)) => Err(e),
        },
        Engine::Acadrust => crate::decode(bytes, limits).map(|(dxf, notes)| Converted { dxf, notes, engine }),
    }
}

/// The error for a DWG file over `Limits::max_dwg_bytes`.
fn too_large(bytes: &[u8], limits: Limits) -> String {
    format!("DWG: the file is {}, larger than the {} CADCraft opens", mb(bytes.len() as u64), mb(limits.max_dwg_bytes as u64))
}

/// A DWG file written from DXF, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// The file.
    pub dwg: Vec<u8>,
    /// The engine that wrote it.
    pub engine: Engine,
    /// When acadrust wrote a version the open engine writes: why the open engine didn't (what its
    /// writer would have left out, or its error), one note each. Empty when the open engine wrote
    /// the file.
    pub notes: Vec<String>,
    /// What the file leaves out of the drawing: records of each DXF type the writer couldn't
    /// write. Empty when the open engine wrote the file (it writes only what it carries whole).
    pub unwritten: crate::Unwritten,
}

/// Convert DXF bytes into a DWG file of `version`, a code from [`VERSIONS`] ("AC1032"), saying
/// which engine wrote it: 2018 (AC1032) by the open engine when it carries the drawing over whole,
/// otherwise acadrust ([`crate::dxf_to_dwg_as`]). When acadrust can't write the drawing either, the
/// error says what the open engine would have left out.
pub fn dxf_to_dwg_written(dxf: &[u8], version: &str) -> Result<Written, String> {
    let code = version.trim().to_ascii_uppercase();
    if !VERSIONS.iter().any(|(c, _)| *c == code) {
        return Err(unsupported(version));
    }
    let mut left_out = None;
    let mut notes = Vec::new();
    if code == CAD_CRATES_WRITES {
        match cad_crates::dxf_to_dwg(dxf) {
            Ok(Outcome::Whole(dwg)) => return Ok(Written { dwg, engine: Engine::CadCrates, notes, unwritten: crate::Unwritten::new() }),
            Ok(Outcome::Partial(_, lost)) => {
                notes.clone_from(&lost);
                left_out = Some(lost);
            }
            Err(Failure::Limit(e) | Failure::Failed(e)) => notes.push(e),
        }
    }
    match crate::encode_noted(dxf, &code) {
        Ok((dwg, unwritten)) => Ok(Written { dwg, engine: Engine::Acadrust, notes, unwritten }),
        Err(e) => Err(match left_out {
            Some(lost) => format!("{e} (the open engine's DWG writer would have left out: {})", lost.join("; ")),
            None => e,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version;

    /// A 2018 DXF of a line and a circle, as a simple exporter writes it: no tables, no handles.
    fn simple_dxf() -> String {
        let mut s = String::from("  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\nAC1032\n  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n");
        s += "  0\nLINE\n  8\n0\n 10\n0\n 20\n0\n 30\n0\n 11\n100\n 21\n50\n 31\n0\n";
        s += "  0\nCIRCLE\n  8\nHOLES\n 10\n20\n 20\n20\n 30\n0\n 40\n5\n";
        s + "  0\nENDSEC\n  0\nEOF\n"
    }

    /// The (code, value) pairs of a DXF text.
    fn pairs(dxf: &str) -> Vec<(String, String)> {
        let lines: Vec<&str> = dxf.lines().map(str::trim).collect();
        lines.chunks(2).filter_map(|p| Some((p.first()?.to_string(), p.get(1)?.to_string()))).collect()
    }

    /// The open engine writes the 2018 DWG of a drawing it carries over whole and reads it back
    /// whole, on its engine alone (so in the web build as well), naming CADCraft as the program
    /// that saved it.
    #[test]
    fn cad_crates_writes_and_reads_2018_whole() {
        let w = dxf_to_dwg_written(simple_dxf().as_bytes(), "ac1032").unwrap();
        assert_eq!((w.engine, w.notes.as_slice()), (Engine::CadCrates, &[] as &[String]));
        let dwg = w.dwg;
        assert_eq!(version(&dwg).as_deref(), Some("AC1032"));
        let name: Vec<u8> = "CADCraft".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(dwg.windows(name.len()).any(|w| w == name), "the file doesn't name CADCraft");
        let c = convert_dwg(&dwg, Limits::DEFAULT).unwrap();
        assert_eq!((c.engine, c.notes.as_slice()), (Engine::CadCrates, &[] as &[String]));
        let pairs = pairs(&String::from_utf8(c.dxf).unwrap());
        let has = |code: &str, value: &str| pairs.iter().any(|(c, v)| c == code && v == value);
        assert!(has("0", "LINE") && has("0", "CIRCLE") && has("8", "HOLES"), "{pairs:?}");
        assert!(has("40", "5.0") || has("40", "5"), "the circle's radius");
        assert_eq!(convert_dwg_with(Engine::CadCrates, &dwg, Limits::DEFAULT).unwrap().engine, Engine::CadCrates);
    }

    #[test]
    fn limits_apply_to_both_engines() {
        let dwg = dxf_to_dwg_written(simple_dxf().as_bytes(), "AC1032").unwrap().dwg;
        let small = Limits { max_dwg_bytes: 1000, ..Limits::DEFAULT };
        let e = convert_dwg(&dwg, small).unwrap_err();
        assert!(e.starts_with("DWG: the file is 0.") && e.ends_with("MB, larger than the 0.0 MB CADCraft opens"), "{e}");
        let tiny = Limits { max_dxf_bytes: 1000, ..Limits::DEFAULT };
        let e = convert_dwg(&dwg, tiny).unwrap_err();
        assert!(e.contains("produced more than 0.0 MB of drawing data (the limit)"), "{e}");
    }

    #[test]
    fn unknown_versions_are_refused() {
        assert!(dxf_to_dwg_written(simple_dxf().as_bytes(), "AC1099").unwrap_err().contains("can't be written"));
    }

    /// Versions the open engine doesn't write are acadrust's, and a 2007 DWG, which the open engine
    /// doesn't read, is read by acadrust.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn other_versions_go_to_acadrust() {
        let w = dxf_to_dwg_written(simple_dxf().as_bytes(), "AC1018").unwrap();
        assert_eq!((version(&w.dwg).as_deref(), w.engine), (Some("AC1018"), Engine::Acadrust));
        let dwg = dxf_to_dwg_written(simple_dxf().as_bytes(), "AC1021").unwrap().dwg;
        assert_eq!(version(&dwg).as_deref(), Some("AC1021"));
        assert_eq!(convert_dwg(&dwg, Limits::DEFAULT).unwrap().engine, Engine::Acadrust);
        assert!(convert_dwg_with(Engine::CadCrates, &dwg, Limits::DEFAULT).is_err());
    }
}
