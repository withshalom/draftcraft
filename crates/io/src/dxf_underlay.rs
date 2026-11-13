//! PDF underlays in DXF (DXF Reference: UNDERLAY entity, UNDERLAYDEFINITION object). A
//! PDFUNDERLAY entity holds the placement, clip boundary and display settings; it points (group
//! 340) to a PDFDEFINITION object holding the file and the page, which the `ACAD_PDFDEFINITIONS`
//! dictionary names.
//!
//! DraftCraft keeps an underlay as an [`Image`] whose [`Image::underlay`] is set: one "pixel" is
//! one PDF point (1/72 inch), so the reference's scale (drawing units per inch, groups 41/42) is
//! 72 times a point's length. The page size isn't in the file; it is filled in when the PDF is
//! read (`draftcraft_engine::cmd::underlay`). The clip boundary (groups 11/21) is in the
//! underlay's own units (inches from the insertion point, before scale and rotation) and is
//! written back as read.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use draftcraft_doc::{Drawing, EntityKind, EntityStore, Handle, Image, POINTS_PER_INCH, Underlay, UnderlayKind};
use draftcraft_dxf::{Section, Tag};
use draftcraft_geom::{Vec2, Vec3};

/// Most clip boundary vertices kept per underlay (hostile files).
const MAX_CLIP: usize = 100_000;
/// The named object dictionary key of the PDF definitions.
pub(crate) const DICT_KEY: &str = "ACAD_PDFDEFINITIONS";

/// Reference flags (group 280).
const FLAG_CLIP: i64 = 1;
const FLAG_ON: i64 = 2;
const FLAG_MONO: i64 = 4;
const FLAG_ADJUST: i64 = 8;

fn key(s: &str) -> String {
    s.trim().to_ascii_uppercase()
}

/// Whether `kind` (a record's DXF type) is an underlay reference DraftCraft models.
pub(crate) fn is_reference(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("PDFUNDERLAY")
}

/// The PDFUNDERLAY entity's own groups (after the `AcDbUnderlayReference` marker when present).
/// The file and page come from its definition ([`resolve`]); the page size from the PDF.
pub(crate) fn read_entity(tags: &[Tag]) -> Image {
    let start = tags.iter().position(|t| t.code == 100 && t.str() == "AcDbUnderlayReference").unwrap_or(0);
    let tags = tags.get(start..).unwrap_or(&[]);
    let f = |c: i32| tags.iter().find(|t| t.code == c).map(Tag::f64).filter(|v| v.is_finite());
    let i = |c: i32| tags.iter().find(|t| t.code == c).map(Tag::i64);
    let pct = |c: i32, d: u8| i(c).and_then(|v| u8::try_from(v.clamp(0, 100)).ok()).unwrap_or(d);
    let insert = Vec3::new(f(10).unwrap_or(0.0), f(20).unwrap_or(0.0), f(30).unwrap_or(0.0));
    let flags = i(280).unwrap_or(FLAG_ON);
    let xs = tags.iter().filter(|t| t.code == 11).map(Tag::f64);
    let ys = tags.iter().filter(|t| t.code == 21).map(Tag::f64);
    let clip: Vec<Vec2> = xs.zip(ys).take(MAX_CLIP).map(|(x, y)| Vec2::new(x, y)).collect();
    let rotation = f(50).unwrap_or(0.0).to_radians();
    reference(insert, (f(41), f(42)), rotation, flags, (pct(281, Underlay::DEFAULT_CONTRAST), pct(282, 0)), clip)
}

