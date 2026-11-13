//! Sweep: every DWG the reader can be given here, read with acadrust disabled (the direct reader),
//! compared with what the drawing should hold.
//!
//! 1. The AutoCAD files (MI-201, every version): direct vs acadrust, per entity type, layers,
//!    blocks, layouts, and what the direct reader left unread.
//! 2. DraftCraft's own drawings (the sample, examples/apartment.dxf): saved as DWG in every
//!    version, read back directly, compared with the drawing saved.
//! 3. Stability: direct read → DXF → read → same drawing.
//!
//! Run: cargo run --release -p draftcraft-io --example direct_sweep -- <out-dir> <dwg-or-dxf files…>

use std::collections::BTreeMap;
use std::path::Path;

use draftcraft_doc::{Drawing, EntityKind};
use draftcraft_io::{DwgReader, SaveOptions, Version, read_dwg_with, write_with};

fn kinds(d: &Drawing) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    let mut add = |k: &EntityKind, space: &str| *m.entry(format!("{space}:{}", k.dxf_name())).or_insert(0) += 1;
    for e in d.model.iter() {
        add(&e.kind, "model");
    }
    for l in &d.layouts {
        for e in l.entities.iter() {
            add(&e.kind, "paper");
        }
    }
    for b in d.blocks.values() {
        for e in b.entities.iter() {
            add(&e.kind, "block");
        }
    }
    m
}

/// A block name with an anonymous block's number dropped (`*D7` → `*D`): DWG keeps no number.
fn bare(name: &str) -> String {
    let up = name.to_ascii_uppercase();
    if up.starts_with('*') && !up.starts_with("*MODEL") && !up.starts_with("*PAPER") {
        up.trim_end_matches(|c: char| c.is_ascii_digit()).to_string()
    } else {
        up
    }
}

fn names(d: &Drawing) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut layers: Vec<String> = d.layers.iter().map(|l| l.name.to_ascii_uppercase()).collect();
    layers.sort();
    let mut blocks: Vec<String> = d.blocks.keys().map(|k| bare(k)).collect();
    blocks.sort();
    let layouts: Vec<String> = d.layouts.iter().map(|l| l.name.clone()).collect();
    (layers, blocks, layouts)
}

fn diff(a: &BTreeMap<String, usize>, b: &BTreeMap<String, usize>) -> Vec<String> {
    let mut out = Vec::new();
    for k in a.keys().chain(b.keys()).collect::<std::collections::BTreeSet<_>>() {
        let (x, y) = (a.get(k).copied().unwrap_or(0), b.get(k).copied().unwrap_or(0));
        if x != y {
            out.push(format!("{k}: {x} vs {y}"));
        }
    }
    out
}

fn compare(label: &str, got: &Drawing, want: &Drawing, report: &mut Vec<String>) -> bool {
    let mut ok = true;
    let d = diff(&kinds(got), &kinds(want));
    if !d.is_empty() {
        ok = false;
        report.push(format!("  {label}: entity counts differ: {}", d.join("; ")));
    }
    let (gl, gb, gy) = names(got);
    let (wl, wb, wy) = names(want);
    for (what, g, w) in [("layers", gl, wl), ("blocks", gb, wb), ("layouts", gy, wy)] {
        if g != w {
            ok = false;
            let missing: Vec<_> = w.iter().filter(|x| !g.contains(x)).collect();
            let extra: Vec<_> = g.iter().filter(|x| !w.contains(x)).collect();
            report.push(format!("  {label}: {what} differ: missing {missing:?}, extra {extra:?}"));
        }
    }
    ok
}

