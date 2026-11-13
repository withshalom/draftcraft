//! The file's other objects: dictionaries, extension records, scales, plot style names, style
//! objects. What the drawing models of them it takes by the DXF reader's rules (DraftCraft's own
//! records, the scale list, the layers' plot styles, table styles); what it does not model it
//! keeps as the DXF reader keeps a DXF file's (`dxf_raw`): the objects a named-object dictionary
//! entry DraftCraft doesn't write itself leads to, as their DXF groups in
//! [`draftcraft_doc::Foreign`], written back on save.

use std::collections::{HashMap, HashSet};

use draftcraft_doc::foreign::{Foreign, ForeignExtra, ForeignKey, ForeignObject};
use draftcraft_doc::{Drawing, EntityKind, Group, Handle, RawTag};
use draftcraft_dwg::file::{CommonRefs, DwgFile, Groups, NonGraphic, ObjectReader, ObjectRef, XrValue};
use draftcraft_dwg::records::Result as DwgResult;

use super::Unread;
use super::blocks::Blocks;
use super::entities::Link;
use super::objects::Named;
use super::tables::Tables;

/// What the earlier passes give this one.
pub(super) struct Source<'a> {
    pub file: &'a DwgFile,
    pub objects: &'a [ObjectRef],
    pub tables: &'a Tables,
    pub blocks: &'a Blocks,
    pub named: &'a Named,
    /// The entities' extension dictionaries, reactors and materials, by entity.
    pub links: &'a HashMap<u64, Link>,
    /// The named object dictionary.
    pub root: u64,
    /// The objects the earlier passes read.
    pub done: &'a HashSet<u64>,
}

/// Most owners followed from an object up to the named object dictionary.
const MAX_DEPTH: usize = 64;

/// Most members of one group that are read.
const MAX_MEMBERS: u32 = 1_000_000;

/// The structure the drawing rebuilds on save, as it does a DXF file's, when nothing of it is
/// kept: dictionaries, their placeholders and scales, DraftCraft's records, dictionary variables
/// and the raster variables of its images.
const REBUILT: &[&str] = &["DICTIONARY", "ACDBDICTIONARYWDFLT", "ACDBPLACEHOLDER", "SCALE", "XRECORD", "DICTIONARYVAR", "RASTERVARIABLES"];

/// One object of the file this pass decodes: its type, what the codec makes of it (`None` for a
/// type it doesn't decode) and the references every object starts with.
struct Obj {
    type_name: String,
    value: Option<NonGraphic>,
    refs: CommonRefs,
}

/// A number as a DXF group's text, as the DXF reader keeps it.
fn real(v: f64) -> String {
    draftcraft_dxf::Value::Real(v).as_str()
}

fn tag(code: i32, value: impl Into<String>) -> RawTag {
    RawTag { code, value: value.into() }
}

fn hex(h: u64) -> String {
    format!("{h:X}")
}

/// An extension record's or style object's items as DXF groups: a point as its x, y and z.
fn item_tags(items: &[(u16, XrValue)], out: &mut Vec<RawTag>) {
    for (code, v) in items {
        let code = i32::from(*code);
        match v {
            XrValue::Str(s) => out.push(tag(code, s.clone())),
            XrValue::Point(p) => {
                out.push(tag(code, real(p.x)));
                out.push(tag(code + 10, real(p.y)));
                out.push(tag(code + 20, real(p.z)));
            }
            XrValue::Real(x) => out.push(tag(code, real(*x))),
            XrValue::Int8(i) => out.push(tag(code, i.to_string())),
            XrValue::Int16(i) => out.push(tag(code, i.to_string())),
            XrValue::Int32(i) => out.push(tag(code, i.to_string())),
            XrValue::Int64(i) => out.push(tag(code, i.to_string())),
            XrValue::Bool(b) => out.push(tag(code, if *b { "1" } else { "0" })),
            XrValue::Bytes(b) => out.push(tag(code, b.iter().map(|x| format!("{x:02X}")).collect::<String>())),
            XrValue::Handle(h) => out.push(tag(code, hex(h.0))),
        }
    }
}

