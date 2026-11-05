//! How much of an AutoCAD drawing CADCraft's model holds: the measurements behind
//! `docs/cadcraft-doc.md`.
//!
//! Runs only when `MI201_CORPUS` names a folder of AutoCAD-saved DXF and DWG files, for example
//! the MI-201 corpus (`corpus/mosaiciron-mi201` in https://github.com/withshalom/blackcad). The
//! files are AutoCAD's, so they are never committed here; without the variable the test does
//! nothing. With `CADCRAFT_MODEL_MAP=path` it also writes the report to that file.
//!
//! For each DXF it reads the file into CADCraft's model, saves it as DXF in its own version (as
//! CADCraft saves a drawing), and compares the two files record by record (matched by handle), by section and type:
//! - how CADCraft holds the type: modelled, or kept as raw DXF groups (unmodelled objects,
//!   `Unknown` entities);
//! - records the same after the round trip, changed, or missing;
//! - for changed records, the group codes that were lost and the ones that were added;
//! - header variables lost or changed.
//!
//! It also writes the model to `.cad` and back and checks the DXF written from that is identical,
//! so anything the model holds, `.cad` holds. For each DWG it lists the object types the file
//! holds (the open engine's reader), so DWG-only types show up next to the DXF ones.
//!
//! It fails only when a file doesn't open or `.cad` loses something the model held.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use cadcraft_doc::{Drawing, EntityKind};
use cadcraft_dxf::{Kind, Tag, kind_of, records, sections};
use cadcraft_io::{cad_file, read_dxf};

/// The drawing saved as DXF the way CADCraft saves it: in its own version.
fn save_dxf(d: &Drawing) -> String {
    String::from_utf8_lossy(&cadcraft_io::write(d, "drawing.dxf").unwrap_or_default()).into_owned()
}

/// A group as text, with reals rounded so formatting differences don't count as changes.
fn norm(t: &Tag) -> (i32, String) {
    let v = match kind_of(t.code) {
        Kind::Real => {
            let f = t.f64();
            if f.abs() < 1e-10 { "0".to_string() } else { format!("{f:.9e}") }
        }
        Kind::Int16 | Kind::Int32 | Kind::Int64 | Kind::Bool => t.i64().to_string(),
        Kind::Hex => t.str().to_ascii_uppercase(),
        Kind::Str => t.str(),
    };
    (t.code, v)
}

/// One record: section, type, handle and its groups as a sorted bag.
struct Rec {
    section: String,
    kind: String,
    /// Group 2 (a table record's or block's name), if any.
    name: Option<String>,
    groups: Vec<(i32, String)>,
}

/// Handles and references to other objects: CADCraft gives tables and objects new handles when it
/// writes a file, so these are compared separately (a record keeps its handle or not).
fn is_reference(code: i32) -> bool {
    matches!(code, 5 | 105 | 330..=369 | 390..=399 | 480 | 481 | 1005)
}

/// Records with a handle (group 5, or 105 for DIMSTYLE), keyed by handle; and header variables.
fn index(text: &str) -> Result<(BTreeMap<String, Rec>, BTreeMap<String, Vec<(i32, String)>>), String> {
    let tags = cadcraft_dxf::parse(text.as_bytes()).map_err(|e| e.to_string())?;
    let mut recs = BTreeMap::new();
    let mut header = BTreeMap::new();
    for s in sections(&tags) {
        if s.name == "HEADER" {
            let mut name: Option<String> = None;
            for t in &s.tags {
                if t.code == 9 {
                    name = Some(t.str());
                    header.insert(t.str(), Vec::new());
                } else if let Some(n) = &name
                    && let Some(v) = header.get_mut(n)
                {
                    v.push(norm(t));
                }
            }
            continue;
        }
        for (kind, groups) in records(&s.tags) {
            let handle = groups.iter().find(|t| t.code == 5 || (kind == "DIMSTYLE" && t.code == 105)).map(Tag::str);
            let Some(h) = handle else { continue };
            let mut g: Vec<(i32, String)> = groups.iter().filter(|t| !is_reference(t.code)).map(norm).collect();
            g.sort();
            let name = groups.iter().find(|t| t.code == 2).map(Tag::str);
            recs.insert(h.to_ascii_uppercase(), Rec { section: s.name.clone(), kind, name, groups: g });
        }
    }
    Ok((recs, header))
}