/// A drawing for one stress case: `mixed` (n entities of nine kinds, cycled), `polyline` (one
/// polyline of n vertices), `hatch` (one hatch of n loops), `layers` (n layers, one line each),
/// `blocks` (n blocks, each inserted once, with an attribute), `mtext` (one paragraph text of n
/// characters), `table` (a table of n rows by 20 columns), `nested` (blocks nested n deep).
fn stress_drawing(case: &str, n: usize) -> Drawing {
    use draftcraft_doc::{
        Arc, Attrib, AttribModes, AttribProps, Block, Circle, Common, DimKind, Dimension, Entity, HAlign, Hatch, HatchLoop, Insert, Line, LwPolyline,
        MText, Space, Table, TableCell, Text, VAlign,
    };
    use draftcraft_geom::{PolyVertex, Vec2, Vec3};
    let mut d = Drawing::new_imperial();
    let p = |x: f64, y: f64| Vec3::new(x, y, 0.0);
    let text = |at: Vec3, value: String| Text {
        insert: at,
        align_pt: None,
        height: 0.2,
        value,
        rotation: 0.0,
        width_factor: 1.0,
        oblique: 0.0,
        style: "Standard".into(),
        halign: HAlign::Left,
        valign: VAlign::Baseline,
    };
    let mtext = |at: Vec3, contents: String| MText {
        insert: at,
        height: 0.2,
        width: 10.0,
        attach: 1,
        rotation: 0.0,
        style: "Standard".into(),
        contents,
        line_spacing: 1.0,
        line_spacing_exact: false,
        columns: None,
    };
    let square = |x: f64, y: f64, s: f64| {
        vec![
            PolyVertex::new(Vec2::new(x, y)),
            PolyVertex::new(Vec2::new(x + s, y)),
            PolyVertex::new(Vec2::new(x + s, y + s)),
            PolyVertex::new(Vec2::new(x, y + s)),
        ]
    };
    let hatch = |loops: Vec<HatchLoop>| Hatch {
        pattern: "SOLID".into(),
        solid: true,
        loops,
        scale: 1.0,
        angle: 0.0,
        associative: false,
        style: 0,
        elevation: 0.0,
        gradient: None,
        origin: Vec2::ZERO,
        background: None,
        pattern_lines: Vec::new(),
    };
    let insert = |block: &str, at: Vec3, attribs: Vec<Attrib>| Insert {
        block: block.into(),
        insert: at,
        scale: Vec3::new(1.0, 1.0, 1.0),
        rotation: 0.0,
        attribs,
        cols: 1,
        rows: 1,
        col_spacing: 0.0,
        row_spacing: 0.0,
        clip: None,
    };
    let attdef = |tag: &str| Attrib {
        tag: tag.into(),
        text: text(p(0.0, 0.0), tag.into()),
        invisible: false,
        constant: false,
        prompt: String::new(),
        props: AttribProps::default(),
        multiline: None,
        modes: AttribModes::default(),
    };
    let add = |d: &mut Drawing, k: EntityKind| {
        let _ = d.add(&Space::Model, Common::default(), k);
    };
    match case {
        "mixed" => {
            let mut b = Block::new("TAG");
            let h = d.new_handle();
            b.entities.push(Entity::new(h, EntityKind::AttDef(attdef("NO"))));
            b.entities.push(Entity::new(d.new_handle(), EntityKind::Circle(Circle { center: p(0.0, 0.0), radius: 0.5 })));
            d.blocks.insert("TAG".into(), std::sync::Arc::new(b));
            for i in 0..n {
                let (x, y) = ((i % 1000) as f64 * 3.0, (i / 1000) as f64 * 3.0);
                let k = match i % 9 {
                    0 => EntityKind::Line(Line { a: p(x, y), b: p(x + 2.0, y + 1.0) }),
                    1 => EntityKind::Circle(Circle { center: p(x, y), radius: 1.0 }),
                    2 => EntityKind::Arc(Arc { center: p(x, y), radius: 1.0, start: 0.3, end: 2.5 }),
                    3 => EntityKind::LwPolyline(LwPolyline {
                        vertices: square(x, y, 2.0),
                        closed: true,
                        const_width: 0.0,
                        elevation: 0.0,
                        plinegen: false,
                    }),
                    4 => EntityKind::Text(text(p(x, y), format!("T{i}"))),
                    5 => EntityKind::MText(mtext(p(x, y), format!("M{i}\\Pline two"))),
                    6 => EntityKind::Insert(insert("TAG", p(x, y), vec![Attrib { text: text(p(x, y), format!("{i}")), ..attdef("NO") }])),
                    7 => EntityKind::Hatch(hatch(vec![HatchLoop { vertices: square(x, y, 1.5), outer: true, sources: Vec::new() }])),
                    _ => EntityKind::Dimension(Dimension {
                        kind: DimKind::Aligned,
                        defpt: p(x + 2.0, y + 1.0),
                        text_mid: p(x + 1.0, y + 1.0),
                        p13: p(x, y),
                        p14: p(x + 2.0, y),
                        p15: Vec3::ZERO,
                        p16: Vec3::ZERO,
                        text: String::new(),
                        style: "Standard".into(),
                        measurement: 0.0,
                        text_rotation: 0.0,
                        user_text_pos: false,
                        block: None,
                        insertion_point: Vec3::ZERO,
                        overrides: Default::default(),
                        assoc: Vec::new(),
                        marks: Default::default(),
                    }),
                };
                add(&mut d, k);
            }
        }
        "polyline" => {
            let vertices = (0..n).map(|i| PolyVertex::new(Vec2::new(i as f64 * 0.01, ((i as f64) * 0.1).sin()))).collect();
            add(&mut d, EntityKind::LwPolyline(LwPolyline { vertices, closed: false, const_width: 0.0, elevation: 0.0, plinegen: false }));
        }
        "hatch" => {
            let mut loops = vec![HatchLoop { vertices: square(0.0, 0.0, n as f64 * 3.0 + 3.0), outer: true, sources: Vec::new() }];
            loops.extend((0..n).map(|i| HatchLoop {
                vertices: square(1.0 + (i % 1000) as f64 * 3.0, 1.0 + (i / 1000) as f64 * 3.0, 1.0),
                outer: false,
                sources: Vec::new(),
            }));
            add(&mut d, EntityKind::Hatch(hatch(loops)));
        }
        "layers" => {
            for i in 0..n {
                let c = Common { layer: format!("L{i:06}"), ..Common::default() };
                let _ = d.add(&Space::Model, c, EntityKind::Line(Line { a: p(0.0, i as f64), b: p(5.0, i as f64) }));
            }
        }
        "blocks" => {
            for i in 0..n {
                let name = format!("B{i:06}");
                let mut b = Block::new(&name);
                b.entities.push(Entity::new(d.new_handle(), EntityKind::AttDef(attdef("NO"))));
                b.entities.push(Entity::new(d.new_handle(), EntityKind::Circle(Circle { center: p(0.0, 0.0), radius: 0.5 })));
                d.blocks.insert(name.clone(), std::sync::Arc::new(b));
                let at = p((i % 1000) as f64 * 2.0, (i / 1000) as f64 * 2.0);
                add(&mut d, EntityKind::Insert(insert(&name, at, vec![Attrib { text: text(at, format!("{i}")), ..attdef("NO") }])));
            }
        }
        "mtext" => {
            let mut s = String::with_capacity(n);
            while s.len() < n {
                s.push_str("Lorem ipsum dolor sit amet ");
            }
            add(&mut d, EntityKind::MText(mtext(p(0.0, 0.0), s)));
        }
        "table" => {
            let cells = (0..n).map(|r| (0..20).map(|c| TableCell { text: format!("R{r}C{c}"), ..TableCell::default() }).collect()).collect();
            add(
                &mut d,
                EntityKind::Table(Table { insert: p(0.0, 0.0), col_widths: vec![1.5; 20], row_heights: vec![0.3; n], cells, ..Table::default() }),
            );
        }
        "nested" => {
            for i in 0..n {
                let name = format!("N{i:04}");
                let mut b = Block::new(&name);
                b.entities.push(Entity::new(d.new_handle(), EntityKind::Line(Line { a: p(0.0, 0.0), b: p(1.0, 1.0) })));
                if i > 0 {
                    b.entities.push(Entity::new(d.new_handle(), EntityKind::Insert(insert(&format!("N{:04}", i - 1), p(0.1, 0.0), Vec::new()))));
                }
                d.blocks.insert(name, std::sync::Arc::new(b));
            }
            add(&mut d, EntityKind::Insert(insert(&format!("N{:04}", n.saturating_sub(1)), p(0.0, 0.0), Vec::new())));
        }
        _ => {}
    }
    d
}