/// The DXF groups of an object after its type, as AutoCAD writes them and the DXF reader keeps
/// them: its handle, reactors, extension dictionary and owner, then its own groups. `None` for a
/// value that has no DXF form here.
fn object_tags(handle: u64, o: &Obj) -> Option<Vec<RawTag>> {
    let mut t = vec![tag(5, hex(handle))];
    if !o.refs.reactors.is_empty() {
        t.push(tag(102, "{ACAD_REACTORS"));
        t.extend(o.refs.reactors.iter().map(|r| tag(330, hex(*r))));
        t.push(tag(102, "}"));
    }
    if o.refs.xdictionary != 0 {
        t.push(tag(102, "{ACAD_XDICTIONARY"));
        t.push(tag(360, hex(o.refs.xdictionary)));
        t.push(tag(102, "}"));
    }
    t.push(tag(330, hex(o.refs.owner)));
    match o.value.as_ref()? {
        NonGraphic::Dictionary(dict) => {
            t.push(tag(100, "AcDbDictionary"));
            // The hard-owner flag only when set, as AutoCAD's DXF has it.
            if dict.hard_owner != 0 {
                t.push(tag(280, dict.hard_owner.to_string()));
            }
            t.push(tag(281, dict.cloning.to_string()));
            let code = if dict.hard_owner != 0 { 360 } else { 350 };
            for (key, h) in &dict.entries {
                t.push(tag(3, key.clone()));
                t.push(tag(code, hex(h.0)));
            }
            if let Some(default) = dict.default {
                t.push(tag(100, "AcDbDictionaryWithDefault"));
                t.push(tag(340, hex(default.0)));
            }
        }
        NonGraphic::XRecord(x) => {
            t.push(tag(100, "AcDbXrecord"));
            t.push(tag(280, x.cloning.to_string()));
            item_tags(&x.items, &mut t);
        }
        NonGraphic::Placeholder => {}
        NonGraphic::DictionaryVar { schema, value } => {
            t.push(tag(100, "DictionaryVariables"));
            t.push(tag(280, schema.to_string()));
            t.push(tag(1, value.clone()));
        }
        NonGraphic::Scale(s) => {
            t.push(tag(100, "AcDbScale"));
            t.push(tag(70, s.flag.to_string()));
            t.push(tag(300, s.name.clone()));
            t.push(tag(140, real(s.paper_units)));
            t.push(tag(141, real(s.drawing_units)));
            t.push(tag(290, if s.unit_scale { "1" } else { "0" }));
        }
        NonGraphic::Groups(g) => item_tags(&g.items, &mut t),
        // Modelled by the drawing: never kept as groups.
        NonGraphic::Layout(_) | NonGraphic::MLineStyle(_) => return None,
    }
    Some(t)
}

/// GROUP (§20.4.?): description, unnamed and selectable flags, members; as its DXF groups.
fn group(r: &mut ObjectReader) -> DwgResult<(Groups, CommonRefs)> {
    let refs = r.common_refs()?;
    let description = r.text()?;
    let unnamed = r.bs()?;
    let selectable = r.bs()?;
    let n = r.bl()?;
    if n > MAX_MEMBERS {
        return Err(draftcraft_dwg::records::Error::format(format!("too many group members: {n}")));
    }
    let mut items = vec![
        (100, XrValue::Str("AcDbGroup".into())),
        (300, XrValue::Str(description)),
        (70, XrValue::Int16(i16::try_from(unnamed).unwrap_or(0))),
        (71, XrValue::Int16(i16::try_from(selectable).unwrap_or(1))),
    ];
    for _ in 0..n {
        items.push((340, XrValue::Handle(draftcraft_dwg::file::Handle(r.reference()?))));
    }
    Ok((Groups { kind: "GROUP".into(), items }, refs))
}