/// Groups in `a` not matched in `b` (multiset difference), as codes.
fn missing_codes(a: &[(i32, String)], b: &[(i32, String)]) -> Vec<i32> {
    let mut left: BTreeMap<&(i32, String), usize> = BTreeMap::new();
    for g in b {
        *left.entry(g).or_default() += 1;
    }
    let mut out = Vec::new();
    for g in a {
        match left.get_mut(g) {
            Some(n) if *n > 0 => *n -= 1,
            _ => out.push(g.0),
        }
    }
    out
}

#[derive(Default)]
struct Row {
    count: usize,
    /// Matched by name or position, not by handle: the record was given a new handle.
    rehandled: usize,
    same: usize,
    changed: usize,
    missing: usize,
    lost: BTreeMap<i32, usize>,
    added: BTreeMap<i32, usize>,
}

/// How CADCraft holds each record type after reading this drawing.
fn holding(d: &Drawing) -> BTreeMap<String, &'static str> {
    let mut m = BTreeMap::new();
    for o in &d.foreign.objects {
        m.insert(o.kind.clone(), "kept raw");
    }
    let mut note = |e: &cadcraft_doc::Entity| {
        if let EntityKind::Unknown(u) = &e.kind {
            m.insert(u.dxf_type.clone(), "kept raw");
        }
    };
    d.model.iter().for_each(|e| note(e));
    d.layouts.iter().for_each(|l| l.entities.iter().for_each(|e| note(e)));
    d.blocks.values().for_each(|b| b.entities.iter().for_each(|e| note(e)));
    m
}

fn top(m: &BTreeMap<i32, usize>) -> String {
    let mut v: Vec<(&i32, &usize)> = m.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    v.iter().take(12).map(|(c, n)| format!("{c}×{n}")).collect::<Vec<_>>().join(" ")
}

fn map_dxf(path: &std::path::Path, out: &mut String) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let src_text = if cadcraft_dxf::is_binary(&bytes) {
        // Compare in ASCII form either way.
        let tags = cadcraft_dxf::parse(&bytes).map_err(|e| e.to_string())?;
        cadcraft_dxf::write_ascii(&tags)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let d = read_dxf(&bytes).map_err(|e| format!("CADCraft doesn't read it: {e}"))?;
    let back = save_dxf(&d);

    // `.cad` must hold everything the model holds.
    let cad = cad_file::write(&d, None).map_err(|e| format!(".cad write: {e}"))?;
    let d2 = cad_file::read(&cad).map_err(|e| format!(".cad read: {e}"))?;
    let back2 = save_dxf(&d2);
    if back2 != back {
        return Err(".cad round trip changes the drawing (the DXF written after it differs)".into());
    }

    let (src, src_hdr) = index(&src_text)?;
    let (dst, dst_hdr) = index(&back)?;
    let held = holding(&d);

    // Match each source record: by handle (same type), else by type and name, else by type and
    // position among the unmatched records of that type.
    let mut used: BTreeSet<&String> = BTreeSet::new();
    let mut pairs: Vec<(&Rec, Option<&Rec>, bool)> = Vec::new();
    let mut pending: Vec<&Rec> = Vec::new();
    for (h, r) in &src {
        match dst.get_key_value(h) {
            Some((k, o)) if o.kind == r.kind && o.section == r.section => {
                used.insert(k);
                pairs.push((r, Some(o), false));
            }
            _ => pending.push(r),
        }
    }
    let mut still: Vec<&Rec> = Vec::new();
    for r in pending {
        let hit = r.name.as_ref().and_then(|n| {
            dst.iter().find(|(k, o)| {
                !used.contains(k) && o.kind == r.kind && o.section == r.section && o.name.as_ref().is_some_and(|m| m.eq_ignore_ascii_case(n))
            })
        });
        match hit {
            Some((k, o)) => {
                used.insert(k);
                pairs.push((r, Some(o), true));
            }
            None => still.push(r),
        }
    }
    for r in still {
        let hit = dst.iter().find(|(k, o)| !used.contains(k) && o.kind == r.kind && o.section == r.section);
        match hit {
            Some((k, o)) => {
                used.insert(k);
                pairs.push((r, Some(o), true));
            }
            None => pairs.push((r, None, false)),
        }
    }

    let mut rows: BTreeMap<(String, String), Row> = BTreeMap::new();
    for (r, o, rehandled) in pairs {
        let row = rows.entry((r.section.clone(), r.kind.clone())).or_default();
        row.count += 1;
        row.rehandled += usize::from(rehandled);
        match o {
            None => row.missing += 1,
            Some(o) if o.groups == r.groups => row.same += 1,
            Some(o) => {
                row.changed += 1;
                for c in missing_codes(&r.groups, &o.groups) {
                    *row.lost.entry(c).or_default() += 1;
                }
                for c in missing_codes(&o.groups, &r.groups) {
                    *row.added.entry(c).or_default() += 1;
                }
            }
        }
    }
    let mut new: BTreeMap<(String, String), usize> = BTreeMap::new();
    for (k, r) in &dst {
        if !used.contains(k) {
            *new.entry((r.section.clone(), r.kind.clone())).or_default() += 1;
        }
    }

    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ver = src_hdr.get("$ACADVER").and_then(|v| v.first()).map(|v| v.1.clone()).unwrap_or_default();
    let (all, same): (usize, usize) = rows.values().fold((0, 0), |a, r| (a.0 + r.count, a.1 + r.same));
    let _ = writeln!(
        out,
        "\n## {name} ({ver})\n\n{same} of {all} records come back with the same values (references to other \
         objects aside).\n"
    );
    let _ = writeln!(out, "| Section | Type | Held as | Records | Same | Changed | Missing | New handle | Codes lost | Codes added |");
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---|");
    for ((sec, kind), r) in &rows {
        let how = held.get(kind).copied().unwrap_or(if r.missing == r.count { "dropped" } else { "modelled" });
        let _ = writeln!(
            out,
            "| {sec} | {kind} | {how} | {} | {} | {} | {} | {} | {} | {} |",
            r.count,
            r.same,
            r.changed,
            r.missing,
            r.rehandled,
            top(&r.lost),
            top(&r.added)
        );
    }
    if !new.is_empty() {
        let list: Vec<String> = new.iter().map(|((s, k), n)| format!("{s} {k} ×{n}")).collect();
        let _ = writeln!(out, "\nRecords only in CADCraft's file: {}", list.join(", "));
    }
    let lost_vars: Vec<&String> = src_hdr.keys().filter(|k| !dst_hdr.contains_key(*k)).collect();
    let changed_vars: Vec<&String> = src_hdr.iter().filter(|(k, v)| dst_hdr.get(*k).is_some_and(|w| w != *v)).map(|(k, _)| k).collect();
    let _ = writeln!(
        out,
        "\nHeader: {} variables; {} lost{}; {} changed{}.",
        src_hdr.len(),
        lost_vars.len(),
        if lost_vars.is_empty() { String::new() } else { format!(" ({})", join(&lost_vars)) },
        changed_vars.len(),
        if changed_vars.is_empty() { String::new() } else { format!(" ({})", join(&changed_vars)) },
    );
    Ok(())
}