/// The process's peak memory so far, from /proc (Linux), in MB.
fn peak_mb() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).map(|l| l.split_whitespace().nth(1).unwrap_or("0").to_string()))
        .and_then(|kb| kb.parse::<u64>().ok())
        .map_or_else(|| "?".into(), |kb| format!("{} MB", kb / 1024))
}

/// One stress case: build, save as DWG, read back directly; print the sizes and times and
/// whether everything came back.
fn stress(case: &str, n: usize, version: Version) {
    let t = std::time::Instant::now();
    let d = stress_drawing(case, n);
    let build = t.elapsed();
    let t = std::time::Instant::now();
    let dwg = match write_with(&d, "x.dwg", &SaveOptions { version: Some(version), binary: false }) {
        Ok(b) => b,
        Err(e) => {
            println!("STRESS {case} n={n} {version:?}: SAVE FAILED after {:?}: {e}", t.elapsed());
            return;
        }
    };
    let save = t.elapsed();
    if let Ok(dir) = std::env::var("SWEEP_SAVE") {
        std::fs::write(Path::new(&dir).join(format!("stress-{case}-{n}-{version:?}.dwg")), &dwg).ok();
    }
    let t = std::time::Instant::now();
    let back = read_dwg_with(&dwg, DwgReader::Direct);
    let read = t.elapsed();
    match back {
        Err(e) => {
            let other = match read_dwg_with(&dwg, DwgReader::Acadrust) {
                Ok(a) => format!("acadrust reads the same file ({} entities in model space)", a.model.len()),
                Err(e) => format!("acadrust can't read it either: {e}"),
            };
            println!("STRESS {case} n={n} {version:?}: {} MB, save {save:?}; DIRECT READ FAILED after {read:?}: {e}\n  {other}", dwg.len() >> 20);
            if let Ok(dir) = std::env::var("SWEEP_KEEP") {
                std::fs::write(Path::new(&dir).join(format!("{case}-{n}-{version:?}.dwg")), &dwg).ok();
            }
        }
        Ok(back) => {
            let saved = write_with(&d, "x.dxf", &SaveOptions::default()).ok().and_then(|b| draftcraft_io::read(&b, "x.dxf").ok()).unwrap_or(d);
            let mut report = Vec::new();
            let same = compare("read back vs saved", &back, &saved, &mut report);
            let content = same && content_same(&back, &saved, &mut report);
            if !content {
                // Whose loss: does acadrust's reader get it from the same file?
                match read_dwg_with(&dwg, DwgReader::Acadrust) {
                    Ok(a) => {
                        let mut r = Vec::new();
                        let a_same = compare("acadrust vs saved", &a, &saved, &mut r) && content_same(&a, &saved, &mut r);
                        report.push(format!(
                            "  acadrust reading the same file: {}",
                            if a_same { "everything came back (the direct reader's loss)" } else { "differs too (the file's)" }
                        ));
                    }
                    Err(e) => report.push(format!("  acadrust can't read it either: {e}")),
                }
            }
            println!(
                "STRESS {case} n={n} {version:?}: {:.1} MB, build {build:?}, save {save:?}, direct read {read:?}, peak memory {}: {}{}",
                dwg.len() as f64 / 1048576.0,
                peak_mb(),
                if content { "everything came back" } else { "DIFFERS" },
                if report.is_empty() { String::new() } else { format!("\n{}", report.join("\n")) }
            );
        }
    }
}

