//! What a real DWG keeps when CADCraft opens it and saves it again as DWG.
//!
//! Runs only when `MI201_CORPUS` names a folder of DWG files. In CI that is the open engine's MI-201
//! corpus (`corpus/mosaiciron-mi201` in withshalom/blackcad): one part saved by AutoCAD in every version
//! from R14 to 2018. The files are AutoCAD's, so they are never committed here; without the
//! variable the test does nothing.
//!
//! For each file it opens the DWG, saves it as 2018 DWG, opens that again, and prints a report:
//! which engine converted the file; CADCraft's entities, layers and blocks before and after; and
//! every object in the DWG by type before and after, as the open engine's reader sees both files. It
//! fails when a file doesn't open, doesn't save, or comes back with fewer model-space entities.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use cadcraft_doc::Drawing;
use cadcraft_io::Version;

/// CADCraft's view of a drawing: entities by space and type, layers, blocks.
fn cadcraft_counts(d: &Drawing) -> BTreeMap<String, usize> {
    let mut m: BTreeMap<String, usize> = BTreeMap::new();
    for e in d.model.iter() {
        *m.entry(format!("model {}", e.kind.dxf_name())).or_default() += 1;
    }
    for l in &d.layouts {
        for e in l.entities.iter() {
            *m.entry(format!("paper {}", e.kind.dxf_name())).or_default() += 1;
        }
    }
    let in_blocks: usize = d.blocks.values().map(|b| b.entities.iter().count()).sum();
    m.insert("block entities".into(), in_blocks);
    m.insert("blocks".into(), d.blocks.len());
    m.insert("layers".into(), d.layers.len());
    m.insert("layouts".into(), d.layouts.len());
    m.insert("text styles".into(), d.text_styles.len());
    m.insert("dimension styles".into(), d.dim_styles.len());
    m.insert("linetypes".into(), d.linetypes.len());
    m.insert("kept unmodelled objects".into(), d.foreign.objects.len());
    m
}

/// the open engine's view of a DWG: every object by type, and how many are kept only as DWG bytes.
fn dwg_counts(bytes: &[u8]) -> Result<BTreeMap<String, usize>, String> {
    let d = cad_dwg::read_bytes(bytes).map_err(|e| e.to_string())?;
    let mut m: BTreeMap<String, usize> = BTreeMap::new();
    for o in d.objects.values() {
        *m.entry(o.type_name.clone()).or_default() += 1;
        if o.raw.is_some() {
            *m.entry("(kept as DWG bytes)".into()).or_default() += 1;
        }
    }
    Ok(m)
}

/// Rows where `before` and `after` differ, as "name: before → after".
fn changes(before: &BTreeMap<String, usize>, after: &BTreeMap<String, usize>) -> Vec<String> {
    let mut keys: Vec<&String> = before.keys().chain(after.keys()).collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter_map(|k| {
            let (a, b) = (before.get(k).copied().unwrap_or(0), after.get(k).copied().unwrap_or(0));
            (a != b).then(|| format!("{k}: {a} → {b}"))
        })
        .collect()
}

/// The first DXF record of each type the DWG writer's notes ("DWG writer: 1A2 TABLESTYLE: …")
/// name, as `code=value` pairs (at most 2,000 characters each).
fn refused_records(notes: &[String], dxf: &str) -> Vec<(String, String)> {
    let mut kinds: Vec<String> = notes
        .iter()
        .filter_map(|n| n.strip_prefix("DWG writer: "))
        .filter_map(|n| n.split_whitespace().nth(1))
        .map(|k| k.trim_end_matches(':').to_string())
        .collect();
    kinds.sort();
    kinds.dedup();
    let lines: Vec<&str> = dxf.lines().map(str::trim).collect();
    let mut out = Vec::new();
    for kind in kinds {
        let mut i = 0;
        while i + 1 < lines.len() {
            if lines[i] == "0" && lines[i + 1].eq_ignore_ascii_case(&kind) {
                let mut pairs = Vec::new();
                let mut j = i + 2;
                while j + 1 < lines.len() && lines[j] != "0" {
                    pairs.push(format!("{}={}", lines[j], lines[j + 1]));
                    j += 2;
                }
                let mut text = pairs.join("; ");
                text.truncate(text.char_indices().nth(2000).map_or(text.len(), |(k, _)| k));
                out.push((kind.clone(), text));
                break;
            }
            i += 1;
        }
    }
    out
}

fn model_entities(m: &BTreeMap<String, usize>) -> usize {
    m.iter().filter(|(k, _)| k.starts_with("model ")).map(|(_, n)| n).sum()
}