/// WIPEOUTVARIABLES: whether wipeout frames show (DXF 70), as its DXF groups.
fn wipeout_variables(r: &mut ObjectReader) -> DwgResult<(Groups, CommonRefs)> {
    let refs = r.common_refs()?;
    let frame = r.bs()?;
    let items = vec![(100, XrValue::Str("AcDbWipeoutVariables".into())), (70, XrValue::Int16(i16::try_from(frame).unwrap_or(1)))];
    Ok((Groups { kind: "WIPEOUTVARIABLES".into(), items }, refs))
}

/// Most vertices of one clip boundary that are read.
const MAX_CLIP_VERTICES: u32 = 1 << 16;

/// SPATIAL_FILTER (§20.4.99): a block reference's clip boundary, as the DXF groups AutoCAD
/// writes of it: the vertices, the boundary's normal and origin, whether it is shown, the front
/// and back clip planes with their distances, then the inverse of the block's transform and the
/// boundary's own, twelve numbers each.
fn spatial_filter(r: &mut ObjectReader) -> DwgResult<(Groups, CommonRefs)> {
    let refs = r.common_refs()?;
    let n = u32::from(r.bs()?);
    if n > MAX_CLIP_VERTICES {
        return Err(draftcraft_dwg::records::Error::format(format!("too many clip vertices: {n}")));
    }
    let mut items = vec![
        (100, XrValue::Str("AcDbFilter".into())),
        (100, XrValue::Str("AcDbSpatialFilter".into())),
        (70, XrValue::Int16(i16::try_from(n).unwrap_or(0))),
    ];
    for _ in 0..n {
        let [x, y] = r.rd2()?;
        items.push((10, XrValue::Real(x)));
        items.push((20, XrValue::Real(y)));
    }
    let point = |[x, y, z]: [f64; 3]| XrValue::Point(draftcraft_dwg::records::geom::Vec3 { x, y, z });
    items.push((210, point(r.bd3()?)));
    items.push((11, point(r.bd3()?)));
    items.push((71, XrValue::Int16(i16::try_from(r.bs()?).unwrap_or(1))));
    for (flag, distance) in [(72, 40), (73, 41)] {
        let on = r.bs()?;
        items.push((flag, XrValue::Int16(i16::try_from(on).unwrap_or(0))));
        if on != 0 {
            items.push((distance, XrValue::Real(r.bd()?)));
        }
    }
    for _ in 0..24 {
        items.push((40, XrValue::Real(r.bd()?)));
    }
    Ok((Groups { kind: "SPATIAL_FILTER".into(), items }, refs))
}

/// The 16-bit item `code` of groups.
fn group_int(g: &Groups, code: u16) -> Option<i64> {
    g.items.iter().find(|(c, _)| *c == code).and_then(|(_, v)| match v {
        XrValue::Int8(i) => Some(i64::from(*i)),
        XrValue::Int16(i) => Some(i64::from(*i)),
        XrValue::Int32(i) => Some(i64::from(*i)),
        _ => None,
    })
}

/// A DXF CLASS record's groups, for a kept object of a class.
fn class_tags(c: &draftcraft_dwg::file::ClassInfo) -> Vec<RawTag> {
    vec![
        tag(1, c.dxf_name.clone()),
        tag(2, c.cpp_name.clone()),
        tag(3, c.app_name.clone()),
        tag(90, c.proxy_flags.to_string()),
        tag(91, c.instance_count.to_string()),
        tag(280, if c.was_zombie { "1" } else { "0" }),
        tag(281, if c.is_entity { "1" } else { "0" }),
    ]
}

/// What says whether an object is kept: the objects by handle, the named object dictionary and
/// its entries, the extension dictionary of each entity and record, and the drawing's entities
/// (true for one of a type the drawing doesn't model) and records by handle.
struct Keeping<'a> {
    objs: &'a HashMap<u64, Obj>,
    root: u64,
    root_keys: &'a HashMap<u64, String>,
    xdict_of: &'a HashMap<u64, u64>,
    ents: &'a HashMap<u64, bool>,
    ids: &'a HashMap<u64, ForeignKey>,
}