/// Two readings of the same value: numbers within 1e-9 (relative to their size, or absolute
/// near zero), as a value read in degrees and written in radians, or through a text format,
/// drifts in its last digits; everything else equal.
fn close(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value as J;
    match (a, b) {
        (J::Number(x), J::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0),
            _ => x == y,
        },
        (J::Array(x), J::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| close(x, y)),
        (J::Object(x), J::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| close(v, w))),
        _ => a == b,
    }
}

/// Every entity's content, in order, space by space (anonymous block names aside).
fn content_same(a: &Drawing, b: &Drawing, report: &mut Vec<String>) -> bool {
    let json = |k: &EntityKind| {
        let mut k = k.clone();
        match &mut k {
            EntityKind::Dimension(dm) => dm.block = dm.block.as_deref().map(bare),
            EntityKind::Insert(i) => i.block = bare(&i.block),
            _ => {}
        }
        serde_json::to_string(&k).unwrap_or_default()
    };
    let key = |d: &Drawing| -> Vec<String> {
        let mut v: Vec<String> = d.model.iter().map(|e| json(&e.kind)).collect();
        for l in &d.layouts {
            v.extend(l.entities.iter().map(|e| json(&e.kind)));
        }
        v
    };
    let (ka, kb) = (key(a), key(b));
    let same = |x: &String, y: &String| x == y || matches!((serde_json::from_str(x), serde_json::from_str(y)), (Ok(x), Ok(y)) if close(&x, &y));
    let differing = ka.iter().zip(&kb).filter(|(x, y)| !same(x, y)).count();
    if differing > 0 || ka.len() != kb.len() {
        if let Some((x, y)) = ka.iter().zip(&kb).find(|(x, y)| !same(x, y)) {
            let at = x.chars().zip(y.chars()).position(|(a, b)| a != b).unwrap_or_else(|| x.chars().count().min(y.chars().count()));
            report.push(format!(
                "  {differing} entities differ; first (read {} characters, saved {}; they part at {at}):\n    read  …{}\n    saved …{}",
                x.chars().count(),
                y.chars().count(),
                x.chars().skip(at.saturating_sub(60)).take(200).collect::<String>(),
                y.chars().skip(at.saturating_sub(60)).take(200).collect::<String>()
            ));
        }
        return false;
    }
    true
}