/// An underlay reference as the drawing holds it: one PDF point as the `u`/`v` vectors from
/// the scales (a zero or missing one is 1) and the rotation (radians), the display and clipping
/// from the flags. Both file formats hold these values.
pub(crate) fn reference(
    insert: Vec3,
    scale: (Option<f64>, Option<f64>),
    rotation: f64,
    flags: i64,
    (contrast, fade): (u8, u8),
    clip: Vec<Vec2>,
) -> Image {
    let nonzero = |v: Option<f64>| v.filter(|s| s.is_finite() && *s != 0.0).unwrap_or(1.0);
    let (sx, sy) = (nonzero(scale.0), nonzero(scale.1));
    let dir = Vec2::from_angle(if rotation.is_finite() { rotation } else { 0.0 });
    let clip = clip.into_iter().take(MAX_CLIP).filter(|q| q.x.is_finite() && q.y.is_finite()).collect();
    Image {
        insert,
        u: (dir * (sx / POINTS_PER_INCH)).to3(0.0),
        v: (dir.perp() * (sy / POINTS_PER_INCH)).to3(0.0),
        // Unknown until the PDF is read.
        size: Vec2::ZERO,
        clipping: flags & FLAG_CLIP != 0,
        display: if flags & FLAG_ON != 0 { 7 } else { 6 },
        contrast,
        fade,
        underlay: Some(Underlay {
            kind: UnderlayKind::Pdf,
            page: String::new(),
            monochrome: flags & FLAG_MONO != 0,
            adjust_for_background: flags & FLAG_ADJUST != 0,
            clip,
        }),
        ..Image::default()
    }
}

/// Call `f` with each record (`0 TYPE` and the groups after it) of a section, without copying.
fn each_record<'a>(tags: &'a [Tag], mut f: impl FnMut(&str, &'a [Tag])) {
    let mut open: Option<(String, usize)> = None;
    for (i, t) in tags.iter().enumerate() {
        if t.code == 0 {
            if let Some((k, s)) = open.take() {
                f(&k, tags.get(s..i).unwrap_or(&[]));
            }
            open = Some((t.str(), i + 1));
        }
    }
    if let Some((k, s)) = open {
        f(&k, tags.get(s..).unwrap_or(&[]));
    }
}

/// A PDFDEFINITION object's data.
#[derive(Default)]
struct Def {
    name: String,
    path: String,
    page: String,
}

fn is_underlay(e: &draftcraft_doc::Entity) -> bool {
    matches!(&e.kind, EntityKind::Image(i) if i.underlay.is_some())
}

fn has_underlays(store: &EntityStore) -> bool {
    store.iter().any(|e| is_underlay(e))
}