impl Keeping<'_> {
    /// An extension dictionary of an entity or record (not of another object).
    fn is_xdict(&self, h: u64) -> bool {
        let Some(o) = self.objs.get(&h) else { return false };
        let owner = o.refs.owner;
        !self.objs.contains_key(&owner) && self.xdict_of.get(&owner) == Some(&h) && (self.ents.contains_key(&owner) || self.ids.contains_key(&owner))
    }

    /// The keys of a dictionary's entries.
    fn keys(&self, h: u64) -> Vec<(&str, u64)> {
        match self.objs.get(&h).and_then(|o| o.value.as_ref()) {
            Some(NonGraphic::Dictionary(dict)) => dict.entries.iter().map(|(k, e)| (k.as_str(), e.0)).collect(),
            _ => Vec::new(),
        }
    }

    /// Whether the object `handle` is kept, by the DXF reader's rule (`dxf_raw::retained_objects`):
    /// its owners lead to an entry of the named object dictionary DraftCraft doesn't write
    /// itself, to an extension dictionary's entry it doesn't model, or to an entity of a type it
    /// doesn't model.
    fn retained(&self, handle: u64, memo: &mut HashMap<u64, bool>) -> bool {
        let mut path = Vec::new();
        let mut at = handle;
        let keep = loop {
            if let Some(&known) = memo.get(&at) {
                break known;
            }
            if path.contains(&at) || path.len() >= MAX_DEPTH {
                break false;
            }
            path.push(at);
            let Some(o) = self.objs.get(&at) else { break false };
            let owner = o.refs.owner;
            if owner == self.root {
                break self.root_keys.get(&at).is_some_and(|key| !crate::dxf_raw::modelled_key(key));
            }
            if self.objs.contains_key(&owner) {
                if self.is_xdict(owner) && self.keys(owner).iter().any(|(k, e)| *e == at && crate::dxf_raw::modelled_xdict_key(k)) {
                    break false;
                }
                at = owner;
                continue;
            }
            if self.is_xdict(at) {
                break self.keys(at).iter().any(|(k, _)| !crate::dxf_raw::modelled_xdict_key(k));
            }
            break self.ents.get(&owner) == Some(&true);
        };
        for p in path {
            memo.insert(p, keep);
        }
        keep
    }
}

/// The roles of the file's handles (`ForeignKey`), as the DXF reader names them: table records
/// and tables by type and name, block records, the named object dictionary, its entries and
/// their entries.
fn identities(src: &Source, objs: &HashMap<u64, Obj>, root_entries: &[(String, u64)]) -> HashMap<u64, ForeignKey> {
    let up = |s: &str| s.trim().to_ascii_uppercase();
    let mut out: HashMap<u64, ForeignKey> = HashMap::new();
    for o in src.objects.iter().filter(|o| !o.is_entity) {
        let record = |kind: &str, names: &HashMap<u64, String>| names.get(&o.handle).filter(|n| !n.is_empty()).map(|n| (kind.to_string(), up(n)));
        let key = match o.type_name.as_str() {
            "LAYER" => record("LAYER", &src.tables.layers),
            "STYLE" => record("STYLE", &src.tables.styles),
            "LTYPE" => record("LTYPE", &src.tables.linetypes),
            "APPID" => record("APPID", &src.tables.apps),
            "DIMSTYLE" => record("DIMSTYLE", &src.tables.dim_styles),
            "UCS" => record("UCS", &src.tables.ucss),
            "BLOCK_HEADER" => record("BLOCK_RECORD", &src.blocks.names),
            "BLOCK_CONTROL" => Some(("TABLE".into(), "BLOCK_RECORD".into())),
            t => t.strip_suffix("_CONTROL").filter(|t| *t != "VX").map(|t| ("TABLE".to_string(), t.to_string())),
        };
        if let Some(key) = key {
            out.entry(o.handle).or_insert(key);
        }
    }
    if src.root != 0 {
        out.entry(src.root).or_insert(("NOD".into(), String::new()));
    }
    for (key, h) in root_entries {
        out.entry(*h).or_insert(("NOD".into(), up(key)));
        if let Some(NonGraphic::Dictionary(dict)) = objs.get(h).and_then(|o| o.value.as_ref()) {
            for (name, e) in &dict.entries {
                out.entry(e.0).or_insert((format!("NOD/{}", up(key)), up(name)));
            }
        }
    }
    out
}

