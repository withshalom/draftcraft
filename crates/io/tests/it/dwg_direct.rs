//! The direct DWG reader (`draftcraft_io::dwg`) on the MI-201 test drawings: one part saved by
//! AutoCAD in every DWG version, each with AutoCAD's own DXF of the same drawing as the reference.
//!
//! The reader is built category by category. These tests hold it to its account of the file: every
//! object is either in the drawing or counted as unread, and a damaged file is an error or a
//! drawing, never a panic.

use draftcraft_dwg::file::DwgFile;
use draftcraft_io::dwg;

/// The MI-201 DWG files the reader opens, with their version codes.
const DWGS: &[(&str, &str, &[u8])] = &[
    ("mi201-r14.dwg", "AC1014", include_bytes!("../data/mi201/mi201-r14.dwg")),
    ("mi201-r2000.dwg", "AC1015", include_bytes!("../data/mi201/mi201-r2000.dwg")),
    ("mi201-r2004.dwg", "AC1018", include_bytes!("../data/mi201/mi201-r2004.dwg")),
    ("mi201-r2007.dwg", "AC1021", include_bytes!("../data/mi201/mi201-r2007.dwg")),
    ("mi201-r2010.dwg", "AC1024", include_bytes!("../data/mi201/mi201-r2010.dwg")),
    ("mi201-r2013.dwg", "AC1027", include_bytes!("../data/mi201/mi201-r2013.dwg")),
    ("mi201-r2018.dwg", "AC1032", include_bytes!("../data/mi201/mi201-r2018.dwg")),
    ("mi201-r2018-from-ascii-dxf.dwg", "AC1032", include_bytes!("../data/mi201/mi201-r2018-from-ascii-dxf.dwg")),
    ("mi201-r2018-from-binary-dxf.dwg", "AC1032", include_bytes!("../data/mi201/mi201-r2018-from-binary-dxf.dwg")),
];

/// AutoCAD's DXF of each version: the references the reader is compared with as it grows.
const DXFS: &[(&str, &[u8])] = &[
    ("mi201-r12.dxf", include_bytes!("../data/mi201/mi201-r12.dxf")),
    ("mi201-r2000.dxf", include_bytes!("../data/mi201/mi201-r2000.dxf")),
    ("mi201-r2004.dxf", include_bytes!("../data/mi201/mi201-r2004.dxf")),
    ("mi201-r2007.dxf", include_bytes!("../data/mi201/mi201-r2007.dxf")),
    ("mi201-r2010.dxf", include_bytes!("../data/mi201/mi201-r2010.dxf")),
    ("mi201-r2013.dxf", include_bytes!("../data/mi201/mi201-r2013.dxf")),
    ("mi201-r2018.dxf", include_bytes!("../data/mi201/mi201-r2018.dxf")),
];