/// `--vs-dxf <dwg> <dxf>`: the direct reading of a DWG against another program's DXF of the
/// same file, read by DraftCraft's DXF reader: entities per type and space, layers, blocks,
/// layouts, then every entity's content in order.
fn vs_dxf(dwg: &str, dxf: &str) {
    let name = Path::new(dwg).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (Ok(a), Ok(b)) = (std::fs::read(dwg), std::fs::read(dxf)) else {
        println!("VS {name}: can't read the files");
        return;
    };
    let direct = match read_dwg_with(&a, DwgReader::Direct) {
        Ok(d) => d,
        Err(e) => {
            println!("VS {name}: DIRECT READ FAILED: {e}");
            return;
        }
    };
    let other = match draftcraft_io::read(&b, "other.dxf") {
        Ok(d) => d,
        Err(e) => {
            println!("VS {name}: the other program's DXF doesn't read: {e}");
            return;
        }
    };
    let mut report = Vec::new();
    let same = compare("direct vs other", &direct, &other, &mut report);
    let content = same && content_same(&direct, &other, &mut report);
    println!(
        "VS {name}: {}{}",
        if content { "same" } else { "DIFFERS" },
        if report.is_empty() { String::new() } else { format!("\n{}", report.join("\n")) }
    );
}

/// Every difference the round-trip comparison finds between AutoCAD's DXF of a drawing, read by
/// the DXF reader, and the direct reading of AutoCAD's DWG of the same drawing: what the DWG
/// reading lost, added or changed.
fn deep(dwg: &str, dxf: &str) {
    let name = Path::new(dwg).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (Ok(a), Ok(b)) = (std::fs::read(dwg), std::fs::read(dxf)) else {
        println!("DEEP {name}: can't read the files");
        return;
    };
    let (direct, other) = match (read_dwg_with(&a, DwgReader::Direct), draftcraft_io::read(&b, "other.dxf")) {
        (Ok(d), Ok(o)) => (d, o),
        (Err(e), _) => return println!("DEEP {name}: DIRECT READ FAILED: {e}"),
        (_, Err(e)) => return println!("DEEP {name}: the DXF doesn't read: {e}"),
    };
    let cmp = draftcraft_io::roundtrip::compare(&other, &direct, &draftcraft_io::roundtrip::Options::default());
    let t = cmp.total();
    println!("DEEP {name}: {} lost, {} added, {} changed (DWG reading vs the DXF)", t.lost, t.added, t.changed);
    for d in &cmp.differences {
        println!("  [{}] {} {}: {}", d.category.label(), d.change.label(), d.place, d.detail);
    }
    if cmp.omitted > 0 {
        println!("  … {} more", cmp.omitted);
    }
    println!("  DWG notes: {}", direct.read_notes.join(" | "));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--deep") {
        for pair in args.get(1..).unwrap_or(&[]).chunks(2) {
            if let [dwg, dxf] = pair {
                deep(dwg, dxf);
            }
        }
        return;
    }
    if args.first().map(String::as_str) == Some("--read") {
        // Only the direct reading of saved files: its time and the process's peak memory.
        for f in args.iter().skip(1) {
            let Ok(bytes) = std::fs::read(f) else {
                println!("READ {f}: can't read the file");
                continue;
            };
            let t = std::time::Instant::now();
            let reader = if std::env::var("SWEEP_READER").is_ok_and(|r| r == "acadrust") { DwgReader::Acadrust } else { DwgReader::Direct };
            match read_dwg_with(&bytes, reader) {
                Ok(d) => {
                    let n: usize = kinds(&d).values().sum();
                    println!(
                        "READ {f}: {:.1} MB, {n} entities, {} layers, {} blocks in {:?}, peak memory {}",
                        bytes.len() as f64 / 1048576.0,
                        d.layers.len(),
                        d.blocks.len(),
                        t.elapsed(),
                        peak_mb()
                    );
                }
                Err(e) => println!("READ {f}: FAILED after {:?}: {e}", t.elapsed()),
            }
        }
        return;
    }
    if args.first().map(String::as_str) == Some("--vs-dxf") {
        vs_dxf(args.get(1).map_or("", String::as_str), args.get(2).map_or("", String::as_str));
        return;
    }
    if args.first().map(String::as_str) == Some("--stress") {
        let case = args.get(1).cloned().unwrap_or_default();
        let n = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
        let version = match args.get(3).map(String::as_str) {
            Some("2000") => Version::R2000,
            Some("2004") => Version::R2004,
            Some("2010") => Version::R2010,
            Some("2013") => Version::R2013,
            _ => Version::R2018,
        };
        stress(&case, n, version);
        return;
    }
    let Some((out, files)) = args.split_first() else {
        eprintln!("usage: direct_sweep <out-dir> <files…>");
        std::process::exit(2);
    };
    std::fs::create_dir_all(out).ok();
    let mut report = Vec::new();
    let mut failures = 0;
    let mut sources: Vec<(String, Drawing)> = Vec::new();
    for f in files {
        let name = Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Ok(bytes) = std::fs::read(f) else {
            report.push(format!("{name}: can't read the file"));
            failures += 1;
            continue;
        };
        if draftcraft_dwg::is_dwg(&bytes) {
            report.push(format!("{name} (AutoCAD DWG)"));
            let t = std::time::Instant::now();
            let direct = read_dwg_with(&bytes, DwgReader::Direct);
            let ms = t.elapsed().as_millis();
            let reference = read_dwg_with(&bytes, DwgReader::Acadrust);
            match (&direct, &reference) {
                (Ok(d), Ok(r)) => {
                    report.push(format!("  direct read in {ms} ms; notes: {}", d.read_notes.join(" | ")));
                    if !compare("direct vs acadrust", d, r, &mut report) {
                        failures += 1;
                    }
                    // Stability: direct → DXF → read.
                    match write_with(d, "x.dxf", &SaveOptions::default()).and_then(|dxf| draftcraft_io::read(&dxf, "x.dxf")) {
                        Ok(back) => {
                            if !compare("direct → DXF → read", &back, d, &mut report) {
                                failures += 1;
                            }
                        }
                        Err(e) => {
                            failures += 1;
                            report.push(format!("  direct → DXF failed: {e}"));
                        }
                    }
                    if let Ok(png) = draftcraft_io::write(d, "x.png") {
                        std::fs::write(Path::new(out).join(format!("{name}.direct.png")), png).ok();
                    }
                    if let Ok(png) = draftcraft_io::write(r, "x.png") {
                        std::fs::write(Path::new(out).join(format!("{name}.acadrust.png")), png).ok();
                    }
                }
                (Err(e), _) => {
                    failures += 1;
                    report.push(format!("  direct read failed: {e}"));
                }
                (Ok(_), Err(e)) => report.push(format!("  (acadrust failed: {e}; nothing to compare)")),
            }
        } else {
            match draftcraft_io::read(&bytes, &name) {
                Ok(d) => sources.push((name, d)),
                Err(e) => {
                    failures += 1;
                    report.push(format!("{name}: DXF read failed: {e}"));
                }
            }
        }
    }
    for (name, d) in &sources {
        report.push(format!("{name} (DraftCraft's drawing saved as DWG)"));
        for version in [Version::R2000, Version::R2004, Version::R2010, Version::R2013, Version::R2018] {
            let label = format!("{version:?}");
            match write_with(d, "x.dwg", &SaveOptions { version: Some(version), binary: false }) {
                Ok(dwg) => match read_dwg_with(&dwg, DwgReader::Direct) {
                    Ok(back) => {
                        let saved = draftcraft_io::read(&write_with(d, "x.dxf", &SaveOptions::default()).unwrap_or_default(), "x.dxf")
                            .unwrap_or_else(|_| d.clone());
                        if compare(&format!("{label}: read back vs saved"), &back, &saved, &mut report) {
                            report.push(format!("  {label}: same; notes: {}", back.read_notes.join(" | ")));
                        } else {
                            failures += 1;
                            report.push(format!("  {label}: notes: {}", back.read_notes.join(" | ")));
                        }
                    }
                    Err(e) => {
                        failures += 1;
                        report.push(format!("  {label}: direct read of the saved DWG failed: {e}"));
                    }
                },
                Err(e) => report.push(format!("  {label}: save failed: {e}")),
            }
        }
    }
    let text = report.join("\n");
    println!("{text}\n\n{failures} failing checks");
    std::fs::write(Path::new(out).join("report.txt"), format!("{text}\n\n{failures} failing checks\n")).ok();
}