/// Every entity of the drawing by handle: true for one of a type the drawing doesn't model.
fn entity_types(d: &Drawing) -> HashMap<u64, bool> {
    std::iter::once(&d.model)
        .chain(d.layouts.iter().map(|l| &l.entities))
        .chain(d.blocks.values().map(|b| &b.entities))
        .flat_map(|s| s.iter())
        .map(|e| (e.handle.0, matches!(e.kind, EntityKind::Unknown(_))))
        .collect()
}

/// What a table record, table or block record carries beyond the model: its extension
/// dictionary and reactors (those kept are chosen later) and other applications' extended data.
fn record_extras(src: &Source, ids: &HashMap<u64, ForeignKey>) -> HashMap<u64, (CommonRefs, Vec<RawTag>)> {
    let mut out = HashMap::new();
    for o in src.objects.iter().filter(|o| !o.is_entity) {
        let Some((kind, _)) = ids.get(&o.handle).filter(|(kind, _)| !kind.starts_with("NOD")) else { continue };
        let Ok(mut r) = src.file.read_object(o) else { continue };
        let xdata = crate::dxf_raw::foreign_xdata(kind, &super::entities::xdata_tags(&r.xdata(), &src.tables.apps));
        let Ok(refs) = r.common_refs() else { continue };
        if refs.xdictionary != 0 || !refs.reactors.is_empty() || !xdata.is_empty() {
            out.insert(o.handle, (refs, xdata));
        }
    }
    out
}

/// The objects the earlier passes left, decoded.
struct Decoded {
    objs: HashMap<u64, Obj>,
    /// Their handles in the file's order.
    order: Vec<u64>,
    /// The handles accounted for: read into the drawing, kept, or counted.
    read: HashSet<u64>,
    /// Table styles as the DXF groups AutoCAD writes of them.
    style_tags: Vec<(u64, Vec<draftcraft_dxf::Tag>)>,
    /// Dictionary entries by the handle of the object they name.
    keys: HashMap<u64, String>,
    /// The named object dictionary's entries.
    root_entries: Vec<(String, u64)>,
}

/// Decode the objects the earlier passes left (`done`).
fn decode(src: &Source, unread: &mut Unread) -> Decoded {
    let Source { file, objects, tables, root, done, .. } = *src;
    let text_styles: Vec<(String, u64)> = tables.styles.iter().map(|(h, n)| (n.clone(), *h)).collect();
    let mut objs: HashMap<u64, Obj> = HashMap::new();
    let mut order: Vec<u64> = Vec::new();
    let mut read = HashSet::new();
    let mut style_tags: Vec<(u64, Vec<draftcraft_dxf::Tag>)> = Vec::new();
    for o in objects.iter().filter(|o| !o.is_entity && !done.contains(&o.handle)) {
        let Ok(mut r) = file.read_object(o) else { continue };
        // Table styles by the DXF reader's rule, from the DXF groups AutoCAD writes of them.
        if o.type_name == "TABLESTYLE" {
            match super::table::table_style(&mut r, &tables.styles) {
                Ok(tags) => style_tags.push((o.handle, tags)),
                Err(_) => unread.count("TABLESTYLE (not decoded)"),
            }
            read.insert(o.handle);
            continue;
        }
        let own = match o.type_name.as_str() {
            "GROUP" => Some(group(&mut r)),
            "WIPEOUTVARIABLES" => Some(wipeout_variables(&mut r)),
            "SPATIAL_FILTER" => Some(spatial_filter(&mut r)),
            _ => None,
        };
        let decoded = match own {
            Some(res) => Some(res.map(|(g, refs)| (NonGraphic::Groups(g), refs))),
            None => r.non_graphic(&text_styles),
        };
        let (value, refs) = match decoded {
            Some(Ok((value, refs))) => (Some(value), refs),
            Some(Err(_)) => {
                unread.count(&format!("{} (not decoded)", o.type_name));
                read.insert(o.handle);
                continue;
            }
            // A type the codec doesn't decode: its owner says whether the drawing would keep it.
            None => match r.common_refs() {
                Ok(refs) => (None, refs),
                Err(_) => continue,
            },
        };
        objs.insert(o.handle, Obj { type_name: o.type_name.clone(), value, refs });
        order.push(o.handle);
    }
    // Dictionary entries by the handle of the object they name.
    let mut keys: HashMap<u64, String> = HashMap::new();
    let mut root_entries: Vec<(String, u64)> = Vec::new();
    for &h in &order {
        if let Some(NonGraphic::Dictionary(dict)) = objs.get(&h).and_then(|o| o.value.as_ref()) {
            for (key, e) in &dict.entries {
                keys.entry(e.0).or_insert_with(|| key.clone());
                if h == root {
                    root_entries.push((key.clone(), e.0));
                }
            }
        }
    }
    Decoded { objs, order, read, style_tags, keys, root_entries }
}