#[test]
fn every_object_of_every_version_is_accounted_for() {
    for (name, code, bytes) in DWGS {
        let file = DwgFile::open(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(file.version_code(), *code, "{name}");
        assert!(file.object_count() > 100, "{name}: {} objects", file.object_count());
        let (d, unread) = dwg::read(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(d.header.str("ACADVER", ""), *code, "{name}");
        // Every object is in the drawing or counted as unread, a type at a time: the reader reads
        // all the objects of a type or counts them all, and the types read so far leave none
        // behind. A key with a space is a part of an object that waits for a later step (a
        // header variable's name, an entity's reactors), or the objects of a type the codec
        // can't decode yet ("… (not decoded)").
        let mut types: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for o in file.objects() {
            *types.entry(o.type_name.clone()).or_default() += 1;
        }
        let unread_of =
            |t: &str| unread.by_type.get(t).copied().unwrap_or(0) + unread.by_type.get(&format!("{t} (not decoded)")).copied().unwrap_or(0);
        for (t, n) in &types {
            let u = unread_of(t);
            assert!(u == 0 || u == *n, "{name}: {u} of the {n} {t} unread: {}", unread.summary());
            assert!(u == 0 || !READ_SO_FAR.contains(&t.as_str()), "{name}: {t} left unread: {}", unread.summary());
        }
        // Nothing is counted that the file doesn't have.
        for k in unread.by_type.keys() {
            let t = k.strip_suffix(" (not decoded)").unwrap_or(k);
            assert!(types.contains_key(t) || (t.contains(' ') && t == k), "{name}: {k} counted, not in the file: {}", unread.summary());
        }
        assert!(!unread.by_type.contains_key("?"), "{name}: {}", unread.summary());
        // A type of a later step is still counted, not dropped: the codec decodes visual styles
        // from AutoCAD 2013 on.
        if *code < "AC1027" {
            assert_eq!(unread_of("VISUALSTYLE"), types.get("VISUALSTYLE").copied().unwrap_or(0), "{name}: {}", unread.summary());
            assert!(unread_of("VISUALSTYLE") > 0, "{name}: no VISUALSTYLE among {}", unread.summary());
        }
    }
}

/// The object types the reader puts in the drawing so far.
const READ_SO_FAR: &[&str] = &[
    "DICTIONARY",
    "ACDBDICTIONARYWDFLT",
    "ACDBPLACEHOLDER",
    "XRECORD",
    "SCALE",
    "DICTIONARYVAR",
    "GROUP",
    "WIPEOUTVARIABLES",
    "RASTERVARIABLES",
    "TABLESTYLE",
    "MLEADERSTYLE",
    "DIMENSION_LINEAR",
    "DIMASSOC",
    "LINE",
    "CIRCLE",
    "ARC",
    "TEXT",
    "POINT",
    "SOLID",
    "MTEXT",
    "ELLIPSE",
    "LWPOLYLINE",
    "TRACE",
    "3DFACE",
    "XLINE",
    "RAY",
    "APPID",
    "STYLE",
    "LTYPE",
    "LAYER",
    "DIMSTYLE",
    "UCS",
    "VIEW",
    "VPORT",
    "BLOCK_HEADER",
    "BLOCK",
    "ENDBLK",
    "LAYOUT",
    "BLOCK_CONTROL",
    "LAYER_CONTROL",
    "STYLE_CONTROL",
    "LTYPE_CONTROL",
    "VIEW_CONTROL",
    "UCS_CONTROL",
    "VPORT_CONTROL",
    "APPID_CONTROL",
    "DIMSTYLE_CONTROL",
    "VX_CONTROL",
    "MLINESTYLE",
];

#[test]
fn the_references_read() {
    for (name, bytes) in DXFS {
        let d = draftcraft_io::read(bytes, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(d.model.len(), 34, "{name}: model-space entities");
    }
}

#[test]
fn damaged_files_never_panic() {
    for (_, _, bytes) in DWGS {
        for cut in [0, 5, 6, 64, 128, 1024, bytes.len() / 2, bytes.len() - 1] {
            let _ = dwg::read(&bytes[..cut.min(bytes.len())]);
        }
        let mut b = bytes.to_vec();
        for i in (0..b.len()).step_by(b.len() / 251 + 1) {
            b[i] ^= 0x5A;
            let _ = dwg::read(&b);
            b[i] ^= 0x5A;
        }
    }
}

/// Each DWG with AutoCAD's DXF of the same version (R14's is the R12 DXF).
fn pairs() -> Vec<(&'static str, &'static [u8], &'static [u8])> {
    let dxf = |name: &str| DXFS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b).unwrap();
    DWGS.iter()
        .map(|(name, _, bytes)| {
            let reference = match *name {
                "mi201-r14.dwg" => "mi201-r12.dxf",
                "mi201-r2000.dwg" => "mi201-r2000.dxf",
                "mi201-r2004.dwg" => "mi201-r2004.dxf",
                "mi201-r2007.dwg" => "mi201-r2007.dxf",
                "mi201-r2010.dwg" => "mi201-r2010.dxf",
                "mi201-r2013.dwg" => "mi201-r2013.dxf",
                _ => "mi201-r2018.dxf",
            };
            (*name, *bytes, dxf(reference))
        })
        .collect()
}

/// Header variables that belong to one save of a file, not to the drawing: AutoCAD wrote the DWG
/// and its DXF at different moments, so these differ between the two.
const OF_THE_SAVE: &[&str] = &["ACADVER", "ACADMAINTVER", "HANDSEED", "TDUPDATE", "TDUUPDATE", "TDINDWG", "TDUSRTIMER", "VERSIONGUID", "DWGCODEPAGE"];

/// The two dates in local time: from AutoCAD 2000 on a DWG stores them in universal time only
/// (TDUCREATE, TDUUPDATE), and the offset AutoCAD used is not in the file.
const LOCAL_TIMES: &[&str] = &["TDCREATE", "TDUPDATE"];

/// The default visual styles of interference checking: AutoCAD's DXF names them, its DWG holds
/// null references.
const NULL_IN_DWG: &[&str] = &["INTERFEREOBJVS", "INTERFEREVPVS"];

/// Variables that name a table record, a block or a UCS. They are set when those are read; until
/// then the reader must count each as unread and leave it out of the header.
const NAMES: &[&str] = &[
    "CLAYER",
    "CELTYPE",
    "TEXTSTYLE",
    "DIMSTYLE",
    "CMLSTYLE",
    "DIMTXSTY",
    "DIMBLK",
    "DIMBLK1",
    "DIMBLK2",
    "DIMLDRBLK",
    "DIMLTYPE",
    "DIMLTEX1",
    "DIMLTEX2",
    "UCSNAME",
    "UCSBASE",
    "UCSORTHOREF",
    "PUCSNAME",
    "PUCSBASE",
    "PUCSORTHOREF",
];

/// Where the R14 DWG and the R12 DXF, the nearest reference AutoCAD writes, are different files
/// of different formats: R12 has a variable for whether handles are on (a later DWG always has
/// them), and AutoCAD wrote 64 as the R12 file's $MAXACTVP where the R14 file holds 48.
const R12_DIFFERS: &[&str] = &["HANDLING", "MAXACTVP"];

fn same(a: &draftcraft_doc::HVal, b: &draftcraft_doc::HVal) -> bool {
    use draftcraft_doc::HVal::*;
    match (a, b) {
        (Real(x), Real(y)) => (x - y).abs() <= 1e-9 * (1.0 + y.abs()),
        (Int(i), Real(r)) | (Real(r), Int(i)) => (*i as f64 - r).abs() < 1e-9,
        (Point(p), Point(q)) => (*p - *q).len() < 1e-9,
        _ => a == b,
    }
}

#[test]
fn header_variables_match_autocads_dxf() {
    for (name, dwg_bytes, dxf_bytes) in pairs() {
        let (d, unread) = dwg::read(dwg_bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let reference = draftcraft_io::read(dxf_bytes, "reference.dxf").unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!unread.by_type.contains_key("header"), "{name}: the header was not read");
        let before_2000 = name.contains("r14");
        let mut wrong = Vec::new();
        let mut compared = 0;
        for (var, want) in &reference.header.vars {
            let v = var.as_str();
            if OF_THE_SAVE.contains(&v)
                || NULL_IN_DWG.contains(&v)
                || (!before_2000 && LOCAL_TIMES.contains(&v))
                || (before_2000 && R12_DIFFERS.contains(&v))
            {
                continue;
            }
            // A name the file gives as a reference waits for its table; one it gives as text
            // (R14's $DIMBLK) is compared like any other variable.
            if NAMES.contains(&v) && unread.by_type.contains_key(&format!("header ${v}")) {
                assert!(d.header.get(v).is_none(), "{name}: ${v} is set before its table is read");
                continue;
            }
            // The two DWGs AutoCAD made from DXFs have handles of their own, so a variable that
            // holds a handle differs from the reference of the original drawing.
            if v == "CMATERIAL" && name.contains("-from-") {
                assert!(d.header.get(v).is_some(), "{name}");
                continue;
            }
            // A 2004 or 2007 DXF holds $XCLIPFRAME as a yes/no and keeps the drawing's real value,
            // 2, in a dictionary variable; the DWG's header holds the 2 itself, as later DXFs do.
            if v == "XCLIPFRAME" && (name.contains("r2004") || name.contains("r2007")) {
                assert_eq!(d.header.i64(v, -1), 2, "{name}");
                continue;
            }
            compared += 1;
            match d.header.get(v) {
                Some(got) if same(got, want) => {}
                got => wrong.push(format!("${v}: read {got:?}, AutoCAD's DXF has {want:?}")),
            }
        }
        assert!(wrong.is_empty(), "{name}: {} of {compared} header variables differ:\n{}", wrong.len(), wrong.join("\n"));
        assert!(compared > 60, "{name}: only {compared} variables compared");
    }
}

#[test]
fn layers_linetypes_and_text_styles_match_autocads_dxf() {
    for (name, dwg_bytes, dxf_bytes) in pairs() {
        let (d, _) = dwg::read(dwg_bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let reference = draftcraft_io::read(dxf_bytes, "reference.dxf").unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(reference.layers.len() >= 2 && reference.linetypes.len() >= 3 && !reference.text_styles.is_empty(), "{name}");
        assert_eq!(d.layers, reference.layers, "{name}: layers");
        assert_eq!(d.linetypes, reference.linetypes, "{name}: linetypes");
        assert_eq!(d.text_styles, reference.text_styles, "{name}: text styles");
        for var in ["CLAYER", "CELTYPE", "TEXTSTYLE", "DIMTXSTY"] {
            // The R12 DXF, R14's reference, has no $DIMTXSTY.
            if let Some(want) = reference.header.get(var) {
                assert_eq!(d.header.get(var), Some(want), "{name}: ${var}");
            }
        }
    }
}

#[test]
fn dimension_styles_views_ucss_and_viewports_match_autocads_dxf() {
    for (name, dwg_bytes, dxf_bytes) in pairs() {
        let (d, _) = dwg::read(dwg_bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let reference = draftcraft_io::read(dxf_bytes, "reference.dxf").unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!reference.dim_styles.is_empty(), "{name}");
        assert_eq!(d.dim_styles.len(), reference.dim_styles.len(), "{name}: dimension styles");
        for (got, want) in d.dim_styles.iter().zip(&reference.dim_styles) {
            let (g, w) = (serde_json::to_value(got).unwrap(), serde_json::to_value(want).unwrap());
            let diff: Vec<String> = w
                .as_object()
                .unwrap()
                .iter()
                .filter(|(k, v)| g.get(k.as_str()) != Some(*v))
                .map(|(k, v)| format!("{k}: read {:?}, AutoCAD's DXF has {v:?}", g.get(k.as_str())))
                .collect();
            assert!(diff.is_empty(), "{name}: dimension style {}:\n{}", want.name, diff.join("\n"));
        }
        assert_eq!(d.views, reference.views, "{name}: views");
        assert_eq!(d.ucss, reference.ucss, "{name}: UCSs");
        assert_eq!(d.vports, reference.vports, "{name}: viewport configurations");
        for var in ["DIMSTYLE", "UCSNAME", "PUCSNAME", "DIMBLK", "DIMBLK1", "DIMBLK2", "DIMLDRBLK", "SNAPUNIT", "GRIDUNIT", "SNAPBASE", "SNAPANG"] {
            if let Some(want) = reference.header.get(var) {
                assert!(
                    d.header.get(var).is_some_and(|got| same(got, want)),
                    "{name}: ${var}: read {:?}, AutoCAD's DXF has {want:?}",
                    d.header.get(var)
                );
            }
        }
    }
}

#[test]
fn blocks_and_layouts_match_autocads_dxf() {
    for (name, dwg_bytes, dxf_bytes) in pairs() {
        let (d, _) = dwg::read(dwg_bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let reference = draftcraft_io::read(dxf_bytes, "reference.dxf").unwrap_or_else(|e| panic!("{name}: {e}"));
        // Layouts: the tabs in order, each with its page setup, limits and saved view. Their
        // entities are read with the entities.
        let tabs = |d: &draftcraft_doc::Drawing| {
            d.layouts.iter().map(|l| (l.name.clone(), l.tab_order, l.page.clone(), l.limits, l.view)).collect::<Vec<_>>()
        };
        if name.contains("r14") {
            // An R12 DXF has no layouts, so its reading has the default page setup; the R14 DWG
            // holds the layout with its own. Only the tabs can be compared.
            let names = |d: &draftcraft_doc::Drawing| d.layouts.iter().map(|l| (l.name.clone(), l.tab_order)).collect::<Vec<_>>();
            assert_eq!(names(&d), names(&reference), "{name}: layouts");
        } else {
            assert_eq!(tabs(&d), tabs(&reference), "{name}: layouts");
        }
        assert_eq!(d.active_layout, reference.active_layout, "{name}: active layout");
        // Block definitions: the same blocks, each with its own properties. An R12 DXF, R14's
        // reference, also has the two spaces as blocks ($MODEL_SPACE, $PAPER_SPACE) and the
        // blocks AutoCAD made of what R12 cannot hold (*U2, *U3), which the R14 DWG does not.
        assert!(!reference.blocks.is_empty(), "{name}");
        if !name.contains("r14") {
            assert_eq!(d.blocks.keys().collect::<Vec<_>>(), reference.blocks.keys().collect::<Vec<_>>(), "{name}: blocks");
        }
        for (block, got) in &d.blocks {
            let want = reference.blocks.get(block).unwrap_or_else(|| panic!("{name}: block {block} is not in the reference"));
            let props = |b: &draftcraft_doc::Block| {
                (b.base, b.description.clone(), b.anonymous, b.xref_path.clone(), b.xref_overlay, b.explodable, b.units, b.annotative)
            };
            assert_eq!(props(got), props(want), "{name}: block {block}");
        }
    }
}

/// The entity types the reader puts in the drawing so far.
fn read_so_far(k: &draftcraft_doc::EntityKind) -> bool {
    use draftcraft_doc::EntityKind::*;
    match k {
        Line(_) | Point(_) | Circle(_) | Arc(_) | Ellipse(_) | LwPolyline(_) | Text(_) | MText(_) | Solid(_) | Trace(_) | Face3d(_) | XLine(_)
        | Ray(_) | Dimension(_) => true,
        Insert(i) => i.attribs.is_empty(),
        _ => false,
    }
}

/// Every entity with where it lives, in order: `model`, `layout NAME` or `block NAME`.
fn placed(d: &draftcraft_doc::Drawing) -> Vec<(String, std::sync::Arc<draftcraft_doc::Entity>)> {
    let mut v: Vec<_> = d.model.iter().map(|e| ("model".to_string(), e.clone())).collect();
    for l in &d.layouts {
        v.extend(l.entities.iter().map(|e| (format!("layout {}", l.name), e.clone())));
    }
    for (n, b) in &d.blocks {
        v.extend(b.entities.iter().map(|e| (format!("block {n}"), e.clone())));
    }
    v
}

/// Two values equal but for the last digits of their numbers: a DXF holds a number as text of
/// 16 digits, a DWG holds the exact double. With `any_case`, names are compared without case too
/// (an R12 DXF leaves out a text's style when it is the standard one, which then reads as
/// `Standard`).
fn close(a: &serde_json::Value, b: &serde_json::Value, any_case: bool) -> bool {
    use serde_json::Value::*;
    match (a, b) {
        (Number(x), Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() <= 1e-9 * (1.0 + y.abs()),
            _ => x == y,
        },
        (String(x), String(y)) if any_case => x.eq_ignore_ascii_case(y),
        (Array(x), Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| close(p, q, any_case)),
        (Object(x), Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| close(v, w, any_case))),
        _ => a == b,
    }
}

#[test]
fn simple_entities_match_autocads_dxf() {
    for (name, dwg_bytes, dxf_bytes) in pairs() {
        // R14's reference is an R12 DXF, where AutoCAD turned what R12 cannot hold into other
        // entities; and the two DWGs made from DXFs have other handles. Those are compared by
        // content and order, the others by handle too.
        let by_handle = !name.contains("r14") && !name.contains("-from-");
        let (d, _) = dwg::read(dwg_bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let reference = draftcraft_io::read(dxf_bytes, "reference.dxf").unwrap_or_else(|e| panic!("{name}: {e}"));
        let r14 = name.contains("r14");
        // R12 has no MTEXT: AutoCAD wrote each one into the R12 DXF as an anonymous block (*U2,
        // *U3) and an insert of it. For R14 the MTEXTs read and those inserts are set aside.
        let converted = |e: &draftcraft_doc::Entity| matches!(&e.kind, draftcraft_doc::EntityKind::Insert(i) if i.block.starts_with("*U"));
        let is_mtext = |e: &draftcraft_doc::Entity| matches!(e.kind, draftcraft_doc::EntityKind::MText(_));
        let got: Vec<_> = placed(&d).into_iter().filter(|(_, e)| !(r14 && is_mtext(e))).collect();
        let want: Vec<_> = placed(&reference)
            .into_iter()
            .filter(|(place, e)| read_so_far(&e.kind) && !place.starts_with("block $") && !place.starts_with("block *U") && !(r14 && converted(e)))
            .collect();
        if r14 {
            let mtexts = placed(&d).iter().filter(|(_, e)| is_mtext(e)).count();
            let inserts = placed(&reference).iter().filter(|(_, e)| converted(e)).count();
            assert_eq!(mtexts, inserts, "{name}: one MTEXT for each converted block");
        }
        assert!(want.len() >= 30, "{name}: {} reference entities", want.len());
        let show = |v: &[(String, std::sync::Arc<draftcraft_doc::Entity>)]| {
            v.iter().map(|(p, e)| format!("{p} {:X} {}", e.handle.0, e.kind.type_name())).collect::<Vec<_>>()
        };
        assert_eq!(got.len(), want.len(), "{name}:\nread {:?}\nreference {:?}", show(&got), show(&want));
        for ((gp, g), (wp, w)) in got.iter().zip(&want) {
            assert_eq!(gp, wp, "{name}: place of {:X}", w.handle.0);
            if by_handle {
                assert_eq!(g.handle, w.handle, "{name}: handle in {wp}");
            }
            let (mut g, mut w) = (serde_json::to_value(g.as_ref()).unwrap(), serde_json::to_value(w.as_ref()).unwrap());
            // The R14 DWG also carries extended data that R12 has no place for (AutoCAD keeps an
            // R14 entity's lineweight there): compared without it.
            if r14
                && w["common"].get("xdata").is_none()
                && let Some(c) = g["common"].as_object_mut()
            {
                c.remove("xdata");
            }
            // So do a dimension's style override in it (AutoCAD keeps an R14 dimension's text
            // style there) and its associativity, which R12 has no DIMASSOC object for.
            // The two DWGs AutoCAD made from DXFs hold no DIMASSOC objects (the DXFs they were
            // made from had none), while their reference is the original drawing's DXF.
            if name.contains("-from-") {
                assert!(g["kind"].get("assoc").is_none(), "{name}: associativity read from a file that holds none");
                if let Some(k) = w.get("kind").cloned().as_mut().and_then(|k| k.as_object_mut()) {
                    k.remove("assoc");
                    w["kind"] = serde_json::Value::Object(k.clone());
                }
            }
            for key in ["overrides", "assoc"] {
                if r14
                    && w["kind"].get(key).is_none()
                    && let Some(k) = g["kind"].as_object_mut()
                {
                    k.remove(key);
                }
            }
            assert!(close(&g["kind"], &w["kind"], r14), "{name}: {wp}\nread {}\nreference {}", g["kind"], w["kind"]);
            assert!(close(&g["common"], &w["common"], r14), "{name}: properties in {wp}\nread {}\nreference {}", g["common"], w["common"]);
        }
    }
}

/// A drawing with attributes in every mode: block TAG with five definitions (plain, invisible,
/// constant, every other mode, and a centred one with a prompt), and two references to it, one
/// plain and one mirrored, scaled and rotated, whose attributes carry their own layer, colour,
/// linetype and lineweight.
fn attributes_drawing() -> draftcraft_doc::Drawing {
    use draftcraft_doc::*;
    use draftcraft_geom::Vec3;
    let text = |value: &str, x: f64, halign: HAlign| Text {
        insert: Vec3::new(x, 2.0, 0.0),
        align_pt: (halign != HAlign::Left).then(|| Vec3::new(x + 3.0, 2.0, 0.0)),
        height: 0.25,
        value: value.into(),
        rotation: 0.0,
        width_factor: 1.0,
        oblique: 0.0,
        style: "Standard".into(),
        halign,
        valign: VAlign::Baseline,
    };
    let attrib = |tag: &str, x: f64| Attrib {
        tag: tag.into(),
        text: text(tag, x, HAlign::Left),
        invisible: false,
        constant: false,
        prompt: String::new(),
        props: AttribProps::default(),
        multiline: None,
        modes: AttribModes::default(),
    };
    let defs = vec![
        attrib("PLAIN", 0.0),
        Attrib { invisible: true, ..attrib("HIDDEN", 1.0) },
        Attrib { constant: true, ..attrib("FIXED", 2.0) },
        Attrib { modes: AttribModes { verify: true, preset: true, lock_position: true }, ..attrib("MODES", 3.0) },
        Attrib { prompt: "Drawing number?".into(), text: text("NUMBER", 4.0, HAlign::Center), ..attrib("NUMBER", 4.0) },
    ];
    let mut d = Drawing::new_imperial();
    d.ensure_layer("TITLE");
    d.ensure_layer("NOTES");
    let mut b = Block::new("TAG");
    for a in &defs {
        let h = d.new_handle();
        b.entities.push(Entity::new(h, EntityKind::AttDef(a.clone())));
    }
    d.blocks.insert("TAG".into(), std::sync::Arc::new(b));
    let values = |own: bool| -> Vec<Attrib> {
        defs.iter()
            .filter(|a| !a.constant)
            .map(|a| Attrib {
                prompt: String::new(),
                text: Text { value: format!("{} value", a.tag), ..a.text.clone() },
                props: if own {
                    AttribProps {
                        layer: Some("NOTES".into()),
                        color: Some(draftcraft_color::Color::Index(1)),
                        linetype: Some("Continuous".into()),
                        lineweight: Some(Lineweight::from_dxf(35)),
                    }
                } else {
                    AttribProps::default()
                },
                ..a.clone()
            })
            .collect()
    };
    for (at, own, scale, rotation) in [(Vec3::new(10.0, 10.0, 0.0), false, 1.0, 0.0), (Vec3::new(30.0, 5.0, 0.0), true, 2.0, 0.5)] {
        let ins = Insert {
            block: "TAG".into(),
            insert: at,
            scale: Vec3::new(scale, scale, 1.0),
            rotation,
            attribs: values(own),
            cols: 1,
            rows: 1,
            col_spacing: 0.0,
            row_spacing: 0.0,
            clip: None,
        };
        let common = Common { layer: "TITLE".into(), ..Common::default() };
        d.add(&Space::Model, common, EntityKind::Insert(ins)).expect("add");
    }
    d
}

/// The attribute definitions of every block and the attributes of every block reference, with
/// their places, for comparing two readings.
fn attributes_of(d: &draftcraft_doc::Drawing) -> Vec<(String, serde_json::Value)> {
    use draftcraft_doc::EntityKind;
    placed(d)
        .into_iter()
        .filter_map(|(place, e)| match &e.kind {
            EntityKind::AttDef(a) => Some((place, serde_json::to_value(a).unwrap())),
            EntityKind::Insert(i) if !i.attribs.is_empty() => {
                Some((place, serde_json::json!({ "block": i.block, "common": e.common, "attribs": i.attribs })))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn attributes_read_as_the_acadrust_reader_reads_them() {
    use draftcraft_io::{DwgReader, SaveOptions, Version, read_dwg_with, write_with};
    let source = attributes_drawing();
    for version in [Version::R2000, Version::R2004, Version::R2007, Version::R2010, Version::R2013, Version::R2018] {
        let bytes = write_with(&source, "t.dwg", &SaveOptions { version: Some(version), binary: false }).unwrap();
        let (d, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        let reference = read_dwg_with(&bytes, DwgReader::Acadrust).unwrap();
        let want = attributes_of(&reference);
        assert_eq!(want.len(), 7, "{version:?}: the reference has the five definitions and the two references: {want:#?}");
        let got = attributes_of(&d);
        assert_eq!(got.len(), want.len(), "{version:?}: unread {}", unread.summary());
        for ((gp, g), (wp, w)) in got.iter().zip(&want) {
            assert_eq!(gp, wp, "{version:?}");
            assert!(close(g, w, false), "{version:?}: {wp}\nread      {g}\nreference {w}");
        }
        for t in ["ATTRIB", "ATTDEF", "SEQEND", "INSERT"] {
            assert!(!unread.by_type.keys().any(|k| k.starts_with(t)), "{version:?}: {t} left unread: {}", unread.summary());
        }
    }
}

/// The multiline attributes fixture of `multiline_attributes.rs` (a block with a multiline
/// definition, and a reference with its own multiline text), as DXF.
fn multiline_fixture() -> String {
    let src = include_str!("multiline_attributes.rs");
    let start = src.find("const FIXTURE: &str = \"").map(|i| i + "const FIXTURE: &str = \"".len()).unwrap();
    let end = start + src[start..].find("\";").unwrap();
    let mut dxf = String::new();
    for line in src[start..end].lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (code, value) = line.split_once(' ').unwrap_or((line, ""));
        dxf.push_str(&format!("{code}\n{value}\n"));
    }
    dxf
}

/// Multiline attributes (AutoCAD 2018 DWG) embed an MTEXT in the record: an entity's common
/// properties, then the MTEXT's fields (AutoCAD's own files show the layout: ACadSharp's samples,
/// [`autocad_samples_entities_read_as_autocads_dxf`]). DraftCraft's saved ones read back as the
/// drawing has them: the definition with its multiline text, the reference with its own. (The
/// acadrust reader loses the reference's vertical alignment; the writer places an aligned text's
/// insertion point itself, from its alignment point, so that is not compared.)
#[test]
fn multiline_attributes_read_back_as_saved() {
    use draftcraft_io::{SaveOptions, Version, write_with};
    let d = draftcraft_io::read(multiline_fixture().as_bytes(), "m.dxf").unwrap();
    // An aligned text's insertion point is the writer's: left out of both sides.
    let unplaced = |list: Vec<(String, serde_json::Value)>| -> Vec<(String, serde_json::Value)> {
        let strip = |a: &mut serde_json::Value| {
            if let Some(t) = a.get_mut("text").and_then(|t| t.as_object_mut())
                && t.get("alignPt").is_some_and(|p| !p.is_null())
            {
                t.remove("insert");
            }
        };
        list.into_iter()
            .map(|(p, mut v)| {
                if let Some(list) = v.get_mut("attribs").and_then(|a| a.as_array_mut()) {
                    list.iter_mut().for_each(strip);
                } else {
                    strip(&mut v);
                }
                (p, v)
            })
            .collect()
    };
    let want = unplaced(attributes_of(&d));
    assert!(!want.is_empty(), "the fixture has its multiline attributes");
    let bytes = write_with(&d, "t.dwg", &SaveOptions { version: Some(Version::R2018), binary: false }).unwrap();
    let (read, unread) = dwg::read(&bytes).unwrap();
    for what in ["ATTDEF (not decoded)", "ATTRIB (not decoded)", "INSERT whose attributes were not all read"] {
        assert_eq!(unread.by_type.get(what), None, "{what}: {}", unread.summary());
    }
    let got = unplaced(attributes_of(&read));
    assert_eq!(got.len(), want.len(), "{got:#?}");
    for ((gp, g), (wp, w)) in got.iter().zip(&want) {
        assert_eq!(gp, wp);
        assert!(close(g, w, false), "{gp}\nread  {g}\nsaved {w}");
    }
}

/// A DWG of `version` holding every kind of polyline and both kinds of spline, written with
/// acadrust's own API: DraftCraft writes 2D polylines and polygon meshes in other forms, so its
/// save path can't make them.
fn curves_dwg(version: acadrust::types::DxfVersion) -> Vec<u8> {
    use acadrust::entities::*;
    use acadrust::types::Vector3;
    let mut doc = acadrust::CadDocument::with_version(version);
    // 2D polylines: open with bulges and widths at an elevation, closed, and mirrored.
    for (normal_z, closed, y) in [(1.0, false, 0.0), (1.0, true, 20.0), (-1.0, false, 40.0)] {
        let mut p = Polyline2D::new();
        p.elevation = 1.5;
        p.normal = Vector3::new(0.0, 0.0, normal_z);
        p.start_width = 0.25;
        p.end_width = 0.25;
        p.add_vertex(Vertex2D::new(Vector3::new(0.0, y, 0.0)).with_bulge(0.5).with_width(0.1, 0.3));
        p.add_vertex(Vertex2D::new(Vector3::new(10.0, y, 0.0)).with_width(0.3, 0.1));
        p.add_vertex(Vertex2D::new(Vector3::new(10.0, y + 10.0, 0.0)).with_bulge(-1.0));
        p.add_vertex(Vertex2D::new(Vector3::new(0.0, y + 10.0, 0.0)));
        if closed {
            p.close();
        }
        doc.add_entity(EntityType::Polyline2D(p)).unwrap();
    }
    // A closed 3D polyline.
    let mut p3 = Polyline3D::from_points(vec![
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(5.0, 0.0, 2.0),
        Vector3::new(5.0, 5.0, 4.0),
        Vector3::new(0.0, 5.0, 6.0),
    ]);
    p3.flags.closed = true;
    doc.add_entity(EntityType::Polyline3D(p3)).unwrap();
    // A polyface mesh: a quad and a triangle over five vertices.
    let mut pf = PolyfaceMesh::new();
    for (x, y, z) in [(0.0, 0.0, 0.0), (4.0, 0.0, 0.0), (4.0, 4.0, 1.0), (0.0, 4.0, 1.0), (2.0, 6.0, 2.0)] {
        pf.add_vertex_xyz(x + 50.0, y, z);
    }
    pf.add_face(PolyfaceFace::quad(1, 2, 3, 4));
    pf.add_face(PolyfaceFace::triangle(3, 5, 4));
    doc.add_entity(EntityType::PolyfaceMesh(pf)).unwrap();
    // A 3 × 4 polygon mesh, closed in N.
    let mut pm = PolygonMeshEntity::new();
    pm.m_vertex_count = 3;
    pm.n_vertex_count = 4;
    pm.flags = PolygonMeshFlags::POLYGON_MESH | PolygonMeshFlags::CLOSED_N;
    for i in 0..3 {
        for j in 0..4 {
            pm.vertices.push(PolygonMeshVertex::at(Vector3::new(70.0 + f64::from(i) * 3.0, f64::from(j) * 2.0, f64::from(i * j))));
        }
    }
    doc.add_entity(EntityType::PolygonMesh(pm)).unwrap();
    // A rational spline by its control points, and a spline by its fit points with end tangents.
    let control = vec![Vector3::new(0.0, 60.0, 0.0), Vector3::new(5.0, 70.0, 0.0), Vector3::new(10.0, 55.0, 0.0), Vector3::new(15.0, 65.0, 0.0)];
    let mut sc = Spline::from_control_points(3, control);
    sc.weights = vec![1.0, 2.0, 0.5, 1.0];
    sc.flags.rational = true;
    doc.add_entity(EntityType::Spline(sc)).unwrap();
    let fit = vec![Vector3::new(30.0, 60.0, 0.0), Vector3::new(35.0, 66.0, 0.0), Vector3::new(42.0, 58.0, 0.0), Vector3::new(48.0, 64.0, 0.0)];
    let mut sf = Spline::from_fit_points(fit);
    sf.begin_tangent = Vector3::new(1.0, 1.0, 0.0);
    sf.end_tangent = Vector3::new(1.0, -1.0, 0.0);
    doc.add_entity(EntityType::Spline(sf)).unwrap();
    acadrust::DwgWriter::write_to_vec(&doc).unwrap()
}

#[test]
fn polylines_and_splines_read_as_the_acadrust_reader_reads_them() {
    use acadrust::types::DxfVersion;
    use draftcraft_doc::EntityKind;
    use draftcraft_io::{DwgReader, read_dwg_with};
    let curve = |k: &EntityKind| {
        matches!(
            k,
            EntityKind::LwPolyline(_) | EntityKind::Polyline3d(_) | EntityKind::PolyfaceMesh(_) | EntityKind::Face3d(_) | EntityKind::Spline(_)
        )
    };
    for version in [DxfVersion::AC1015, DxfVersion::AC1018, DxfVersion::AC1021, DxfVersion::AC1024, DxfVersion::AC1027, DxfVersion::AC1032] {
        let bytes = curves_dwg(version);
        let (d, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        let reference = read_dwg_with(&bytes, DwgReader::Acadrust).unwrap();
        let want: Vec<_> = reference.model.iter().filter(|e| curve(&e.kind)).collect();
        // Three 2D polylines, a 3D one, a polyface mesh, the 3 × 4 mesh closed in N as 2 × 4 faces,
        // two splines.
        assert_eq!(want.len(), 15, "{version:?}: {:?}", want.iter().map(|e| e.kind.type_name()).collect::<Vec<_>>());
        let got: Vec<_> = d.model.iter().filter(|e| curve(&e.kind)).collect();
        assert_eq!(got.len(), want.len(), "{version:?}: unread {}", unread.summary());
        for (g, w) in got.iter().zip(&want) {
            // acadrust's DXF gives a vertex stored with no width the polyline's default width and
            // leaves the default out, so 2D polylines are checked against what was written.
            if matches!(g.kind, EntityKind::LwPolyline(_)) {
                continue;
            }
            // From AutoCAD 2013 on a fit spline's tangents are its end derivatives, length included
            // (AutoCAD's own file and DXF show it: SPLINE 434 of ACadSharp's samples), where the
            // acadrust reader takes them as directions. The fit spline here stores (1, 1) and
            // (1, -1): AutoCAD's curve, the same fit points.
            if let (EntityKind::Spline(gs), EntityKind::Spline(ws)) = (&g.kind, &w.kind)
                && !ws.fit.is_empty()
                && matches!(version, DxfVersion::AC1027 | DxfVersion::AC1032)
            {
                use draftcraft_geom::{FitOptions, Spline, Vec2};
                let opts = FitOptions { knots: ws.fit_opts.knots, tolerance: ws.fit_opts.tolerance, ..FitOptions::default() };
                let autocad = Spline::fit_with_derivatives(&ws.fit, opts, Some(Vec2::new(1.0, 1.0)), Some(Vec2::new(1.0, -1.0)));
                assert_eq!(gs.fit, ws.fit, "{version:?}");
                assert_eq!(gs.control.len(), autocad.control.len(), "{version:?}");
                for (a, b) in gs.control.iter().zip(&autocad.control) {
                    assert!(a.near(*b, 1e-9), "{version:?}: control point {a:?}, AutoCAD's {b:?}");
                }
                continue;
            }
            let (g, w) = (serde_json::to_value(g.as_ref()).unwrap(), serde_json::to_value(w.as_ref()).unwrap());
            assert!(close(&g["kind"], &w["kind"], false), "{version:?}\nread      {}\nreference {}", g["kind"], w["kind"]);
            assert!(close(&g["common"], &w["common"], false), "{version:?}: properties\nread      {}\nreference {}", g["common"], w["common"]);
        }
        // The 2D polylines as they were written: the vertices with their own widths and bulges,
        // the default width, the elevation, closed or not; the mirrored one with x and the bulges
        // turned round and its extrusion kept.
        let written = |closed: bool, y: f64, m: f64| {
            let v = |x: f64, y: f64, bulge: f64, sw: f64, ew: f64| draftcraft_geom::PolyVertex {
                p: draftcraft_geom::Vec2::new(m * x, y),
                bulge: m * bulge,
                start_width: sw,
                end_width: ew,
            };
            draftcraft_doc::LwPolyline {
                vertices: vec![
                    v(0.0, y, 0.5, 0.1, 0.3),
                    v(10.0, y, 0.0, 0.3, 0.1),
                    v(10.0, y + 10.0, -1.0, 0.0, 0.0),
                    v(0.0, y + 10.0, 0.0, 0.0, 0.0),
                ],
                closed,
                const_width: 0.25,
                elevation: 1.5,
                plinegen: false,
            }
        };
        let polylines: Vec<_> =
            d.model.iter().filter_map(|e| if let EntityKind::LwPolyline(p) = &e.kind { Some((p, e.common.extrusion.z)) } else { None }).collect();
        let expected = [(written(false, 0.0, 1.0), 1.0), (written(true, 20.0, 1.0), 1.0), (written(false, 40.0, -1.0), -1.0)];
        assert_eq!(polylines.len(), 3, "{version:?}");
        for ((got, z), (want, wz)) in polylines.iter().zip(&expected) {
            assert_eq!(*got, want, "{version:?}");
            assert_eq!(*z, *wz, "{version:?}: extrusion");
        }
        for t in ["POLYLINE", "VERTEX", "SEQEND", "SPLINE", "polyline", "vertex"] {
            assert!(!unread.by_type.keys().any(|k| k.starts_with(t)), "{version:?}: {t} left unread: {}", unread.summary());
        }
    }
}

/// A drawing with one dimension of every kind: linear (rotated, with oblique extension lines and an
/// override), aligned with text the user placed and wrote, both angular kinds, radius, diameter,
/// both ordinate kinds, and a mirrored linear one.
fn dimensions_drawing() -> draftcraft_doc::Drawing {
    use draftcraft_doc::*;
    use draftcraft_geom::Vec3;
    let p = |x: f64, y: f64| Vec3::new(x, y, 0.0);
    let dim = |kind: DimKind, defpt: Vec3, text_mid: Vec3, p13: Vec3, p14: Vec3, p15: Vec3, p16: Vec3| Dimension {
        kind,
        defpt,
        text_mid,
        p13,
        p14,
        p15,
        p16,
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
    };
    let mut d = Drawing::new_imperial();
    let mut linear = dim(DimKind::Linear { rotation: 0.3 }, p(10.0, 5.0), p(5.0, 5.0), p(0.0, 0.0), p(10.0, 2.0), Vec3::ZERO, Vec3::ZERO);
    linear.marks.oblique = Some(0.3 + 1.0);
    linear.overrides.insert("textHeight".into(), serde_json::json!(0.3));
    let mut aligned = dim(DimKind::Aligned, p(32.0, 6.0), p(28.0, 8.0), p(20.0, 0.0), p(30.0, 4.0), Vec3::ZERO, Vec3::ZERO);
    aligned.text = "<> typ.".into();
    aligned.user_text_pos = true;
    let kinds = vec![
        linear,
        aligned,
        dim(DimKind::Angular, p(45.0, 8.0), p(46.0, 9.0), p(40.0, 0.0), p(48.0, 0.0), p(40.0, 0.0), p(46.0, 6.0)),
        dim(DimKind::Angular3P, p(58.0, 8.0), p(58.0, 9.0), p(55.0, 0.0), p(62.0, 3.0), p(54.0, 2.0), Vec3::ZERO),
        dim(DimKind::Radius, p(70.0, 0.0), p(73.0, 2.0), Vec3::ZERO, Vec3::ZERO, p(74.0, 3.0), Vec3::ZERO),
        dim(DimKind::Diameter, p(80.0, 0.0), p(84.0, 3.0), Vec3::ZERO, Vec3::ZERO, p(86.0, 0.0), Vec3::ZERO),
        dim(DimKind::Ordinate { x_type: true }, p(0.0, 20.0), p(5.0, 30.0), p(5.0, 22.0), p(5.0, 28.0), Vec3::ZERO, Vec3::ZERO),
        dim(DimKind::Ordinate { x_type: false }, p(0.0, 20.0), p(15.0, 25.0), p(8.0, 25.0), p(14.0, 25.0), Vec3::ZERO, Vec3::ZERO),
    ];
    for k in kinds {
        d.add(&Space::Model, Common::default(), EntityKind::Dimension(k)).expect("add");
    }
    d
}

#[test]
fn every_kind_of_dimension_reads_as_the_acadrust_reader_reads_it() {
    use draftcraft_doc::EntityKind;
    use draftcraft_io::{DwgReader, SaveOptions, Version, read_dwg_with, write_with};
    let source = dimensions_drawing();
    for version in [Version::R2000, Version::R2004, Version::R2007, Version::R2010, Version::R2013, Version::R2018] {
        let bytes = write_with(&source, "t.dwg", &SaveOptions { version: Some(version), binary: false }).unwrap();
        let (d, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        let reference = read_dwg_with(&bytes, DwgReader::Acadrust).unwrap();
        let dims = |d: &draftcraft_doc::Drawing| d.model.iter().filter(|e| matches!(e.kind, EntityKind::Dimension(_))).cloned().collect::<Vec<_>>();
        let (got, want) = (dims(&d), dims(&reference));
        assert_eq!(want.len(), 8, "{version:?}");
        assert_eq!(got.len(), want.len(), "{version:?}: unread {}", unread.summary());
        for (ge, we) in got.iter().zip(&want) {
            let (mut g, mut w) = (serde_json::to_value(ge.as_ref()).unwrap(), serde_json::to_value(we.as_ref()).unwrap());
            // DWG stores anonymous blocks unnumbered: this reader numbers them as AutoCAD does in a
            // DXF (*D0 first), acadrust from *D1. The dimension's block must hold the same.
            let block = |d: &draftcraft_doc::Drawing, v: &serde_json::Value| {
                let b = d.blocks.get(v["kind"]["block"].as_str().unwrap_or_default());
                b.map(|b| b.entities.iter().map(|e| serde_json::to_value(&e.kind).unwrap()).collect::<Vec<_>>())
            };
            let (gb, wb) = (block(&d, &g), block(&reference, &w));
            assert!(wb.is_some(), "{version:?}: the reference's dimension has a block");
            assert!(close(&serde_json::json!(gb), &serde_json::json!(wb), false), "{version:?}: the dimension's block");
            g["kind"]["block"] = serde_json::Value::Null;
            w["kind"]["block"] = serde_json::Value::Null;
            assert!(close(&g["kind"], &w["kind"], false), "{version:?}\nread      {}\nreference {}", g["kind"], w["kind"]);
            assert!(close(&g["common"], &w["common"], false), "{version:?}: properties\nread      {}\nreference {}", g["common"], w["common"]);
        }
        assert!(!unread.by_type.keys().any(|k| k.starts_with("DIMENSION")), "{version:?}: {}", unread.summary());
    }
}

/// A drawing with hatches of every kind of boundary and fill: a solid fill whose outer loop has a
/// bulge and an island, a scaled and rotated pattern with its own origin, a pattern the library
/// doesn't know (its definition lines kept), a gradient, a background colour, and an associative
/// hatch whose loop names the circle it was made from.
fn hatches_drawing() -> draftcraft_doc::Drawing {
    use draftcraft_doc::library::PatternLine;
    use draftcraft_doc::*;
    use draftcraft_geom::{PolyVertex, Vec2};
    let v = |x: f64, y: f64| PolyVertex::new(Vec2::new(x, y));
    let square = |x: f64, y: f64, s: f64| vec![v(x, y), v(x + s, y), v(x + s, y + s), v(x, y + s)];
    let hatch = |pattern: &str, loops: Vec<HatchLoop>| Hatch {
        pattern: pattern.into(),
        solid: pattern == "SOLID",
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
    let outer = |vertices: Vec<PolyVertex>| HatchLoop { vertices, outer: true, sources: Vec::new() };
    let mut d = Drawing::new_imperial();
    let circle = d
        .add(&Space::Model, Common::default(), EntityKind::Circle(Circle { center: draftcraft_geom::Vec3::new(60.0, 5.0, 0.0), radius: 4.0 }))
        .expect("circle");
    let mut solid_loop = square(0.0, 0.0, 10.0);
    solid_loop[1].bulge = 0.4;
    let mut solid = hatch("SOLID", vec![outer(solid_loop), HatchLoop { vertices: square(3.0, 3.0, 2.0), outer: false, sources: Vec::new() }]);
    solid.elevation = 1.5;
    let mut ansi = hatch("ANSI31", vec![outer(square(15.0, 0.0, 10.0))]);
    ansi.scale = 2.0;
    ansi.angle = 0.5;
    ansi.origin = Vec2::new(1.0, 2.0);
    ansi.style = 1;
    ansi.background = Some(color::Color::Index(3));
    let mut own = hatch("MYGRID", vec![outer(square(30.0, 0.0, 10.0))]);
    own.pattern_lines = vec![
        PatternLine { angle: 0.0, origin: (0.0, 0.0), delta: (0.0, 0.5), dashes: vec![0.25, -0.25] },
        PatternLine { angle: 90.0, origin: (0.1, 0.0), delta: (0.0, 0.5), dashes: Vec::new() },
    ];
    own.scale = 1.5;
    let mut gradient = hatch("SOLID", vec![outer(square(45.0, 0.0, 10.0))]);
    gradient.gradient = Some(Gradient {
        name: "CYLINDER".into(),
        color1: color::Color::Index(1),
        color2: color::Color::True(color::Rgb(10, 200, 30)),
        angle: 0.25,
        centered: false,
    });
    let mut associative = hatch("SOLID", vec![HatchLoop { vertices: Vec::new(), outer: true, sources: vec![circle] }]);
    associative.associative = true;
    if let Some(l) = associative.loops.first_mut() {
        l.vertices = vec![PolyVertex::with_bulge(Vec2::new(56.0, 5.0), 1.0), PolyVertex::with_bulge(Vec2::new(64.0, 5.0), 1.0)];
    }
    let mut by_layer = hatch("SOLID", vec![outer(square(70.0, 0.0, 10.0))]);
    by_layer.gradient =
        Some(Gradient { name: "CURVED".into(), color1: color::Color::Index(5), color2: color::Color::ByLayer, angle: 0.0, centered: true });
    for h in [solid, ansi, own, gradient, associative, by_layer] {
        d.add(&Space::Model, Common::default(), EntityKind::Hatch(h)).expect("add");
    }
    d
}

#[test]
fn hatches_read_as_the_acadrust_reader_reads_them() {
    use draftcraft_doc::EntityKind;
    use draftcraft_io::{DwgReader, SaveOptions, Version, read_dwg_with, write_with};
    let source = hatches_drawing();
    for version in [Version::R2000, Version::R2004, Version::R2007, Version::R2010, Version::R2013, Version::R2018] {
        let bytes = write_with(&source, "t.dwg", &SaveOptions { version: Some(version), binary: false }).unwrap();
        let (d, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        let reference = read_dwg_with(&bytes, DwgReader::Acadrust).unwrap();
        let hatches = |d: &draftcraft_doc::Drawing| d.model.iter().filter(|e| matches!(e.kind, EntityKind::Hatch(_))).cloned().collect::<Vec<_>>();
        let (got, want) = (hatches(&d), hatches(&reference));
        assert_eq!(want.len(), 6, "{version:?}");
        assert_eq!(got.len(), want.len(), "{version:?}: unread {}", unread.summary());
        for (ge, we) in got.iter().zip(&want) {
            let (g, w) = (serde_json::to_value(ge.as_ref()).unwrap(), serde_json::to_value(we.as_ref()).unwrap());
            assert!(close(&g["kind"], &w["kind"], false), "{version:?}\nread      {}\nreference {}", g["kind"], w["kind"]);
            assert!(close(&g["common"], &w["common"], false), "{version:?}: properties\nread      {}\nreference {}", g["common"], w["common"]);
        }
        assert!(!unread.by_type.contains_key("HATCH"), "{version:?}: {}", unread.summary());
    }
}

/// Hatches whose loops are edges, written through acadrust's own API (DraftCraft writes polyline
/// loops): lines, a counter-clockwise and clockwise arcs, an elliptic arc ending on its axes, and
/// a spline, with seed points, a derived loop (pixel size) and a pattern's own definition lines.
fn hatch_edges_dwg(version: acadrust::types::DxfVersion) -> Vec<u8> {
    use acadrust::entities::*;
    use acadrust::types::Vector2;
    use std::f64::consts::{FRAC_PI_2, PI};
    let mut doc = acadrust::CadDocument::with_version(version);
    let p = |x: f64, y: f64| Vector2::new(x, y);
    let line = |a: Vector2, b: Vector2| BoundaryEdge::Line(LineEdge { start: a, end: b });
    let arc = |c: Vector2, r: f64, s: f64, e: f64, ccw: bool| {
        BoundaryEdge::CircularArc(CircularArcEdge { center: c, radius: r, start_angle: s, end_angle: e, counter_clockwise: ccw })
    };
    // Line, arc, line, half an ellipse (its major axis up, so it bows out to the left).
    let mut a = BoundaryPath::with_flags(BoundaryPathFlags::EXTERNAL);
    a.add_edge(line(p(100.0, 0.0), p(110.0, 0.0)));
    a.add_edge(arc(p(110.0, 5.0), 5.0, -FRAC_PI_2, FRAC_PI_2, true));
    a.add_edge(line(p(110.0, 10.0), p(100.0, 10.0)));
    a.add_edge(BoundaryEdge::EllipticArc(EllipticArcEdge {
        center: p(100.0, 5.0),
        major_axis_endpoint: p(0.0, 5.0),
        minor_axis_ratio: 0.4,
        start_angle: 0.0,
        end_angle: PI,
        counter_clockwise: true,
    }));
    // A spline and the line that closes it.
    let mut b = BoundaryPath::with_flags(BoundaryPathFlags::EXTERNAL);
    b.add_edge(BoundaryEdge::Spline(SplineEdge {
        degree: 3,
        rational: false,
        periodic: false,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            acadrust::types::Vector3::new(120.0, 0.0, 1.0),
            acadrust::types::Vector3::new(122.0, 5.0, 1.0),
            acadrust::types::Vector3::new(128.0, 5.0, 1.0),
            acadrust::types::Vector3::new(130.0, 0.0, 1.0),
        ],
        fit_points: Vec::new(),
        start_tangent: Vector2::ZERO,
        end_tangent: Vector2::ZERO,
    }));
    b.add_edge(line(p(130.0, 0.0), p(120.0, 0.0)));
    // A circle as two clockwise arcs, derived (a pixel size follows the loops).
    let mut c = BoundaryPath::with_flags(BoundaryPathFlags::from_bits(1 | 4));
    c.add_edge(arc(p(140.0, 5.0), 5.0, 0.0, PI, false));
    c.add_edge(arc(p(140.0, 5.0), 5.0, PI, 2.0 * PI, false));
    let mut solid = Hatch::solid();
    solid.add_path(a);
    solid.add_seed_point(p(105.0, 5.0));
    doc.add_entity(EntityType::Hatch(Box::new(solid))).unwrap();
    let mut spline = Hatch::solid();
    spline.add_path(b);
    doc.add_entity(EntityType::Hatch(Box::new(spline))).unwrap();
    let mut pattern = HatchPattern::new("STRIPES");
    pattern.add_line(HatchPatternLine { angle: 0.3, base_point: p(140.0, 5.0), offset: p(-0.2, 0.6), dash_lengths: vec![0.5, -0.25] });
    pattern.add_line(HatchPatternLine { angle: 0.3 + FRAC_PI_2, base_point: p(140.0, 5.0), offset: p(0.0, 1.0), dash_lengths: Vec::new() });
    let mut patterned = Hatch::with_pattern(pattern);
    patterned.is_solid = false;
    patterned.pattern_type = HatchPatternType::Custom;
    patterned.pattern_angle = 0.3;
    patterned.pattern_scale = 2.0;
    patterned.pixel_size = 0.05;
    patterned.add_path(c);
    patterned.add_seed_point(p(140.0, 5.0));
    patterned.add_seed_point(p(141.0, 5.0));
    doc.add_entity(EntityType::Hatch(Box::new(patterned))).unwrap();
    acadrust::DwgWriter::write_to_vec(&doc).unwrap()
}

#[test]
fn hatch_edges_read_as_the_acadrust_reader_reads_them() {
    use acadrust::types::DxfVersion as A;
    use draftcraft_doc::EntityKind;
    use draftcraft_io::{DwgReader, read_dwg_with};
    for version in [A::AC1015, A::AC1018, A::AC1021, A::AC1024, A::AC1027, A::AC1032] {
        let bytes = hatch_edges_dwg(version);
        let (d, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        let reference = read_dwg_with(&bytes, DwgReader::Acadrust).unwrap();
        let hatches = |d: &draftcraft_doc::Drawing| d.model.iter().filter(|e| matches!(e.kind, EntityKind::Hatch(_))).cloned().collect::<Vec<_>>();
        let (got, want) = (hatches(&d), hatches(&reference));
        assert_eq!(want.len(), 3, "{version:?}");
        assert_eq!(got.len(), want.len(), "{version:?}: unread {}", unread.summary());
        for (ge, we) in got.iter().zip(&want) {
            let (g, w) = (serde_json::to_value(ge.as_ref()).unwrap(), serde_json::to_value(we.as_ref()).unwrap());
            assert!(close(&g["kind"], &w["kind"], false), "{version:?}\nread      {}\nreference {}", g["kind"], w["kind"]);
            let EntityKind::Hatch(h) = &ge.kind else { unreachable!() };
            assert!(h.loops.iter().all(|l| l.vertices.len() >= 2), "{version:?}: every edge loop has its vertices");
        }
        let EntityKind::Hatch(patterned) = &got[2].kind else { unreachable!() };
        assert_eq!(patterned.pattern_lines.len(), 2, "{version:?}");
        assert!(!unread.by_type.contains_key("HATCH"), "{version:?}: {}", unread.summary());
    }
}

/// A drawing with annotation: leaders (straight with an arrowhead, a spline without one, in a
/// dimension style of its own), a feature control frame, and multileaders with text content, with
/// a block whose attribute has its own value, smoothed as a spline, with two leader lines, and
/// with none.
fn annotation_drawing() -> draftcraft_doc::Drawing {
    use draftcraft_doc::*;
    use draftcraft_geom::Vec3;
    let p = |x: f64, y: f64| Vec3::new(x, y, 0.0);
    let mut d = Drawing::new_imperial();
    let mut own_style = d.dim_styles.first().cloned().expect("a dimension style");
    own_style.name = "NOTES".into();
    d.dim_styles.push(own_style);
    let mut tag = Block::new("CALLOUT");
    let def = Attrib {
        tag: "NO".into(),
        text: Text {
            insert: p(0.0, 0.0),
            align_pt: None,
            height: 0.2,
            value: "1".into(),
            rotation: 0.0,
            width_factor: 1.0,
            oblique: 0.0,
            style: "Standard".into(),
            halign: HAlign::Left,
            valign: VAlign::Baseline,
        },
        invisible: false,
        constant: false,
        prompt: String::new(),
        props: AttribProps::default(),
        multiline: None,
        modes: AttribModes::default(),
    };
    let def_handle = d.new_handle();
    tag.entities.push(Entity::new(def_handle, EntityKind::AttDef(def.clone())));
    d.blocks.insert("CALLOUT".into(), std::sync::Arc::new(tag));
    let leaders = [
        Leader { vertices: vec![p(0.0, 0.0), p(5.0, 5.0), p(8.0, 5.0)], arrow: true, spline: false, style: "Standard".into() },
        Leader { vertices: vec![p(10.0, 0.0), p(12.0, 4.0), p(15.0, 5.0), p(18.0, 5.0)], arrow: false, spline: true, style: "NOTES".into() },
    ];
    for l in leaders {
        d.add(&Space::Model, Common::default(), EntityKind::Leader(l)).expect("leader");
    }
    let frame =
        Tolerance { insert: p(30.0, 0.0), dir: Vec3::new(0.0, 1.0, 0.0), style: "NOTES".into(), text: "{\\Fgdt;j}%%v0.05{\\Fgdt;m}%%vA%%vB".into() };
    d.add(&Space::Model, Common::default(), EntityKind::Tolerance(frame)).expect("tolerance");
    let text = |at: Vec3, contents: &str| MText {
        insert: at,
        height: 0.18,
        width: 0.0,
        attach: 1,
        rotation: 0.0,
        style: "Standard".into(),
        contents: contents.into(),
        line_spacing: 1.0,
        line_spacing_exact: false,
        columns: None,
    };
    let ml = |leaders: Vec<Vec<Vec3>>, landing: Vec3, text: Option<MText>| MLeader {
        leaders,
        landing,
        dogleg: 0.36,
        text,
        style: "Standard".into(),
        arrow_size: 0.18,
        spline: false,
        block: None,
    };
    d.mleader_styles.push(MLeaderStyle { name: "CALLOUTS".into(), ..MLeaderStyle::default() });
    let mut plain = ml(vec![vec![p(40.0, 0.0)]], p(44.0, 3.0), Some(text(p(44.5, 3.1), "SEE NOTE 4")));
    plain.style = "CALLOUTS".into();
    let two = ml(vec![vec![p(50.0, 0.0)], vec![p(52.0, -1.0), p(53.0, 1.0)]], p(55.0, 3.0), Some(text(p(55.5, 3.1), "TYP.\\P2 PLACES")));
    let mut curved = ml(vec![vec![p(60.0, 0.0), p(62.0, 2.0)]], p(65.0, 3.0), Some(text(p(65.5, 3.1), "WELD")));
    curved.spline = true;
    let none = ml(Vec::new(), p(75.0, 3.0), Some(text(p(75.0, 3.0), "NO LEADER")));
    let mut block = ml(vec![vec![p(80.0, 0.0)]], p(84.0, 3.0), None);
    block.block = Some(Insert {
        block: "CALLOUT".into(),
        insert: p(84.5, 3.0),
        scale: Vec3::new(1.0, 1.0, 1.0),
        rotation: 0.0,
        attribs: vec![Attrib { text: Text { value: "7".into(), insert: p(84.5, 3.0), ..def.text.clone() }, ..def.clone() }],
        cols: 1,
        rows: 1,
        col_spacing: 0.0,
        row_spacing: 0.0,
        clip: None,
    });
    for m in [plain, two, curved, none, block] {
        d.add(&Space::Model, Common::default(), EntityKind::MLeader(m)).expect("multileader");
    }
    d
}

#[test]
fn leaders_tolerances_and_multileaders_read_as_the_acadrust_reader_reads_them() {
    use draftcraft_doc::EntityKind;
    use draftcraft_io::{DwgReader, SaveOptions, Version, read_dwg_with, write_with};
    let source = annotation_drawing();
    for version in [Version::R2000, Version::R2004, Version::R2007, Version::R2010, Version::R2013, Version::R2018] {
        let bytes = write_with(&source, "t.dwg", &SaveOptions { version: Some(version), binary: false }).unwrap();
        let (d, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        let reference = read_dwg_with(&bytes, DwgReader::Acadrust).unwrap();
        // Up to 2004 a multileader is saved as a leader and its text or block reference: every
        // entity is compared. From 2007 it is a MULTILEADER.
        let (got, want) = (d.model.iter().cloned().collect::<Vec<_>>(), reference.model.iter().cloned().collect::<Vec<_>>());
        let count = |f: fn(&EntityKind) -> bool| (got.iter().filter(|e| f(&e.kind)).count(), want.iter().filter(|e| f(&e.kind)).count());
        assert_eq!(count(|k| matches!(k, EntityKind::Tolerance(_))), (1, 1), "{version:?}");
        let multileaders = if version >= Version::R2007 { 5 } else { 0 };
        assert_eq!(count(|k| matches!(k, EntityKind::MLeader(_))), (multileaders, multileaders), "{version:?}");
        assert_eq!(got.len(), want.len(), "{version:?}: unread {}", unread.summary());
        let with_value = got.iter().filter_map(|e| match &e.kind {
            EntityKind::MLeader(m) => m.block.as_ref(),
            EntityKind::Insert(i) => Some(i),
            _ => None,
        });
        assert!(with_value.flat_map(|i| &i.attribs).any(|a| a.text.value == "7"), "{version:?}: the block's attribute value");
        if version >= Version::R2010 {
            assert!(got.iter().any(|e| matches!(&e.kind, EntityKind::MLeader(m) if m.style == "CALLOUTS")), "{version:?}: the multileader style");
        }
        for (ge, we) in got.iter().zip(&want) {
            let (g, w) = (serde_json::to_value(ge.as_ref()).unwrap(), serde_json::to_value(we.as_ref()).unwrap());
            assert!(close(&g["kind"], &w["kind"], false), "{version:?}\nread      {}\nreference {}", g["kind"], w["kind"]);
            assert!(close(&g["common"], &w["common"], false), "{version:?}: properties\nread      {}\nreference {}", g["common"], w["common"]);
        }
        for t in ["LEADER", "TOLERANCE", "MULTILEADER"] {
            assert!(!unread.by_type.keys().any(|k| k.starts_with(t)), "{version:?}: {}", unread.summary());
        }
    }
}

/// Mirrored text, written through acadrust's API (DraftCraft keeps text readable and never saves
/// it mirrored): texts and paragraph texts whose plane faces down, left-aligned and aligned on a
/// second point, rotated, beside their upright twins.
fn mirrored_text_dwg(version: acadrust::types::DxfVersion) -> Vec<u8> {
    use acadrust::entities::*;
    use acadrust::types::Vector3;
    let mut doc = acadrust::CadDocument::with_version(version);
    for (z, y) in [(1.0, 0.0), (-1.0, 10.0)] {
        let mut t = Text::with_value("MIRRORED", Vector3::new(5.0, y, 0.0));
        t.height = 0.5;
        t.rotation = 0.3;
        t.normal = Vector3::new(0.0, 0.0, z);
        doc.add_entity(EntityType::Text(t)).unwrap();
        let mut c = Text::with_value("CENTRED", Vector3::new(5.0, y + 2.0, 0.0));
        c.height = 0.5;
        c.horizontal_alignment = TextHorizontalAlignment::Center;
        c.alignment_point = Some(Vector3::new(8.0, y + 2.0, 0.0));
        c.normal = Vector3::new(0.0, 0.0, z);
        doc.add_entity(EntityType::Text(c)).unwrap();
        let mut m = MText::with_value("PARAGRAPH\\PTWO LINES", Vector3::new(5.0, y + 5.0, 0.0));
        m.height = 0.4;
        m.rectangle_width = 6.0;
        m.rotation = 0.2;
        m.attachment_point = AttachmentPoint::TopLeft;
        m.normal = Vector3::new(0.0, 0.0, z);
        doc.add_entity(EntityType::MText(Box::new(m))).unwrap();
    }
    acadrust::DwgWriter::write_to_vec(&doc).unwrap()
}

#[test]
fn mirrored_text_reads_as_the_acadrust_reader_reads_it() {
    use acadrust::types::DxfVersion as A;
    use draftcraft_doc::EntityKind;
    use draftcraft_io::{DwgReader, read_dwg_with};
    for version in [A::AC1015, A::AC1018, A::AC1021, A::AC1024, A::AC1027, A::AC1032] {
        let bytes = mirrored_text_dwg(version);
        let (d, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        let reference = read_dwg_with(&bytes, DwgReader::Acadrust).unwrap();
        let texts = |d: &draftcraft_doc::Drawing| {
            d.model.iter().filter(|e| matches!(e.kind, EntityKind::Text(_) | EntityKind::MText(_))).cloned().collect::<Vec<_>>()
        };
        let (got, want) = (texts(&d), texts(&reference));
        assert_eq!(want.len(), 6, "{version:?}");
        assert_eq!(got.len(), want.len(), "{version:?}: unread {}", unread.summary());
        for (ge, we) in got.iter().zip(&want) {
            let (g, w) = (serde_json::to_value(ge.as_ref()).unwrap(), serde_json::to_value(we.as_ref()).unwrap());
            assert!(close(&g["kind"], &w["kind"], false), "{version:?}\nread      {}\nreference {}", g["kind"], w["kind"]);
            assert!(close(&g["common"], &w["common"], false), "{version:?}: properties\nread      {}\nreference {}", g["common"], w["common"]);
        }
        // The mirrored ones are not the upright ones read again.
        assert_ne!(serde_json::to_value(&got[0].kind).unwrap(), serde_json::to_value(&got[3].kind).unwrap(), "{version:?}");
    }
}

#[test]
fn damaged_entity_drawings_never_panic() {
    use draftcraft_io::{SaveOptions, Version, write_with};
    for source in
        [annotation_drawing(), hatches_drawing(), viewports_drawing(), images_drawing(), mlines_drawing(), tables_drawing(), regions_drawing()]
    {
        for version in [Version::R2000, Version::R2018] {
            let bytes = write_with(&source, "t.dwg", &SaveOptions { version: Some(version), binary: false }).unwrap();
            let mut b = bytes.clone();
            for i in (0..b.len()).step_by(b.len() / 1200 + 1) {
                for flip in [0x01, 0xFF] {
                    b[i] ^= flip;
                    let _ = dwg::read(&b);
                    b[i] ^= flip;
                }
            }
        }
    }
}

/// A layout with viewports: plain, locked, off, twisted with its view off-centre, with frozen
/// layers (one without a layer record) and layer colour overrides, and one clipped by a circle.
fn viewports_drawing() -> draftcraft_doc::Drawing {
    use draftcraft_doc::*;
    use draftcraft_geom::{Vec2, Vec3};
    let mut d = Drawing::new_imperial();
    d.ensure_layer("WALLS");
    d.ensure_layer("NOTES");
    d.add(&Space::Model, Common::default(), EntityKind::Line(Line { a: Vec3::ZERO, b: Vec3::new(100.0, 50.0, 0.0) })).expect("line");
    let paper = Space::Paper(d.layouts.first().map(|l| l.name.clone()).expect("a layout"));
    let vp = |x: f64, view: Vec2| Viewport {
        center: Vec3::new(x, 4.0, 0.0),
        width: 4.0,
        height: 3.0,
        view_center: view,
        view_height: 40.0,
        id: 0,
        locked: false,
        frozen_layers: Vec::new(),
        layer_colors: Vec::new(),
        clip: None,
        on: true,
        twist: 0.0,
    };
    let circle = d.add(&paper, Common::default(), EntityKind::Circle(Circle { center: Vec3::new(30.0, 4.0, 0.0), radius: 1.4 })).expect("circle");
    let viewports = [
        vp(5.0, Vec2::new(50.0, 25.0)),
        Viewport { locked: true, ..vp(10.0, Vec2::new(20.0, 10.0)) },
        Viewport { on: false, ..vp(15.0, Vec2::new(30.0, 10.0)) },
        Viewport { twist: 0.4, ..vp(20.0, Vec2::new(60.0, 35.0)) },
        Viewport {
            frozen_layers: vec!["WALLS".into(), "GONE".into()],
            layer_colors: vec![("NOTES".into(), draftcraft_color::Color::Index(3))],
            ..vp(25.0, Vec2::new(10.0, 5.0))
        },
        Viewport { clip: Some(circle), ..vp(30.0, Vec2::new(40.0, 20.0)) },
    ];
    for v in viewports {
        d.add(&paper, Common::default(), EntityKind::Viewport(v)).expect("viewport");
    }
    d
}

#[test]
fn viewports_read_as_the_acadrust_reader_reads_them() {
    use draftcraft_io::{DwgReader, SaveOptions, Version, read_dwg_with, write_with};
    let source = viewports_drawing();
    for version in [Version::R2000, Version::R2004, Version::R2007, Version::R2010, Version::R2013, Version::R2018] {
        let bytes = write_with(&source, "t.dwg", &SaveOptions { version: Some(version), binary: false }).unwrap();
        let (d, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        let reference = read_dwg_with(&bytes, DwgReader::Acadrust).unwrap();
        let sheet =
            |d: &draftcraft_doc::Drawing| d.layouts.iter().flat_map(|l| l.entities.iter().cloned().map(|e| (l.name.clone(), e))).collect::<Vec<_>>();
        let (got, want) = (sheet(&d), sheet(&reference));
        let viewports = want.iter().filter(|(_, e)| matches!(e.kind, draftcraft_doc::EntityKind::Viewport(_))).count();
        assert!(viewports >= 6, "{version:?}: {viewports}");
        assert_eq!(got.len(), want.len(), "{version:?}: unread {}", unread.summary());
        for ((gl, ge), (wl, we)) in got.iter().zip(&want) {
            assert_eq!(gl, wl, "{version:?}");
            let (g, w) = (serde_json::to_value(ge.as_ref()).unwrap(), serde_json::to_value(we.as_ref()).unwrap());
            assert!(close(&g["kind"], &w["kind"], false), "{version:?}\nread      {}\nreference {}", g["kind"], w["kind"]);
            assert!(close(&g["common"], &w["common"], false), "{version:?}: properties\nread      {}\nreference {}", g["common"], w["common"]);
        }
        assert!(!unread.by_type.keys().any(|k| k.starts_with("VIEWPORT")), "{version:?}: {}", unread.summary());
    }
}

/// Images (whole, clipped by a rectangle, clipped by a polygon, faded and dimmed), a wipeout and
/// a PDF underlay (rotated, scaled, on page 2, clipped, monochrome).
fn images_drawing() -> draftcraft_doc::Drawing {
    use draftcraft_doc::*;
    use draftcraft_geom::{Vec2, Vec3};
    let mut d = Drawing::new_imperial();
    let image = |x: f64, path: &str| Image {
        insert: Vec3::new(x, 0.0, 0.0),
        u: Vec3::new(0.01, 0.0, 0.0),
        v: Vec3::new(0.0, 0.01, 0.0),
        size: Vec2::new(640.0, 480.0),
        path: path.into(),
        ..Image::default()
    };
    let rect = Image { clip: vec![Vec2::new(10.0, 20.0), Vec2::new(300.0, 200.0)], clipping: true, display: 7, ..image(10.0, "photos/site.png") };
    let polygon = Image {
        clip: vec![Vec2::new(0.0, 0.0), Vec2::new(400.0, 50.0), Vec2::new(200.0, 300.0)],
        clipping: true,
        display: 15,
        brightness: 70,
        contrast: 40,
        fade: 30,
        ..image(20.0, "C:\\scans\\plan.tif")
    };
    let wipeout = Wipeout { boundary: vec![Vec2::new(30.0, 0.0), Vec2::new(34.0, 0.0), Vec2::new(34.0, 3.0), Vec2::new(30.0, 3.0)] };
    let mut pdf = Image {
        insert: Vec3::new(40.0, 0.0, 0.0),
        u: Vec3::new(2.0 / 72.0 * 0.6_f64.cos(), 2.0 / 72.0 * 0.6_f64.sin(), 0.0),
        v: Vec3::new(-1.5 / 72.0 * 0.6_f64.sin(), 1.5 / 72.0 * 0.6_f64.cos(), 0.0),
        size: Vec2::ZERO,
        path: "specs/sheet.pdf".into(),
        display: 7,
        contrast: 20,
        fade: 25,
        clipping: true,
        underlay: Some(Underlay {
            kind: UnderlayKind::Pdf,
            page: "2".into(),
            monochrome: true,
            adjust_for_background: false,
            clip: vec![Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0), Vec2::new(5.0, 4.0)],
        }),
        ..Image::default()
    };
    pdf.name = "sheet - 2".into();
    for k in [
        EntityKind::Image(image(0.0, "photos/site.png")),
        EntityKind::Image(rect),
        EntityKind::Image(polygon),
        EntityKind::Wipeout(wipeout),
        EntityKind::Image(pdf),
    ] {
        d.add(&Space::Model, Common::default(), k).expect("add");
    }
    d
}

#[test]
fn images_wipeouts_and_underlays_read_as_the_acadrust_reader_reads_them() {
    use draftcraft_doc::EntityKind;
    use draftcraft_io::{DwgReader, SaveOptions, Version, read_dwg_with, write_with};
    let source = images_drawing();
    for version in [Version::R2000, Version::R2004, Version::R2007, Version::R2010, Version::R2013, Version::R2018] {
        let bytes = write_with(&source, "t.dwg", &SaveOptions { version: Some(version), binary: false }).unwrap();
        let (d, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        let reference = read_dwg_with(&bytes, DwgReader::Acadrust).unwrap();
        let (got, want) = (d.model.iter().cloned().collect::<Vec<_>>(), reference.model.iter().cloned().collect::<Vec<_>>());
        assert_eq!(want.iter().filter(|e| matches!(e.kind, EntityKind::Image(_) | EntityKind::Wipeout(_))).count(), 5, "{version:?}");
        assert_eq!(got.len(), want.len(), "{version:?}: unread {}", unread.summary());
        for (ge, we) in got.iter().zip(&want) {
            let (g, w) = (serde_json::to_value(ge.as_ref()).unwrap(), serde_json::to_value(we.as_ref()).unwrap());
            assert!(close(&g["kind"], &w["kind"], false), "{version:?}\nread      {}\nreference {}", g["kind"], w["kind"]);
            assert!(close(&g["common"], &w["common"], false), "{version:?}: properties\nread      {}\nreference {}", g["common"], w["common"]);
        }
        for t in ["IMAGE", "WIPEOUT", "PDFUNDERLAY", "IMAGEDEF", "PDFDEFINITION"] {
            assert!(!unread.by_type.keys().any(|k| k.starts_with(t)), "{version:?}: {}", unread.summary());
        }
    }
}

/// Multilines: in STANDARD and in a style of three elements with colours, a linetype, a fill,
/// caps and joints; open and closed, each justification, a negative scale, caps suppressed and a
/// break in one element.
fn mlines_drawing() -> draftcraft_doc::Drawing {
    use draftcraft_doc::*;
    use draftcraft_geom::Vec3;
    let mut d = Drawing::new_imperial();
    if !d.linetypes.iter().any(|l| l.name.eq_ignore_ascii_case("DASHED"))
        && let Some(lt) = library::standard_linetypes().into_iter().find(|l| l.name.eq_ignore_ascii_case("DASHED"))
    {
        d.linetypes.push(lt);
    }
    let wall = MLineStyle {
        name: "WALL3".into(),
        description: "Three lines".into(),
        elements: vec![
            MLineElement { offset: 1.0, color: draftcraft_color::Color::Index(1), linetype: "ByLayer".into() },
            MLineElement { offset: 0.0, color: draftcraft_color::Color::Index(3), linetype: "DASHED".into() },
            MLineElement { offset: -1.0, color: draftcraft_color::Color::ByBlock, linetype: "ByBlock".into() },
        ],
        start_caps: MLineCaps { line: true, outer_arc: false, inner_arcs: false, angle: std::f64::consts::FRAC_PI_2 },
        end_caps: MLineCaps { line: false, outer_arc: true, inner_arcs: true, angle: 1.2 },
        joints: true,
        fill: Some(draftcraft_color::Color::Index(8)),
    };
    d.mline_styles.push(wall.clone());
    let v = |x: f64, y: f64| MLineVertex::new(Vec3::new(x, y, 0.0));
    let ml = |style: &MLineStyle, vertices: Vec<MLineVertex>| MLine {
        style: style.name.clone(),
        def: style.clone(),
        scale: 1.0,
        justification: MLineJust::Top,
        closed: false,
        no_start_caps: false,
        no_end_caps: false,
        vertices,
    };
    let standard = MLineStyle::default();
    let mut cut = vec![v(0.0, 20.0), v(10.0, 20.0), v(10.0, 30.0)];
    cut[0].cuts.push(MLineCut { element: 0, start: 2.0, end: 4.0 });
    let lines = [
        ml(&standard, vec![v(0.0, 0.0), v(10.0, 0.0), v(10.0, 5.0)]),
        MLine { closed: true, justification: MLineJust::Zero, ..ml(&wall, vec![v(20.0, 0.0), v(30.0, 0.0), v(30.0, 8.0), v(20.0, 8.0)]) },
        MLine {
            scale: -2.0,
            justification: MLineJust::Bottom,
            no_start_caps: true,
            no_end_caps: true,
            ..ml(&wall, vec![v(40.0, 0.0), v(55.0, 5.0)])
        },
        ml(&standard, cut),
    ];
    for m in lines {
        d.add(&Space::Model, Common::default(), EntityKind::MLine(m)).expect("multiline");
    }
    d
}

#[test]
fn multilines_read_as_the_acadrust_reader_reads_them() {
    use draftcraft_doc::EntityKind;
    use draftcraft_io::{DwgReader, SaveOptions, Version, read_dwg_with, write_with};
    let source = mlines_drawing();
    for version in [Version::R2000, Version::R2004, Version::R2007, Version::R2010, Version::R2013, Version::R2018] {
        let bytes = write_with(&source, "t.dwg", &SaveOptions { version: Some(version), binary: false }).unwrap();
        let (d, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        let reference = read_dwg_with(&bytes, DwgReader::Acadrust).unwrap();
        let mlines = |d: &draftcraft_doc::Drawing| d.model.iter().filter(|e| matches!(e.kind, EntityKind::MLine(_))).cloned().collect::<Vec<_>>();
        let (got, want) = (mlines(&d), mlines(&reference));
        assert_eq!(want.len(), 4, "{version:?}");
        assert_eq!(got.len(), want.len(), "{version:?}: unread {}", unread.summary());
        for (ge, we) in got.iter().zip(&want) {
            let (g, w) = (serde_json::to_value(ge.as_ref()).unwrap(), serde_json::to_value(we.as_ref()).unwrap());
            assert!(close(&g["kind"], &w["kind"], false), "{version:?}\nread      {}\nreference {}", g["kind"], w["kind"]);
        }
        let style = |d: &draftcraft_doc::Drawing| d.mline_styles.iter().find(|s| s.name == "WALL3").map(|s| serde_json::to_value(s).unwrap());
        assert!(style(&d).is_some(), "{version:?}");
        assert!(
            close(&serde_json::json!(style(&d)), &serde_json::json!(style(&reference)), false),
            "{version:?}\nread      {:?}\nreference {:?}",
            style(&d),
            style(&reference)
        );
        assert!(!unread.by_type.keys().any(|k| k.starts_with("MLINE")), "{version:?}: {}", unread.summary());
    }
}

/// Tables: one with a title merged across, a header row, a merged block of cells, a cell with
/// its own text height and a formula, and a plain grid of numbers.
fn tables_drawing() -> draftcraft_doc::Drawing {
    use draftcraft_doc::*;
    use draftcraft_geom::Vec3;
    let mut d = Drawing::new_imperial();
    let cell = |s: &str| TableCell { text: s.into(), ..TableCell::default() };
    let mut cells = vec![
        vec![TableCell { merged: Some((1, 3)), ..cell("DOOR SCHEDULE") }, cell(""), cell("")],
        vec![cell("MARK"), cell("SIZE"), cell("QTY")],
        vec![TableCell { merged: Some((2, 1)), ..cell("D1") }, cell("3'-0\" x 7'-0\""), cell("4")],
        vec![cell(""), TableCell { text_height: Some(0.25), ..cell("2'-8\" x 6'-8\"") }, cell("2")],
        vec![cell("TOTAL"), cell(""), TableCell { formula: Some("=Sum(C3:C4)".into()), ..cell("6") }],
    ];
    cells[1][0].style = "Header".into();
    let schedule = Table {
        insert: Vec3::new(0.0, 20.0, 0.0),
        col_widths: vec![1.5, 3.0, 1.0],
        row_heights: vec![0.5, 0.4, 0.3, 0.3, 0.3],
        cells,
        title: true,
        header: true,
        text_height: 0.18,
        ..Table::default()
    };
    let grid = Table {
        insert: Vec3::new(10.0, 20.0, 0.0),
        col_widths: vec![1.0, 1.0],
        row_heights: vec![0.3, 0.3],
        cells: vec![vec![cell("1"), cell("2")], vec![cell("3"), cell("4")]],
        ..Table::default()
    };
    for t in [schedule, grid] {
        d.add(&Space::Model, Common::default(), EntityKind::Table(t)).expect("table");
    }
    d
}

#[test]
fn tables_read_as_the_acadrust_reader_reads_them() {
    use draftcraft_doc::EntityKind;
    use draftcraft_io::{DwgReader, SaveOptions, Version, read_dwg_with, write_with};
    let source = tables_drawing();
    for version in [Version::R2000, Version::R2004, Version::R2007, Version::R2010, Version::R2013, Version::R2018] {
        let bytes = write_with(&source, "t.dwg", &SaveOptions { version: Some(version), binary: false }).unwrap();
        let (d, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        let reference = read_dwg_with(&bytes, DwgReader::Acadrust).unwrap();
        let (got, want) = (d.model.iter().cloned().collect::<Vec<_>>(), reference.model.iter().cloned().collect::<Vec<_>>());
        assert_eq!(want.iter().filter(|e| matches!(e.kind, EntityKind::Table(_))).count(), 2, "{version:?}");
        assert_eq!(got.len(), want.len(), "{version:?}: unread {}", unread.summary());
        for (ge, we) in got.iter().zip(&want) {
            let (g, w) = (serde_json::to_value(ge.as_ref()).unwrap(), serde_json::to_value(we.as_ref()).unwrap());
            assert!(close(&g["kind"], &w["kind"], false), "{version:?}\nread      {}\nreference {}", g["kind"], w["kind"]);
            assert!(close(&g["common"], &w["common"], false), "{version:?}: properties\nread      {}\nreference {}", g["common"], w["common"]);
        }
        let blocks = |d: &draftcraft_doc::Drawing| d.blocks.keys().map(|k| k.to_ascii_uppercase()).collect::<Vec<_>>();
        assert_eq!(blocks(&d), blocks(&reference), "{version:?}: the tables' blocks are left out");
        assert!(!unread.by_type.keys().any(|k| k.starts_with("ACAD_TABLE") || k.starts_with("TABLE ")), "{version:?}: {}", unread.summary());
    }
}

/// Regions: one loop with an arc, one with an island and a second outer loop, and one whose
/// other loops are too large for the extended data (written as plain polylines).
fn regions_drawing() -> draftcraft_doc::Drawing {
    use draftcraft_doc::*;
    use draftcraft_geom::{PolyVertex, Vec2};
    let v = |x: f64, y: f64| PolyVertex::new(Vec2::new(x, y));
    let square = |x: f64, y: f64, s: f64| vec![v(x, y), v(x + s, y), v(x + s, y + s), v(x, y + s)];
    let lp = |vertices: Vec<PolyVertex>, outer: bool| HatchLoop { vertices, outer, sources: Vec::new() };
    let mut d = Drawing::new_imperial();
    let mut arc = square(0.0, 0.0, 4.0);
    arc[1].bulge = 0.5;
    let many: Vec<PolyVertex> =
        (0..400).map(|i| PolyVertex::new(Vec2::polar(Vec2::new(40.0, 5.0), 3.0, f64::from(i) * std::f64::consts::TAU / 400.0))).collect();
    let regions = [
        Region { loops: vec![lp(arc, true)] },
        Region { loops: vec![lp(square(10.0, 0.0, 10.0), true), lp(square(13.0, 3.0, 2.0), false), lp(square(25.0, 0.0, 3.0), true)] },
        Region { loops: vec![lp(square(35.0, 0.0, 10.0), true), lp(many, false)] },
    ];
    for r in regions {
        d.add(&Space::Model, Common::default(), EntityKind::Region(r)).expect("region");
    }
    d
}

#[test]
fn regions_read_as_the_acadrust_reader_reads_them() {
    use draftcraft_doc::EntityKind;
    use draftcraft_io::{DwgReader, SaveOptions, Version, read_dwg_with, write_with};
    let source = regions_drawing();
    for version in [Version::R2000, Version::R2004, Version::R2007, Version::R2010, Version::R2013, Version::R2018] {
        let bytes = write_with(&source, "t.dwg", &SaveOptions { version: Some(version), binary: false }).unwrap();
        let (d, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        let reference = read_dwg_with(&bytes, DwgReader::Acadrust).unwrap();
        let (got, want) = (d.model.iter().cloned().collect::<Vec<_>>(), reference.model.iter().cloned().collect::<Vec<_>>());
        assert_eq!(want.iter().filter(|e| matches!(e.kind, EntityKind::Region(_))).count(), 3, "{version:?}");
        assert_eq!(got.len(), want.len(), "{version:?}: unread {}", unread.summary());
        for (ge, we) in got.iter().zip(&want) {
            let (g, w) = (serde_json::to_value(ge.as_ref()).unwrap(), serde_json::to_value(we.as_ref()).unwrap());
            assert!(close(&g["kind"], &w["kind"], false), "{version:?}\nread      {}\nreference {}", g["kind"], w["kind"]);
            assert!(close(&g["common"], &w["common"], false), "{version:?}: properties\nread      {}\nreference {}", g["common"], w["common"]);
        }
        assert!(!unread.by_type.keys().any(|k| k.starts_with("LWPOLYLINE") || k.starts_with("DraftCraft")), "{version:?}: {}", unread.summary());
    }
}

/// DIMASSOC objects as DraftCraft's save path writes them (through acadrust) name subentities
/// with values AutoCAD's own files have as zero: each is read, in every version.
#[test]
fn saved_dimension_associations_are_read() {
    use draftcraft_io::{SaveOptions, Version, write_with};
    let source = draftcraft_io::read(include_bytes!("../../../../examples/apartment.dxf"), "apartment.dxf").unwrap();
    for version in [Version::R2000, Version::R2004, Version::R2007, Version::R2010, Version::R2013, Version::R2018] {
        let bytes = write_with(&source, "t.dwg", &SaveOptions { version: Some(version), binary: false }).unwrap();
        let file = DwgFile::open(&bytes).unwrap();
        assert!(file.objects().any(|o| o.type_name == "DIMASSOC"), "{version:?}: the drawing has associative dimensions");
        let (_, unread) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        assert!(!unread.by_type.keys().any(|k| k.starts_with("DIMASSOC")), "{version:?}: {}", unread.summary());
    }
}

/// Writes every step's test drawing as DWG in every version to `DWG_FIXTURES_OUT`, for checking
/// them with other programs (BlackCAD, AutoCAD, the web build): `DWG_FIXTURES_OUT=dir cargo test
/// -p draftcraft-io --test it write_dwg_fixtures -- --ignored`.
#[test]
#[ignore = "writes files for other programs to read"]
fn write_dwg_fixtures() {
    use draftcraft_io::{SaveOptions, Version, write_with};
    let Ok(dir) = std::env::var("DWG_FIXTURES_OUT") else { return };
    let drawings = [
        ("attributes", attributes_drawing()),
        ("dimensions", dimensions_drawing()),
        ("hatches", hatches_drawing()),
        ("annotation", annotation_drawing()),
        ("viewports", viewports_drawing()),
        ("images", images_drawing()),
        ("mlines", mlines_drawing()),
        ("tables", tables_drawing()),
        ("regions", regions_drawing()),
    ];
    for (name, d) in &drawings {
        for version in [Version::R2000, Version::R2004, Version::R2007, Version::R2010, Version::R2013, Version::R2018] {
            let bytes = write_with(d, "t.dwg", &SaveOptions { version: Some(version), binary: false }).unwrap();
            std::fs::write(std::path::Path::new(&dir).join(format!("{name}-{version:?}.dwg")), bytes).unwrap();
        }
    }
    let acad = [
        ("curves", curves_dwg as fn(acadrust::types::DxfVersion) -> Vec<u8>),
        ("hatch-edges", hatch_edges_dwg),
        ("mirrored-text", mirrored_text_dwg),
    ];
    for (name, make) in acad {
        for v in [
            acadrust::types::DxfVersion::AC1015,
            acadrust::types::DxfVersion::AC1018,
            acadrust::types::DxfVersion::AC1024,
            acadrust::types::DxfVersion::AC1027,
            acadrust::types::DxfVersion::AC1032,
        ] {
            std::fs::write(std::path::Path::new(&dir).join(format!("{name}-{v:?}.dwg")), make(v)).unwrap();
        }
    }
}

/// AutoCAD's own sample drawings, when `DRAFTCRAFT_DWG_SAMPLES` names their folder (ACadSharp's
/// `samples`, MIT: `sample_AC1015.dwg` to `sample_AC1032.dwg`, saved by AutoCAD). Colour book
/// colours read as AutoCAD has them in every version: from the DBCOLOR object the entity names
/// (2004 on: its colour stores no true colour, which used to lose the entity) and from AutoCAD's
/// round-trip data (2000), which no entity keeps as extended data.
#[test]
fn autocad_samples_colour_book_colours() {
    use draftcraft_color::{Color, Rgb};
    use draftcraft_doc::Handle;
    let Some(dir) = std::env::var_os("DRAFTCRAFT_DWG_SAMPLES").map(std::path::PathBuf::from) else {
        eprintln!("DRAFTCRAFT_DWG_SAMPLES is not set: AutoCAD's sample drawings skipped");
        return;
    };
    let ral = Some(("14848000".to_string(), "RAL CLASSIC$RAL 1006".to_string()));
    for code in ["AC1015", "AC1018", "AC1021", "AC1024", "AC1027", "AC1032"] {
        let bytes = std::fs::read(dir.join(format!("sample_{code}.dwg"))).unwrap_or_else(|e| panic!("{code}: {e}"));
        let (d, _) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{code}: {e}"));
        let circle = d.entity(Handle(0x99F)).unwrap_or_else(|| panic!("{code}: CIRCLE 99F is read"));
        assert_eq!(circle.common.color, Color::True(Rgb::from_u32(0xE2_9000)), "{code}: AutoCAD's 420 of CIRCLE 99F");
        assert_eq!(d.foreign.entities.get(&Handle(0x99F)).and_then(|x| x.color_book.clone()), ral, "{code}: its 430");
        // A true colour of no book, from 2000's round-trip data as from 2004's colour.
        let line = d.entity(Handle(0x99E)).unwrap_or_else(|| panic!("{code}: LINE 99E"));
        assert_eq!(line.common.color, Color::True(Rgb::from_u32(0x34_C918)), "{code}: AutoCAD's 420 of LINE 99E");
        for e in d.model.iter() {
            let rt = e.common.xdata.iter().any(|t| t.code == 1000 && (t.value == "RTTcAl" || t.value == "RTMaterial"));
            assert!(!rt, "{code}: {} {:X} keeps AutoCAD's round-trip data", e.kind.dxf_name(), e.handle.0);
        }
        if code != "AC1015" {
            let layer = d.foreign.records.get(&("LAYER".to_string(), "LAYER_COLOR_BOOK".to_string())).and_then(|x| x.color_book.clone());
            assert_eq!(layer, Some(("16564615".to_string(), "DIC COLOR GUIDE(R)$DIC 7".to_string())), "{code}: layer's 420 and 430");
        }
    }
}

/// AutoCAD's own sample drawings (as [`autocad_samples_colour_book_colours`]): entities read as
/// AutoCAD's DXF of the same drawing has them, in every version. The leader gets its hook line,
/// the tolerance frame its `^J` line breaks, and the spline that AutoCAD 2013 and later store as
/// fit points and end derivatives AutoCAD's control points.
#[test]
fn autocad_samples_entities_read_as_autocads_dxf() {
    use draftcraft_doc::{EntityKind, Handle};
    let Some(dir) = std::env::var_os("DRAFTCRAFT_DWG_SAMPLES").map(std::path::PathBuf::from) else {
        eprintln!("DRAFTCRAFT_DWG_SAMPLES is not set: AutoCAD's sample drawings skipped");
        return;
    };
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    for code in ["AC1015", "AC1018", "AC1021", "AC1024", "AC1027", "AC1032"] {
        let bytes = std::fs::read(dir.join(format!("sample_{code}.dwg"))).unwrap_or_else(|e| panic!("{code}: {e}"));
        let (d, _) = dwg::read(&bytes).unwrap_or_else(|e| panic!("{code}: {e}"));
        let kind = |h: u64| d.entity(Handle(h)).map(|e| e.kind.clone()).unwrap_or_else(|| panic!("{code}: {h:X} is read"));
        let EntityKind::Leader(l) = kind(0x512) else { panic!("{code}: LEADER 512") };
        let xs: Vec<f64> = l.vertices.iter().map(|v| v.x).collect();
        assert_eq!(xs.len(), 4, "{code}: AutoCAD's DXF lists four vertices: {xs:?}");
        assert!(near(xs[2], 737.898_750_025_284_9) && near(xs[3], 740.398_750_025_284_9), "{code}: {xs:?}");
        // The DWG stores the clip polygon open and AutoCAD's DXF closed: the drawing holds either,
        // the same four vertices (`Wipeout::vertices`).
        let EntityKind::Wipeout(w) = kind(0x44D) else { panic!("{code}: WIPEOUT 44D") };
        assert_eq!(w.vertices().len(), 4, "{code}: AutoCAD's four clip vertices");
        let EntityKind::Tolerance(t) = kind(0x823) else { panic!("{code}: TOLERANCE 823") };
        assert_eq!(t.text, "99^Ja", "{code}");
        let EntityKind::Spline(s) = kind(0x434) else { panic!("{code}: SPLINE 434") };
        let want = [
            (250.458_790_783_224_1, 17.426_024_184_844_77),
            (254.713_771_109_526_1, 28.254_981_364_991_63),
            (259.387_964_086_943_8, 17.503_121_515_419_48),
            (259.224_551_110_004_1, 27.287_504_552_472_21),
        ];
        assert_eq!(s.control.len(), 4, "{code}");
        for (c, (x, y)) in s.control.iter().zip(want) {
            assert!(near(c.x, x) && near(c.y, y), "{code}: control point {c:?}, AutoCAD's ({x}, {y})");
        }
        // Multileader styles (MLEADERSTYLE objects, written from 2004 on): AutoCAD's Standard and
        // Annotative, with the values of its DXF.
        if code != "AC1015" {
            for (name, annotative) in [("Standard", false), ("Annotative", true)] {
                let st = d.mleader_styles.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("{code}: multileader style {name}"));
                assert!(
                    near(st.arrow_size, 0.18) && near(st.text_height, 0.18) && near(st.landing_gap, 0.09) && near(st.dogleg, 0.36),
                    "{code}: {st:?}"
                );
                assert_eq!((st.text_style.as_str(), st.annotative), ("Standard", annotative), "{code}: {name}");
            }
        }
        // No entity keeps AutoCAD's round-trip spline data as extended data.
        assert!(d.model.iter().all(|e| !e.common.xdata.iter().any(|t| t.code == 1001 && t.value == "ACAD_SPLINE_DATA")), "{code}");
        if code != "AC1032" {
            continue;
        }
        // 2018: MTEXT columns (an embedded object in AutoCAD's DXF), whose height AutoCAD stores
        // negative and its DXF has as 0.
        let EntityKind::MText(m) = kind(0x3EC) else { panic!("{code}: MTEXT 3EC") };
        let c = m.columns.unwrap_or_else(|| panic!("{code}: MTEXT 3EC has its columns"));
        assert_eq!((c.kind, c.count, c.auto_height, c.flow_reversed), (draftcraft_doc::MTextColumnType::Dynamic, 1, false, false), "{code}");
        assert!(near(c.gutter, 110.223_167_038_119_5) && c.heights == vec![0.0], "{code}: {c:?}");
        assert!(near(m.width, 15.741_414_244_987_48), "{code}");
        // 2018: multiline attributes, their text an MTEXT embedded in the record.
        let block = d.blocks.get("my_block_v2").unwrap_or_else(|| panic!("{code}: block my_block_v2"));
        let def = block.entities.iter().find_map(|e| match &e.kind {
            EntityKind::AttDef(a) if a.tag == "MULTI_LINE_ATT" => Some(a.clone()),
            _ => None,
        });
        let def = def.unwrap_or_else(|| panic!("{code}: the multiline ATTDEF 796 is read"));
        assert!(def.multiline.is_some(), "{code}");
        assert_eq!(def.prompt, "this is an example of prompt", "{code}");
        assert!(def.modes.lock_position, "{code}");
        let EntityKind::Insert(i) = kind(0x79C) else { panic!("{code}: INSERT 79C is read") };
        let att = i.attribs.iter().find(|a| a.tag == "MULTI_LINE_ATT").unwrap_or_else(|| panic!("{code}: its multiline ATTRIB"));
        assert_eq!(att.text.value, "my multi line text for the attrrib", "{code}");
        assert_eq!(att.multiline.as_ref().map(|m| m.mtext.contents.as_str()), Some("my multi line text for the attrrib"), "{code}");
        assert!(near(att.text.height, 1.720_654_456_209_621), "{code}");
    }
}
