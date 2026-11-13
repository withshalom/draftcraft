//! BlackCAD's DWG reader and writer.
//!
//! Reading: [`read_bytes`] turns a DWG file of any version from R13 to
//! AutoCAD 2018 and later (AC1012–AC1032) into a [`Drawing`].
//! Writing: [`write::to_bytes`] writes a drawing as an AutoCAD 2018
//! (AC1032) file.
//!
//! The drawing is `cad-model`'s database, re-exported here as
//! [`cad_model`].
//!
//! Reading, step by step: detect the version → extract sections from the
//! container
//! (R13–R15 flat, R2004-family paged, or AutoCAD 2007's codewords) → classes → object map → decode
//! every object → header variables (validated against the objects).
//!
//! Each object is decoded in isolation: a failure is recorded on that
//! object and the rest of the drawing still loads.

// A file is only a claim: no input reaches a panic. Tests may unwrap.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub use crate::records::version;
pub(crate) use crate::records::{error, geom, model, text};

pub mod nongraphic;

pub(crate) mod classes;
pub(crate) mod container;
pub(crate) mod eed;
pub(crate) mod entities;
pub(crate) mod header;
pub mod hvars;
pub(crate) mod object;
pub(crate) mod objmap;
#[cfg(test)]
mod probe;
pub(crate) mod r2000;
pub(crate) mod r2004;
pub(crate) mod r2007;
pub(crate) mod section;
pub(crate) mod streams;
pub(crate) mod styles;
pub(crate) mod tables;

pub use version::DwgVersion;

/// The name of one of DWG's fixed object types (`LINE` for 0x13), or `None`
/// for class-defined and unknown type numbers.
pub fn fixed_type_name(code: u16) -> Option<&'static str> {
    object::fixed_type(code).map(|(name, _)| name)
}

use crate::read::error::{Error, ErrorKind, Result};
use crate::read::model::{Diagnostic, Drawing, DrawingIndex, EntityCommon, Handle, HeaderVars, Object, ObjectKind, Properties, Severity};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

/// Read a DWG file from disk.
pub fn read_file(path: impl AsRef<Path>) -> Result<Drawing> {
    let bytes = std::fs::read(path.as_ref()).map_err(|e| Error::new(ErrorKind::Io, format!("{}: {e}", path.as_ref().display())))?;
    read_bytes(&bytes)
}

/// Identify a DWG file's version from its first six bytes.
pub fn sniff_version(bytes: &[u8]) -> Option<DwgVersion> {
    DwgVersion::from_magic(bytes)
}

/// A file's version and its container opened to sections.
pub(crate) fn open_container(bytes: &[u8]) -> Result<(DwgVersion, container::Container)> {
    let ver = DwgVersion::from_magic(bytes).ok_or_else(|| {
        let tag: String = bytes.iter().take(6).map(|&b| if b.is_ascii_graphic() { b as char } else { '?' }).collect();
        Error::new(ErrorKind::NotDwg, format!("unrecognised version tag {tag:?}"))
    })?;
    let container = match ver {
        DwgVersion::R13 | DwgVersion::R14 | DwgVersion::R2000 => r2000::read(bytes, ver)?,
        DwgVersion::R2004 | DwgVersion::R2010 | DwgVersion::R2013 | DwgVersion::R2018 => r2004::read(bytes, ver)?,
        DwgVersion::R2007 => r2007::read(bytes, ver)?,
    };
    Ok((ver, container))
}