/// The drawing's part of the objects, by the DXF reader's rules: the scale list, DraftCraft's own
/// records, the layers' plot styles, table styles, groups, the wipeout frame setting. Returns the
/// objects read into the model.
fn model(src: &Source, dec: &Decoded, d: &mut Drawing, unread: &mut Unread) -> HashSet<u64> {
    let Source { tables, named, .. } = *src;
    let Decoded { objs, order, style_tags, keys, root_entries, .. } = dec;
    let entry = |key: &str| root_entries.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, h)| *h);
    let dictionary = |h: u64| match objs.get(&h).and_then(|o| o.value.as_ref()) {
        Some(NonGraphic::Dictionary(dict)) => Some(dict),
        _ => None,
    };

    // The annotation scale list, in its dictionary's order.
    let scale_handles: Vec<u64> = match entry("ACAD_SCALELIST").and_then(dictionary) {
        Some(dict) => dict.entries.iter().map(|(_, h)| h.0).collect(),
        None => order.iter().copied().filter(|h| objs.get(h).is_some_and(|o| o.type_name == "SCALE")).collect(),
    };
    let scales: Vec<draftcraft_doc::AnnoScale> = scale_handles
        .iter()
        .filter_map(|h| match objs.get(h).and_then(|o| o.value.as_ref()) {
            Some(NonGraphic::Scale(s)) => crate::dxf_ext::scale_from(&s.name, s.paper_units, s.drawing_units),
            _ => None,
        })
        .fold(Vec::new(), |mut list, s| {
            if list.len() < draftcraft_doc::MAX_SCALES && !list.iter().any(|x: &draftcraft_doc::AnnoScale| x.name.eq_ignore_ascii_case(&s.name)) {
                list.push(s);
            }
            list
        });
    crate::dxf_read::apply_scales(d, &scales);

    // DraftCraft's own records, under their keys in the named object dictionary.
    let text = |key: &str| -> Option<String> {
        let h = entry(key)?;
        let Some(NonGraphic::XRecord(x)) = objs.get(&h).and_then(|o| o.value.as_ref()) else { return None };
        let mut out = String::new();
        for (code, v) in &x.items {
            if let (1 | 3, XrValue::Str(s)) = (code, v) {
                if out.len() > crate::dxf_ext::MAX_PAYLOAD {
                    break;
                }
                out.push_str(s);
            }
        }
        Some(out)
    };
    crate::dxf_read::apply_own_records(d, &text);
    // A file DraftCraft wrote names its own next handle (#495), as the DXF reader takes it.
    if let Some(seed) = text(crate::dxf_ext::HANDSEED_KEY).and_then(|t| Handle::parse_hex(t.trim())) {
        d.bump_handseed(Handle(seed.0.saturating_sub(1)));
    }

    // The layers' plot styles, named by the plot style dictionary's placeholders.
    let plot_styles: HashMap<u64, String> = entry(crate::dxf_ext::PLOT_STYLE_NAMES_KEY)
        .and_then(dictionary)
        .map(|dict| dict.entries.iter().map(|(name, h)| (h.0, name.clone())).collect())
        .unwrap_or_default();
    for (layer, h) in &tables.layer_plot_styles {
        match (plot_styles.get(h), d.layers.iter_mut().find(|l| &l.name == layer)) {
            (Some(name), Some(l)) => l.plot_style.clone_from(name),
            _ => unread.count("LAYER plot style"),
        }
    }

    // Table styles, named by the ACAD_TABLESTYLE dictionary, by the DXF reader's rule: a name the
    // drawing has is replaced.
    let mut modelled: HashSet<u64> = HashSet::new();
    for (h, tags) in style_tags {
        let Some(name) = named.table_styles.get(h).filter(|n| !n.is_empty()) else {
            unread.count("TABLESTYLE of no dictionary entry");
            continue;
        };
        let st = crate::dxf_table::read_table_style(name, tags);
        match d.table_styles.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name)) {
            Some(x) => *x = st,
            None => d.table_styles.push(st),
        }
    }

    // Groups, named by their dictionary (ACAD_GROUP), by the DXF reader's rule.
    let mut groups = Vec::new();
    for &h in order.iter() {
        let Some(o) = objs.get(&h) else { continue };
        let (Some(NonGraphic::Groups(g)), "GROUP") = (&o.value, o.type_name.as_str()) else { continue };
        let Some(name) = keys.get(&h).filter(|n| !n.is_empty()) else { continue };
        let description = g.items.iter().find_map(|(c, v)| if let (300, XrValue::Str(s)) = (c, v) { Some(s.clone()) } else { None });
        let members = g.items.iter().filter_map(|(c, v)| if let (340, XrValue::Handle(m)) = (c, v) { Some(Handle(m.0)) } else { None }).collect();
        groups.push(Group {
            name: name.clone(),
            description: description.unwrap_or_default(),
            selectable: group_int(g, 71).unwrap_or(1) != 0,
            members,
        });
        modelled.insert(h);
    }
    crate::dxf_read::put_groups(d, groups);
    // The wipeout frame setting (WIPEOUTFRAME), which the DXF reader takes from the object too.
    for o in objs.values().filter(|o| o.type_name == "WIPEOUTVARIABLES") {
        if let Some(NonGraphic::Groups(g)) = &o.value
            && let Some(v) = group_int(g, 70).filter(|v| (0..=2).contains(v))
        {
            d.header.set_i64("WIPEOUTFRAME", v);
        }
    }

    modelled
}