#[test]
fn corpus_dwg_round_trips() {
    let Some(dir) = std::env::var_os("MI201_CORPUS") else {
        eprintln!("MI201_CORPUS not set: no corpus to check");
        return;
    };
    let mut files: Vec<std::path::PathBuf> =
        std::fs::read_dir(&dir).map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dwg")));
    files.sort();
    assert!(!files.is_empty(), "no DWG files in {}", std::path::Path::new(&dir).display());

    let mut report = String::new();
    let mut failures = Vec::new();
    for path in &files {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let _ = writeln!(report, "## {name}");
        let Ok(bytes) = std::fs::read(path) else {
            failures.push(format!("{name}: can't read the file"));
            continue;
        };
        if let Ok(c) = cadcraft_dwg::convert_dwg_with(cadcraft_dwg::Engine::CadCrates, &bytes, cadcraft_dwg::Limits::DEFAULT)
            && !c.notes.is_empty()
        {
            let _ = writeln!(report, "The open engine alone would leave out ({} notes):", c.notes.len());
            for n in c.notes.iter().take(5) {
                let _ = writeln!(report, "  {n}");
            }
        }
        let mut cad_crates_opened = false;
        match cadcraft_dwg::convert_dwg(&bytes, cadcraft_dwg::Limits::DEFAULT) {
            Ok(c) => {
                cad_crates_opened = c.engine == cadcraft_dwg::Engine::CadCrates;
                let _ = writeln!(
                    report,
                    "opened by: {:?}{}",
                    c.engine,
                    if c.notes.is_empty() { String::new() } else { format!(" ({} notes)", c.notes.len()) }
                );
                for n in c.notes.iter().take(5) {
                    let _ = writeln!(report, "  note: {n}");
                }
            }
            Err(e) => {
                let _ = writeln!(report, "no engine converts it: {e}");
            }
        }
        let first = match cadcraft_io::read(&bytes, &name) {
            Ok(d) => d,
            Err(e) => {
                let _ = writeln!(report, "OPEN FAILED: {e}\n");
                failures.push(format!("{name}: doesn't open: {e}"));
                continue;
            }
        };
        let saved = match cadcraft_io::write_dwg_noted(&first, Version::R2018) {
            Ok(w) => {
                let _ = writeln!(report, "saved by: {:?}", w.engine);
                for n in w.notes.iter().take(10) {
                    let _ = writeln!(report, "  the open engine's writer: {n}");
                }
                // For files the open engine opened, the DXF records its writer refused, as CADCraft
                // wrote them: what to teach the writer.
                if cad_crates_opened {
                    let dxf = cadcraft_io::dwg_bridge_dxf(&first, Version::R2018);
                    for (kind, record) in refused_records(&w.notes, &dxf) {
                        let _ = writeln!(report, "  refused {kind} as CADCraft writes it: {record}");
                    }
                }
                w.dwg
            }
            Err(e) => {
                let _ = writeln!(report, "SAVE FAILED: {e}\n");
                failures.push(format!("{name}: doesn't save as DWG: {e}"));
                continue;
            }
        };
        let again = match cadcraft_io::read(&saved, "round-trip.dwg") {
            Ok(d) => d,
            Err(e) => {
                let _ = writeln!(report, "REOPEN FAILED: {e}\n");
                failures.push(format!("{name}: the saved DWG doesn't open: {e}"));
                continue;
            }
        };
        let (a, b) = (cadcraft_counts(&first), cadcraft_counts(&again));
        let _ = writeln!(report, "CADCraft: {} model-space entities, {} layers, {} blocks", model_entities(&a), a["layers"], a["blocks"]);
        let diff = changes(&a, &b);
        if diff.is_empty() {
            let _ = writeln!(report, "CADCraft after the round trip: the same");
        } else {
            let _ = writeln!(report, "CADCraft after the round trip: {}", diff.join("; "));
        }
        if model_entities(&b) < model_entities(&a) {
            failures.push(format!("{name}: {} of {} model-space entities came back", model_entities(&b), model_entities(&a)));
        }
        match (dwg_counts(&bytes), dwg_counts(&saved)) {
            (Ok(x), Ok(y)) => {
                let total = |m: &BTreeMap<String, usize>| m.iter().filter(|(k, _)| !k.starts_with('(')).map(|(_, n)| n).sum::<usize>();
                let _ = writeln!(report, "DWG objects: {} before, {} after", total(&x), total(&y));
                let diff = changes(&x, &y);
                if !diff.is_empty() {
                    let _ = writeln!(report, "DWG objects by type: {}", diff.join("; "));
                }
            }
            (Err(e), _) => {
                let _ = writeln!(report, "The open engine doesn't read the original: {e}");
            }
            (_, Err(e)) => {
                let _ = writeln!(report, "The open engine doesn't read the saved file: {e}");
            }
        }
        let _ = writeln!(report);
    }
    println!("{report}");
    if let Some(out) = std::env::var_os("DWG_CORPUS_REPORT") {
        let _ = std::fs::write(out, &report);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