/// Read a DWG file from memory.
pub fn read_bytes(bytes: &[u8]) -> Result<Drawing> {
    let (ver, container) = open_container(bytes)?;
    let mut diagnostics = container.notes;
    let maintenance = container.maintenance;
    let codepage = container.codepage;
    let warn = |d: &mut Vec<Diagnostic>, sev: Severity, msg: String| d.push(Diagnostic { severity: sev, message: msg });

    // Classes.
    let classes = match &container.classes {
        Some(data) => match classes::parse(data, ver, maintenance, codepage) {
            Ok((list, notes)) => {
                for n in notes {
                    warn(&mut diagnostics, Severity::Info, n);
                }
                list
            }
            Err(e) => {
                warn(&mut diagnostics, Severity::Warning, format!("classes: {e}"));
                Vec::new()
            }
        },
        None => {
            warn(&mut diagnostics, Severity::Warning, "no classes section".into());
            Vec::new()
        }
    };

    // Object map, choosing between the two readings of the delta reset.
    let entries = choose_object_map(&container.handles, ver, &container.objects, &mut diagnostics)?;

    // Objects.
    let ctx = object::DecodeCtx { ver, codepage, classes: &classes, text: Default::default() };
    let mut objects: BTreeMap<Handle, Object> = BTreeMap::new();
    let mut out_of_range = 0usize;
    let mut mismatched = 0usize;
    for e in &entries {
        if e.handle == 0 {
            continue;
        }
        if e.location < 0 || e.location as usize >= container.objects.len() {
            out_of_range += 1;
            continue;
        }
        let off = e.location as usize;
        let decoded = catch_unwind(AssertUnwindSafe(|| object::decode(&ctx, &container.objects, off)));
        let obj = match decoded {
            Ok(Ok(o)) => {
                if o.handle.0 != e.handle {
                    mismatched += 1;
                }
                o
            }
            Ok(Err(err)) => failed(&ctx, &container.objects, e.handle, off, err.to_string()),
            Err(_) => failed(&ctx, &container.objects, e.handle, off, "decoder panicked (bug)".into()),
        };
        let mut obj = obj;
        // Keep the record's bytes when the model cannot hold all of it:
        // undecoded objects, and table records kept by name only (VPORT,
        // DIMSTYLE, VIEW, UCS, APPID), so the native file loses nothing
        // a DWG writer needs.
        if !obj.kind.is_decoded() || matches!(obj.kind, ObjectKind::NamedRecord(_)) {
            obj.raw = object::raw(ver, &container.objects, off).ok();
        }
        objects.insert(Handle(e.handle), obj);
    }
    if out_of_range > 0 {
        warn(&mut diagnostics, Severity::Warning, format!("{out_of_range} object map entries point outside the object data"));
    }
    if mismatched > 0 {
        warn(&mut diagnostics, Severity::Warning, format!("{mismatched} objects carry a handle different from their map entry"));
    }
    eed::resolve(&mut objects, &mut diagnostics);
    let failed_count = objects.values().filter(|o| matches!(o.kind, ObjectKind::Failed(_))).count();
    if failed_count > 0 {
        warn(&mut diagnostics, Severity::Warning, format!("{failed_count} of {} objects failed to decode", objects.len()));
    }

    // Header variables, validated against the decoded objects.
    let type_of = |h: u64| objects.get(&Handle(h)).map(|o| o.type_code);
    let header = match &container.header {
        Some(data) => header::parse(data, ver, maintenance, codepage, &type_of),
        None => HeaderVars { note: "no header section".into(), ..Default::default() },
    };
    if !header.verified {
        warn(&mut diagnostics, Severity::Info, format!("header: {}", header.note));
    }

    let mut header = header;
    if let Some((_, s)) = container.others.iter().find(|(n, _)| n == "AcDb:SummaryInfo")
        && let Some(p) = summary_info(s, ver >= DwgVersion::R2007)
    {
        header.properties = p;
    }
    // MEASUREMENT (imperial or metric) is not a header variable in DWG
    // but the last field of AcDb:Template.
    if let Some((_, s)) = container.others.iter().find(|(n, _)| n == "AcDb:Template")
        && let Some(m) = template_measurement(s)
    {
        header.measurement = Some(m);
    }

    let mut drawing = Drawing {
        version: ver,
        maintenance_version: maintenance,
        codepage,
        header,
        classes,
        objects: objects.into(),
        sections: container.sections,
        diagnostics,
        index: DrawingIndex::default(),
    };
    nongraphic::promote(&mut drawing);
    drawing.rebuild_index();
    drawing.number_viewports();
    Ok(drawing)
}

/// MEASUREMENT from AcDb:Template (spec chapter 22): the section's last
/// short, after the template description, which AutoCAD leaves empty (it
/// writes `01 00 00 00` before it). 0 is imperial, 1 metric; anything else
/// is not believed. R13–R15 files have no such section and keep the
/// default.
pub(crate) fn template_measurement(s: &[u8]) -> Option<u16> {
    let at = s.len().checked_sub(2)?;
    let m = u16::from_le_bytes([*s.get(at)?, *s.get(at + 1)?]);
    (m <= 1).then_some(m)
}