/// What the drawing doesn't model, kept as the DXF reader keeps it; the rest of the structure
/// (the dictionaries of what it models, their scales and placeholders, DraftCraft's records) it
/// rebuilds on save, as it does a DXF file's. Anything else is counted.
fn keep(src: &Source, dec: &mut Decoded, modelled: &HashSet<u64>, d: &mut Drawing, unread: &mut Unread) {
    let Source { file, root, .. } = *src;
    let Decoded { objs, order, read, root_entries, .. } = dec;
    let root_keys: HashMap<u64, String> = root_entries.iter().map(|(k, h)| (*h, k.clone())).collect();
    let ents = entity_types(d);
    let ids = identities(src, objs, root_entries);
    let records = record_extras(src, &ids);
    let mut xdict_of: HashMap<u64, u64> = src.links.iter().filter(|(_, l)| l.xdictionary != 0).map(|(h, l)| (*h, l.xdictionary)).collect();
    xdict_of.extend(records.iter().filter(|(_, (refs, _))| refs.xdictionary != 0).map(|(h, (refs, _))| (*h, refs.xdictionary)));
    let keeping = Keeping { objs, root, root_keys: &root_keys, xdict_of: &xdict_of, ents: &ents, ids: &ids };
    let mut memo = HashMap::new();
    let mut foreign = Foreign::default();
    let mut classes: HashSet<String> = HashSet::new();
    let mut kept: HashSet<u64> = HashSet::new();
    for &h in order.iter() {
        let Some(o) = objs.get(&h) else { continue };
        if modelled.contains(&h) {
            read.insert(h);
            continue;
        }
        if !keeping.retained(h, &mut memo) {
            if REBUILT.contains(&o.type_name.as_str()) {
                read.insert(h);
            }
            continue;
        }
        let Some(mut tags) = object_tags(h, o) else {
            unread.count(&o.type_name);
            read.insert(h);
            continue;
        };
        // An extension dictionary's modelled entries are written from the model.
        if keeping.is_xdict(h) {
            crate::dxf_raw::strip_entries(&mut tags);
        }
        kept.insert(h);
        if o.refs.owner == root
            && let Some(key) = root_keys.get(&h)
        {
            foreign.dictionary.push((key.clone(), Handle(h)));
        }
        if let Some(c) = file.class_of(&o.type_name)
            && classes.insert(c.dxf_name.to_ascii_uppercase())
        {
            foreign.classes.push(class_tags(&c));
        }
        foreign.objects.push(ForeignObject { handle: Handle(h), kind: o.type_name.clone(), tags });
        read.insert(h);
    }
    // What entities, table records, tables and block records carry beyond the model: the kept
    // extension dictionary and reactors, a material, other applications' extended data.
    let extra = |xdict: u64, reactors: &[u64]| ForeignExtra {
        xdict: Some(xdict).filter(|x| kept.contains(x)).map(Handle),
        reactors: reactors.iter().filter(|r| kept.contains(r)).map(|r| Handle(*r)).collect(),
        ..ForeignExtra::default()
    };
    for (h, l) in src.links {
        let Some(&unknown) = ents.get(h) else { continue };
        let mut x = extra(l.xdictionary, &l.reactors);
        if !unknown && l.material != 0 {
            x.props.push(tag(347, hex(l.material)));
        }
        if !x.is_empty() {
            foreign.entities.insert(Handle(*h), x);
        }
    }
    for (h, (refs, xdata)) in &records {
        let Some(key) = ids.get(h) else { continue };
        let mut x = extra(refs.xdictionary, &refs.reactors);
        x.xdata.clone_from(xdata);
        if !x.is_empty() {
            foreign.records.insert(key.clone(), x);
        }
    }
    // The roles of the handles the kept data refers to, for what the drawing writes itself
    // under a new handle.
    let mut refs: Vec<u64> = Vec::new();
    let mut note = |tags: &[RawTag]| {
        refs.extend(tags.iter().filter(|t| crate::dxf_raw::is_pointer(t.code)).filter_map(|t| Handle::parse_hex(&t.value)).map(|h| h.0))
    };
    for o in &foreign.objects {
        note(&o.tags);
    }
    for x in foreign.entities.values().chain(foreign.records.values()) {
        note(&x.xdata);
        note(&x.props);
    }
    for e in
        std::iter::once(&d.model).chain(d.layouts.iter().map(|l| &l.entities)).chain(d.blocks.values().map(|b| &b.entities)).flat_map(|s| s.iter())
    {
        note(&e.common.xdata);
    }
    for r in refs {
        if let (Some(k), false) = (ids.get(&r), kept.contains(&r)) {
            foreign.targets.insert(Handle(r), k.clone());
        }
    }
    if !foreign.is_empty() {
        let f = std::sync::Arc::make_mut(&mut d.foreign);
        f.objects.extend(foreign.objects);
        f.dictionary.extend(foreign.dictionary);
        f.classes.extend(foreign.classes);
        for (h, x) in foreign.entities {
            let e = f.entities.entry(h).or_default();
            e.xdict = x.xdict;
            e.reactors = x.reactors;
            e.props.extend(x.props);
        }
        for (k, x) in foreign.records {
            let e = f.records.entry(k).or_default();
            e.xdict = x.xdict;
            e.reactors = x.reactors;
            e.xdata = x.xdata;
        }
        f.targets.extend(foreign.targets);
    }
}

/// Read the objects the earlier passes left (`done`): the drawing's part of them, and what it
/// keeps of the rest. Returns the handles accounted for.
pub(super) fn read(src: &Source, d: &mut Drawing, unread: &mut Unread) -> HashSet<u64> {
    let mut dec = decode(src, unread);
    let modelled = model(src, &dec, d, unread);
    keep(src, &mut dec, &modelled, d, unread);
    dec.read
}