/// Fill in each underlay's file, page and name from the definition it points to.
pub(crate) fn resolve(secs: &[Section], d: &mut Drawing) {
    if !has_underlays(&d.model) && !d.layouts.iter().any(|l| has_underlays(&l.entities)) && !d.blocks.values().any(|b| has_underlays(&b.entities)) {
        return;
    }
    // Reference handle → definition handle.
    let mut refs: HashMap<Handle, String> = HashMap::new();
    for s in secs.iter().filter(|s| s.name == "ENTITIES" || s.name == "BLOCKS") {
        each_record(&s.tags, |kind, tags| {
            if !is_reference(kind) {
                return;
            }
            let h = tags.iter().find(|t| t.code == 5).and_then(|t| Handle::parse_hex(&t.str()));
            let def = tags.iter().find(|t| t.code == 340).map(|t| key(&t.str()));
            if let (Some(h), Some(def)) = (h, def) {
                refs.insert(h, def);
            }
        });
    }
    let mut defs: HashMap<String, Def> = HashMap::new();
    let mut names: HashMap<String, String> = HashMap::new();
    for s in secs.iter().filter(|s| s.name == "OBJECTS") {
        each_record(&s.tags, |kind, tags| match kind {
            "PDFDEFINITION" => {
                let Some(h) = tags.iter().find(|t| t.code == 5).map(|t| key(&t.str())) else { return };
                let start = tags.iter().position(|t| t.code == 100 && t.str() == "AcDbUnderlayDefinition").unwrap_or(0);
                let body = tags.get(start..).unwrap_or(&[]);
                let s = |c: i32| body.iter().find(|t| t.code == c).map(Tag::str).unwrap_or_default();
                defs.insert(h, Def { path: s(1), page: s(2), ..Def::default() });
            }
            "DICTIONARY" => {
                let mut name: Option<String> = None;
                for t in tags {
                    match t.code {
                        3 => name = Some(t.str()),
                        350 | 360 => {
                            if let Some(n) = name.take() {
                                names.insert(key(&t.str()), n);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        });
    }
    for (h, def) in &mut defs {
        if let Some(n) = names.get(h) {
            def.name = n.clone();
        }
    }
    let by_ref: HashMap<Handle, &Def> = refs.iter().filter_map(|(r, dh)| Some((*r, defs.get(dh)?))).collect();
    if by_ref.is_empty() {
        return;
    }
    // A shared budget on the strings copied into references (see `dxf_image::resolve`).
    let mut budget: usize = 64 << 20;
    fill(&mut d.model, &by_ref, &mut budget);
    for l in &mut d.layouts {
        fill(&mut l.entities, &by_ref, &mut budget);
    }
    for b in d.blocks.values_mut() {
        if b.entities.iter().any(|e| is_underlay(e) && by_ref.contains_key(&e.handle)) {
            fill(&mut Arc::make_mut(b).entities, &by_ref, &mut budget);
        }
    }
}

fn fill(store: &mut EntityStore, by_ref: &HashMap<Handle, &Def>, budget: &mut usize) {
    let hs: Vec<Handle> = store.iter().filter(|e| is_underlay(e) && by_ref.contains_key(&e.handle)).map(|e| e.handle).collect();
    for h in hs {
        let Some(def) = by_ref.get(&h) else { continue };
        let cost = def.path.len() + def.name.len() + def.page.len();
        let Some(left) = budget.checked_sub(cost) else { return };
        *budget = left;
        store.modify(h, |e| {
            if let EntityKind::Image(i) = &mut e.kind
                && let Some(u) = &mut i.underlay
            {
                i.path = def.path.clone();
                i.name = def.name.clone();
                u.page = def.page.clone();
            }
        });
    }
}

/// One PDFDEFINITION to write, shared by the references to the same file, page and name.
struct PlannedDef {
    handle: String,
    name: String,
    path: String,
    page: String,
}

/// The objects behind a drawing's underlays, with handles allocated before anything is written.
#[derive(Default)]
pub(crate) struct Plan {
    /// Reference entity → its definition's handle.
    pub by_entity: HashMap<Handle, String>,
    defs: Vec<PlannedDef>,
    /// The `ACAD_PDFDEFINITIONS` dictionary.
    pub dict: String,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// The definition's name: the underlay's own, else the file name without extension, with the
/// page after it when it isn't the first ("plan - 2").
fn def_name(i: &Image, page: &str) -> String {
    let n = i.name.trim();
    if !n.is_empty() {
        return n.to_string();
    }
    let file = i.path.rsplit(['/', '\\']).next().unwrap_or("");
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file).trim();
    let stem = if stem.is_empty() { "PDF" } else { stem };
    match page.trim() {
        "" | "1" => stem.to_string(),
        p => format!("{stem} - {p}"),
    }
}

/// Allocate the definition and dictionary handles for every underlay.
pub(crate) fn plan<'a>(entities: impl Iterator<Item = &'a draftcraft_doc::Entity>, mut next: impl FnMut() -> String) -> Plan {
    let mut p = Plan::default();
    let mut index: HashMap<(String, String, String), usize> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    for e in entities {
        let EntityKind::Image(i) = &e.kind else { continue };
        let Some(u) = &i.underlay else { continue };
        if p.by_entity.contains_key(&e.handle) {
            continue;
        }
        let k = (i.path.clone(), u.page.clone(), i.name.clone());
        let at = match index.get(&k) {
            Some(&at) => at,
            None => {
                let base = def_name(i, &u.page);
                let mut name = base.clone();
                let mut n = 2usize;
                while !used.insert(key(&name)) {
                    name = format!("{base}_{n}");
                    n = n.saturating_add(1);
                }
                p.defs.push(PlannedDef { handle: next(), name, path: i.path.clone(), page: u.page.clone() });
                let at = p.defs.len().saturating_sub(1);
                index.insert(k, at);
                at
            }
        };
        if let Some(def) = p.defs.get(at) {
            p.by_entity.insert(e.handle, def.handle.clone());
        }
    }
    if !p.defs.is_empty() {
        p.dict = next();
    }
    p
}

/// The reference's scale (drawing units per inch along its x and y) and rotation (degrees).
pub fn scale_and_rotation(i: &Image) -> (f64, f64, f64) {
    let u = i.u.xy();
    let v = i.v.xy();
    let sx = u.len() * POINTS_PER_INCH;
    // A mirrored underlay has its y axis clockwise from its x axis.
    let mirrored = u.x * v.y - u.y * v.x < 0.0;
    let sy = v.len() * POINTS_PER_INCH * if mirrored { -1.0 } else { 1.0 };
    let rot = u.y.atan2(u.x).to_degrees();
    let rot = if rot < 0.0 { rot + 360.0 } else { rot };
    (sx, sy, rot)
}

/// The PDFUNDERLAY entity's groups after its common entity groups.
pub(crate) fn entity_tags(i: &Image, def: &str) -> Vec<Tag> {
    let Some(u) = &i.underlay else { return Vec::new() };
    let (sx, sy, rot) = scale_and_rotation(i);
    let finite = |v: f64, d: f64| if v.is_finite() && v != 0.0 { v } else { d };
    let mut t = vec![Tag::s(340, def), Tag::f(10, i.insert.x), Tag::f(20, i.insert.y), Tag::f(30, i.insert.z)];
    t.extend([Tag::f(41, finite(sx, 1.0)), Tag::f(42, finite(sy, 1.0)), Tag::f(43, 1.0), Tag::f(50, finite(rot, 0.0))]);
    t.extend([Tag::f(210, 0.0), Tag::f(220, 0.0), Tag::f(230, 1.0)]);
    let mut flags = 0;
    if i.clipping {
        flags |= FLAG_CLIP;
    }
    if i.display & 1 != 0 {
        flags |= FLAG_ON;
    }
    if u.monochrome {
        flags |= FLAG_MONO;
    }
    if u.adjust_for_background {
        flags |= FLAG_ADJUST;
    }
    t.extend([Tag::i(280, flags), Tag::i(281, i64::from(i.contrast.min(100))), Tag::i(282, i64::from(i.fade.min(80)))]);
    for q in &u.clip {
        t.extend([Tag::f(11, q.x), Tag::f(21, q.y)]);
    }
    t
}

/// The definitions dictionary and the PDFDEFINITION objects; `root` is the named object
/// dictionary, which lists `ACAD_PDFDEFINITIONS`.
pub(crate) fn objects(p: &Plan, root: &str) -> Vec<Tag> {
    let mut t = Vec::new();
    if p.is_empty() {
        return t;
    }
    t.extend([Tag::s(0, "DICTIONARY"), Tag::s(5, &p.dict), Tag::s(330, root), Tag::s(100, "AcDbDictionary"), Tag::i(281, 1)]);
    for d in &p.defs {
        t.extend([Tag::s(3, &d.name), Tag::s(350, &d.handle)]);
    }
    for d in &p.defs {
        t.extend([Tag::s(0, "PDFDEFINITION"), Tag::s(5, &d.handle), Tag::s(102, "{ACAD_REACTORS"), Tag::s(330, &p.dict), Tag::s(102, "}")]);
        t.extend([Tag::s(330, &p.dict), Tag::s(100, "AcDbUnderlayDefinition"), Tag::s(1, &d.path)]);
        t.push(Tag::s(2, if d.page.trim().is_empty() { "1" } else { d.page.trim() }));
    }
    t
}