/// One string of AcDb:SummaryInfo: RS length counting the terminating 0,
/// then UTF-16 units (R2007+) or 8-bit characters.
fn summary_text(s: &[u8], p: &mut usize, wide: bool) -> Option<String> {
    let n = u16::from_le_bytes(s.get(*p..*p + 2)?.try_into().ok()?) as usize;
    *p += 2;
    let t = if wide {
        let b = s.get(*p..*p + 2 * n)?;
        *p += 2 * n;
        let units: Vec<u16> = b.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        let b = s.get(*p..*p + n)?;
        *p += n;
        b.iter().map(|&c| c as char).collect()
    };
    Some(t.trim_end_matches('\0').to_string())
}

/// AcDb:SummaryInfo (spec chapter 13): eight strings, three dates the
/// header also holds, then custom properties as name and value pairs.
/// `None` if it does not parse.
pub(crate) fn summary_info(s: &[u8], wide: bool) -> Option<Properties> {
    let mut p = 0usize;
    let mut f = Vec::with_capacity(8);
    for _ in 0..8 {
        f.push(summary_text(s, &mut p, wide)?);
    }
    let mut props = Properties {
        title: f[0].clone(),
        subject: f[1].clone(),
        author: f[2].clone(),
        keywords: f[3].clone(),
        comments: f[4].clone(),
        last_saved_by: f[5].clone(),
        revision: f[6].clone(),
        hyperlink_base: f[7].clone(),
        ..Default::default()
    };
    p += 24;
    let count = u16::from_le_bytes(s.get(p..p + 2)?.try_into().ok()?) as usize;
    p += 2;
    for _ in 0..count.min(1024) {
        let k = summary_text(s, &mut p, wide)?;
        let v = summary_text(s, &mut p, wide)?;
        props.custom.push((k, v));
    }
    Some(props)
}

fn failed(ctx: &object::DecodeCtx, data: &[u8], handle: u64, offset: usize, message: String) -> Object {
    let (type_code, type_name, is_entity) = match object::peek(ctx.ver, data, offset) {
        Ok((_, code)) => {
            let (name, ent) = object::classify(code, ctx.classes);
            (code, name, ent)
        }
        Err(_) => (0, "?".to_string(), false),
    };
    Object {
        handle: Handle(handle),
        type_code,
        type_name,
        owner: Handle::NULL,
        reactors: Vec::new(),
        xdictionary: Handle::NULL,
        entity: if is_entity { Some(EntityCommon::default()) } else { None },
        kind: ObjectKind::Failed(message),
        offset,
        size: 0,
        note: None,
        raw: None,
        xdata: Vec::new(),
        unknown_tags: Vec::new(),
    }
}

/// Parse the object map both ways and keep the reading whose entries agree
/// with the handles stored in the objects themselves.
pub(crate) fn choose_object_map(handles: &[u8], ver: DwgVersion, objects: &[u8], diagnostics: &mut Vec<Diagnostic>) -> Result<Vec<objmap::MapEntry>> {
    let score = |entries: &[objmap::MapEntry]| -> (usize, usize) {
        if entries.is_empty() {
            return (0, 0);
        }
        let step = (entries.len() / 64).max(1);
        let mut hit = 0;
        let mut tried = 0;
        for e in entries.iter().step_by(step) {
            tried += 1;
            if e.location >= 0
                && let Ok((h, _)) = object::peek(ver, objects, e.location as usize)
                && h == e.handle
            {
                hit += 1;
            }
        }
        (hit, tried)
    };
    let reset = objmap::parse(handles, true);
    let running = objmap::parse(handles, false);
    match (reset, running) {
        (Ok(a), Ok(b)) => {
            let (sa, ta) = score(&a);
            let (sb, tb) = score(&b);
            let (chosen, label, s, t) = if sb > sa { (b, "running", sb, tb) } else { (a, "per-block reset", sa, ta) };
            let sev = if t > 0 && s * 10 >= t * 9 { Severity::Info } else { Severity::Warning };
            diagnostics.push(Diagnostic {
                severity: sev,
                message: format!("object map: {} entries, {label} reading, {s}/{t} sampled handles match", chosen.len()),
            });
            Ok(chosen)
        }
        (Ok(a), Err(_)) | (Err(_), Ok(a)) => Ok(a),
        (Err(e), Err(_)) => Err(e.context("object map")),
    }
}