fn join(v: &[&String]) -> String {
    v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
}

fn map_dwg(path: &std::path::Path, out: &mut String) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let Ok(bytes) = std::fs::read(path) else { return };
    match cad_dwg::read_bytes(&bytes) {
        Ok(d) => {
            let mut m: BTreeMap<&str, usize> = BTreeMap::new();
            for o in d.objects.values() {
                *m.entry(o.type_name.as_str()).or_default() += 1;
            }
            let list: Vec<String> = m.iter().map(|(k, n)| format!("{k} {n}")).collect();
            let _ = writeln!(out, "- **{name}**: {} objects: {}", d.objects.len(), list.join(", "));
        }
        Err(e) => {
            let _ = writeln!(out, "- **{name}**: not read by the open engine's reader ({e})");
        }
    }
}

#[test]
fn model_map() {
    let Some(dir) = std::env::var_os("MI201_CORPUS") else {
        eprintln!("MI201_CORPUS not set: no corpus to map");
        return;
    };
    let mut files: Vec<std::path::PathBuf> =
        std::fs::read_dir(&dir).map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default();
    files.sort();
    let ext = |p: &std::path::Path, e: &str| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(e));

    let mut out = String::from("# Model map: AutoCAD drawings through CADCraft's model\n");
    let mut failures = Vec::new();
    for p in files.iter().filter(|p| ext(p, "dxf")) {
        if let Err(e) = map_dxf(p, &mut out) {
            failures.push(format!("{}: {e}", p.display()));
        }
    }
    let _ = writeln!(out, "\n## DWG object types (the open engine's reader)\n");
    for p in files.iter().filter(|p| ext(p, "dwg")) {
        map_dwg(p, &mut out);
    }
    if let Some(path) = std::env::var_os("CADCRAFT_MODEL_MAP") {
        let _ = std::fs::write(path, &out);
    }
    eprintln!("{out}");
    assert!(failures.is_empty(), "{failures:#?}");
}
