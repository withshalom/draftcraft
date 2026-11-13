//! The entities of a DWG file, into the drawing: each is decoded field by field (Open Design
//! Alliance specification, chapter 20) straight into the drawing's own entity, with the
//! conventions the drawing keeps: angles in radians, the points of flat entities out of their
//! object coordinate system, and a mirrored one (extrusion pointing down) as the geometry it draws.

use std::collections::{HashMap, HashSet};

use draftcraft_doc::{
    Arc, Attrib, AttribModes, AttribProps, Circle, Common, DimKind, Dimension, Drawing, Ellipse, Entity, EntityKind, Face3d, Gradient, HAlign,
    Handle, Hatch, HatchLoop, Image, Insert, Leader, Line, Lineweight, LwPolyline, MAX_FRAME_CHARS, MText, MTextColumnType, MTextColumns,
    MultilineAttrib, Point, PolyfaceMesh, Polyline3d, RawTag, RayLine, Solid, Text, Tolerance, VAlign, Viewport, Wipeout,
};
use draftcraft_dwg::DwgVersion as V;
use draftcraft_dwg::file::{ColorValue, DwgFile, EntityLinetype, EntityRefs, EntitySpace, ObjectReader, ObjectRef, XDataBlock, XDataItem};
use draftcraft_dwg::records::Result as DwgResult;
use draftcraft_geom::{PolyVertex, Vec2, Vec3};

use super::Unread;
use super::blocks::{Blocks, Space};
use super::tables::Tables;
use crate::dxf_read::{SplineParts, mesh_faces_of, mirror_text, mtext_dxf, ocs_n, spline_from, transparency_440};

/// Most entities of one drawing that are read.
const MAX_ENTITIES: usize = 1 << 24;

/// The names an entity's references resolve to.
struct Names<'a> {
    tables: &'a Tables,
    blocks: &'a HashMap<u64, String>,
    /// What the file's objects give entities: multileader style names, image and underlay
    /// definitions.
    objects: &'a super::objects::Named,
    /// Text styles, blocks and linetypes by handle in hexadecimal, as a dimension's overrides
    /// name them: made once, not per dimension.
    by_hex: (HashMap<String, String>, HashMap<String, String>, HashMap<String, String>),
}

/// What an entity refers to that the objects pass keeps with the drawing's other foreign data:
/// its extension dictionary, the objects that react to it, its material.
pub(super) struct Link {
    pub xdictionary: u64,
    pub reactors: Vec<u64>,
    pub material: u64,
}

/// One entity's place: the block record that owns it, and the entity after it in the chain.
struct Place {
    record: u64,
    next: Option<u64>,
}

/// What reading one entity gave: the entity, when its type is read, and what of it was not.
#[derive(Default)]
struct Decoded {
    entity: Option<Entity>,
    /// An attribute of a block reference, with its own properties, waiting for its reference.
    attrib: Option<(Attrib, Common)>,
    /// A block reference's attributes: listed (2004 and later) or its first and last (up to
    /// 2000), and the SEQEND that ends them.
    attribs: Option<Attribs>,
    /// A polyline waiting for its vertices: its kind, common properties and the vertices it names.
    polyline: Option<(PolyKind, Common, Attribs)>,
    /// A vertex or polyface face waiting for its polyline.
    vertex: Option<Vertex>,
    /// A multileader's block attribute values (ATTDEF handle in hexadecimal, value), waiting for
    /// the attribute definitions of its block.
    mleader_attribs: Option<Vec<(String, String)>>,
    /// The sheet viewport DraftCraft's writer adds, which the drawing leaves implicit.
    paper_view: bool,
    /// Other objects the entity holds in the drawing (an image's definition and reactor).
    held: Vec<u64>,
    /// A table's block (the geometry it is drawn as, which the drawing rebuilds) and, for a
    /// table from another writer, its style's name, whose text height it takes.
    table: Option<(Option<String>, Option<Option<String>>)>,
    /// The colour book colour's true colour and names, kept for saving (as the DXF reader keeps
    /// group 430).
    color_book: Option<(String, String)>,
    /// A leader to text: its hook direction flag and x direction ([`add_hook_lines`]).
    leader_hook: Option<(bool, Vec2)>,
    partial: Vec<&'static str>,
}

/// The attributes a block reference names.
struct Attribs {
    listed: Vec<u64>,
    chain: Option<(u64, u64)>,
    seqend: u64,
}

fn v3(p: [f64; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}

/// The properties every entity has.
fn common(r: &ObjectReader, refs: &EntityRefs, n: &Names, partial: &mut Vec<&'static str>) -> Common {
    let mut c = Common::default();
    let Some(head) = r.entity() else { return c };
    c.layer = n.tables.layers.get(&refs.layer).cloned().unwrap_or_else(|| "0".into());
    if let Some(color) = super::tables::color(head.color) {
        c.color = color;
    }
    // A colour book colour is its DBCOLOR object's; in an older version's file, the true colour
    // AutoCAD kept in its round-trip data.
    if let Some(book) = n.objects.colors.get(&refs.color_book) {
        c.color = book.color;
    } else if let Some((color, _)) = round_trip_color(r, n) {
        c.color = color;
    }
    c.linetype = match refs.linetype {
        EntityLinetype::ByLayer => "ByLayer".into(),
        EntityLinetype::ByBlock => "ByBlock".into(),
        EntityLinetype::Continuous => "Continuous".into(),
        EntityLinetype::Record(h) => n.tables.linetypes.get(&h).cloned().unwrap_or_else(|| "ByLayer".into()),
    };
    c.lineweight = Lineweight::from_dxf(i16::try_from(super::header::lineweight(i64::from(head.lineweight))).unwrap_or(-1));
    c.ltscale = if head.linetype_scale.is_finite() && head.linetype_scale > 0.0 { head.linetype_scale } else { 1.0 };
    if let Some(t) = head.transparency {
        c.transparency = transparency_440(i64::from(t));
    }
    c.visible = !head.invisible;
    // What the entity refers to that is read with the objects.
    if refs.plotstyle != 0 {
        partial.push("entity plot style");
    }
    c.xdata = xdata(&r.xdata(), n, partial);
    c
}

/// The properties an attribute carries itself, as a DXF holds them: those it doesn't take from
/// its layer. Its layer counts only when it differs from its block reference's (an attribute
/// defined on layer 0 goes on the reference's layer, as it is drawn anyway).
fn attrib_props(c: &Common, insert_layer: &str) -> AttribProps {
    AttribProps {
        layer: (!c.layer.is_empty() && !c.layer.eq_ignore_ascii_case(insert_layer)).then(|| c.layer.clone()),
        color: (c.color != draftcraft_color::Color::ByLayer).then_some(c.color),
        linetype: (!c.linetype.eq_ignore_ascii_case("ByLayer")).then(|| c.linetype.clone()),
        lineweight: (c.lineweight != Lineweight::ByLayer).then_some(c.lineweight),
    }
}

/// What AutoCAD keeps of a later version's properties in a file saved as an older version: ACAD
/// extended data sequences that its own DXF of the file holds as the properties instead. `RTTcAl`
/// is the colour (its flags and index, its colour word, its colour book name or `$$NoName$$`),
/// `RTMaterial` the material. Returns the items without them, and the colour sequence's values.
fn round_trip(items: &[XDataItem]) -> (Vec<&XDataItem>, Option<(i16, i32, &str)>) {
    let mut kept = Vec::with_capacity(items.len());
    let mut color = None;
    let mut i = 0;
    while let Some(item) = items.get(i) {
        match (item, items.get(i + 1), items.get(i + 2), items.get(i + 3)) {
            (XDataItem::Text(t), Some(XDataItem::Int16(flags)), Some(XDataItem::Int32(word)), Some(XDataItem::Text(book))) if t == "RTTcAl" => {
                color = Some((*flags, *word, book.as_str()));
                i += 4;
            }
            (XDataItem::Text(t), Some(XDataItem::Handle(_)), _, _) if t == "RTMaterial" => i += 2,
            _ => {
                kept.push(item);
                i += 1;
            }
        }
    }
    (kept, color)
}

/// The true colour and colour book name an older version's file keeps in AutoCAD's round-trip
/// data ([`round_trip`]), while the entity's colour is still the index AutoCAD saved with them: a
/// program that changed the colour since left them stale.
fn round_trip_color(r: &ObjectReader, n: &Names) -> Option<(draftcraft_color::Color, Option<String>)> {
    let head = r.entity()?;
    let index = match head.color {
        ColorValue::ByBlock => 0,
        ColorValue::ByLayer => 256,
        ColorValue::Index(i) => i16::from(i),
        _ => return None,
    };
    let blocks = r.xdata();
    let acad = blocks.iter().find(|b| n.tables.apps.get(&b.app).is_some_and(|a| a.eq_ignore_ascii_case("ACAD")))?;
    let (_, Some((flags, word, book))) = round_trip(&acad.items) else { return None };
    let word = u32::from_ne_bytes(word.to_ne_bytes());
    if (flags & 0x01FF) != index || word >> 24 != 0xC2 {
        return None;
    }
    let color = draftcraft_color::Color::True(draftcraft_color::Rgb::from_u32(word & 0x00FF_FFFF));
    let book = (book != "$$NoName$$" && !book.trim().is_empty()).then(|| book.to_string());
    Some((color, book))
}

/// The colour book colour's true colour and names, kept for saving: its DBCOLOR object's, or the
/// ones of AutoCAD's round-trip data.
fn color_book(r: &ObjectReader, refs: &EntityRefs, n: &Names) -> Option<(String, String)> {
    if let Some(book) = n.objects.colors.get(&refs.color_book) {
        return book.book.clone();
    }
    let (color, book) = round_trip_color(r, n)?;
    super::tables::color_book(color, book, None)
}

/// Extended data groups without the application `app`'s.
fn without_app(groups: Vec<RawTag>, app: &str) -> Vec<RawTag> {
    let mut keep = true;
    groups
        .into_iter()
        .filter(|g| {
            if g.code == 1001 {
                keep = !g.value.eq_ignore_ascii_case(app);
            }
            keep
        })
        .collect()
}

/// DraftCraft's own extended data on an entity, counted until the entity's type reads it.
const DRAFTCRAFT_DATA: &str = "DraftCraft extended data";

/// The types whose DraftCraft extended data this reader reads: a dimension's overrides,
/// associativity and marks, a hatch's origin, background and gradient, a viewport's frozen layers
/// and colours, a table's rows and cells, a region's loops (on the polyline it is written as).
/// DraftCraft writes it on no other entity.
fn reads_own_data(k: &EntityKind) -> bool {
    matches!(k, EntityKind::Dimension(_) | EntityKind::Hatch(_) | EntityKind::Viewport(_) | EntityKind::Table(_) | EntityKind::Region(_))
}

/// Other applications' extended data as the groups the drawing keeps with the entity, by the DXF
/// reader's rules: the applications that reader turns into properties are not kept as groups (and
/// are counted, as this reader doesn't read them yet), and an application whose list braces don't
/// balance is dropped.
fn xdata(blocks: &[XDataBlock], n: &Names, partial: &mut Vec<&'static str>) -> Vec<RawTag> {
    let real = |code: i32, v: f64| RawTag { code, value: draftcraft_dxf::Value::Real(v).as_str() };
    let mut out = Vec::new();
    for b in blocks {
        let Some(name) = n.tables.apps.get(&b.app) else {
            partial.push("extended data of an unknown application");
            continue;
        };
        if name.eq_ignore_ascii_case(crate::dxf_ext::APP) {
            partial.push(DRAFTCRAFT_DATA);
            continue;
        }
        if crate::dxf_raw::OWN_APPS.contains(&name.to_ascii_uppercase().as_str()) {
            partial.push("extended data read as properties");
            continue;
        }
        // A spline's method and knot spacing that an older version's file keeps for AutoCAD 2013
        // and later: AutoCAD's DXF of the file has none of it.
        if name.eq_ignore_ascii_case("ACAD_SPLINE_DATA") {
            continue;
        }
        let mut groups = vec![RawTag { code: 1001, value: name.clone() }];
        let (mut depth, mut ok) = (0i32, true);
        // AutoCAD's round-trip data are properties (`common`), not kept as groups.
        let items: Vec<&XDataItem> = if name.eq_ignore_ascii_case("ACAD") { round_trip(&b.items).0 } else { b.items.iter().collect() };
        if items.is_empty() {
            continue;
        }
        for v in items {
            let pt = |base: i32, p: &[f64; 3]| [real(base, p[0]), real(base + 10, p[1]), real(base + 20, p[2])];
            match v {
                XDataItem::Text(t) => groups.push(RawTag { code: 1000, value: t.clone() }),
                XDataItem::Open => {
                    depth += 1;
                    groups.push(RawTag { code: 1002, value: "{".into() });
                }
                XDataItem::Close => {
                    depth -= 1;
                    ok &= depth >= 0;
                    groups.push(RawTag { code: 1002, value: "}".into() });
                }
                XDataItem::Layer(h) => groups.push(RawTag { code: 1003, value: n.tables.layers.get(h).cloned().unwrap_or_else(|| "0".into()) }),
                XDataItem::Bytes(bytes) => groups.push(RawTag { code: 1004, value: bytes.iter().map(|x| format!("{x:02X}")).collect() }),
                XDataItem::Handle(h) => groups.push(RawTag { code: 1005, value: format!("{h:X}") }),
                XDataItem::Point(p) => groups.extend(pt(1010, p)),
                XDataItem::Position(p) => groups.extend(pt(1011, p)),
                XDataItem::Displacement(p) => groups.extend(pt(1012, p)),
                XDataItem::Direction(p) => groups.extend(pt(1013, p)),
                XDataItem::Real(x) => groups.push(real(1040, *x)),
                XDataItem::Distance(x) => groups.push(real(1041, *x)),
                XDataItem::ScaleFactor(x) => groups.push(real(1042, *x)),
                XDataItem::Int16(i) => groups.push(RawTag { code: 1070, value: i.to_string() }),
                XDataItem::Int32(i) => groups.push(RawTag { code: 1071, value: i.to_string() }),
                XDataItem::Undecoded(_) => {
                    partial.push("undecoded extended data");
                    ok = false;
                }
            }
        }
        if ok && depth == 0 && out.len() + groups.len() <= crate::dxf_raw::MAX_XDATA_GROUPS {
            out.extend(groups);
        }
    }
    out
}

/// The entity types read so far.
fn reads(type_name: &str) -> bool {
    matches!(
        type_name,
        "LINE"
            | "OLE2FRAME"
            | "HELIX"
            | "3DSOLID"
            | "REGION"
            | "BODY"
            | "POINT"
            | "CIRCLE"
            | "ARC"
            | "ELLIPSE"
            | "LWPOLYLINE"
            | "TEXT"
            | "MTEXT"
            | "INSERT"
            | "MINSERT"
            | "SOLID"
            | "TRACE"
            | "3DFACE"
            | "XLINE"
            | "RAY"
            | "ATTRIB"
            | "ATTDEF"
            | "POLYLINE_2D"
            | "POLYLINE_3D"
            | "POLYLINE_PFACE"
            | "POLYLINE_MESH"
            | "VERTEX_2D"
            | "VERTEX_3D"
            | "VERTEX_MESH"
            | "VERTEX_PFACE"
            | "VERTEX_PFACE_FACE"
            | "SPLINE"
            | "HATCH"
            | "LEADER"
            | "TOLERANCE"
            | "MULTILEADER"
            | "VIEWPORT"
            | "IMAGE"
            | "MLINE"
            | "ACAD_TABLE"
            | "WIPEOUT"
            | "PDFUNDERLAY"
            | "DIMENSION_ORDINATE"
            | "DIMENSION_LINEAR"
            | "DIMENSION_ALIGNED"
            | "DIMENSION_ANG_3PT"
            | "DIMENSION_ANG_2LN"
            | "DIMENSION_RADIUS"
            | "DIMENSION_DIAMETER"
    )
}

/// The fields of one entity's type, held until its references (a text style, a block) are read.
enum Body {
    Ready(Box<EntityKind>),
    /// An entity of a type the drawing keeps without modelling it: its DXF type and its own
    /// groups (from its `100` subclass marker on), as AutoCAD's DXF has them.
    Unknown(&'static str, Vec<RawTag>),
    /// A text, with its alignment and insertion points in its own coordinate system.
    Text(Box<Text>, Option<Vec3>, Vec3),
    MText(Box<MText>),
    /// A block reference, whether it has attributes, and how many it lists (2004 and later).
    Insert(Box<Insert>, bool, u32),
    /// An attribute (of a block reference) or an attribute definition, with its alignment and
    /// insertion points in its own coordinate system.
    /// With the handles a multiline attribute's embedded text has, when it has one.
    Attrib(Box<Attrib>, Option<Vec3>, Vec3, bool, Option<EmbeddedRefs>),
    /// A polyline, waiting for its vertices, and how many it lists (2004 and later).
    Polyline(PolyKind, u32),
    /// A vertex or a polyface face, waiting for its polyline.
    Vertex(Vertex),
    /// A dimension, waiting for its style and block, with its extension lines' angle (oblique)
    /// for linear and aligned ones.
    Dimension(Box<Dimension>, f64),
    /// A hatch, waiting for the boundary objects of its loops (how many each names) and for its
    /// extended data (origin, background, a gradient DraftCraft wrote there).
    Hatch(Box<Hatch>, Vec<u32>, Vec<RawPatternLine>),
    /// A leader, waiting for its dimension style.
    /// A leader, and what its hook line needs when its annotation is text: the hook direction
    /// flag and the annotation's x direction.
    Leader(Box<Leader>, Option<(bool, Vec2)>),
    /// A feature control frame, waiting for its dimension style.
    Tolerance(Box<Tolerance>),
    /// A multiline, waiting for its style.
    MLine(f64, i64, i64, Vec<crate::dxf_mline::RawVertex>),
    /// A raster image, waiting for its image definition and reactor.
    Raster(Box<Image>),
    /// A PDF underlay, waiting for its definition.
    Underlay(Box<Image>),
    /// A paper space viewport, waiting for its frozen layers (how many it names) and clipping
    /// boundary, with its status flags.
    Viewport(Box<Viewport>, u32, u32),
    /// A multileader's values, waiting for the objects its references name, in their order.
    MLeader(Box<crate::dxf_mleader::Parsed>, Vec<MlRef>),
}

/// MLINE (§20.4.50): scale, justification, base point, plane, flags, then per vertex its point,
/// segment direction, miter and per style element the distances along it (and the fill's,
/// which the drawing does not keep).
fn mline(r: &mut ObjectReader) -> DwgResult<Shape> {
    use draftcraft_doc::{MAX_MLINE_CUTS, MAX_MLINE_ELEMENTS, MAX_MLINE_VERTICES};
    let scale = r.bd()?;
    let just = i64::from(r.rc()?);
    let _base = r.bd3()?;
    let normal = v3(r.bd3()?);
    let flags = i64::from(r.bs()?);
    let elements = usize::from(r.rc()?);
    let n = u32::from(r.bs()?);
    let n = bounded(r, n, u32::try_from(MAX_MLINE_VERTICES).unwrap_or(u32::MAX), 18, "multiline vertex")?;
    let mut vertices = Vec::with_capacity(n);
    for _ in 0..n {
        let mut v = crate::dxf_mline::RawVertex { p: v3(r.bd3()?), dir: v3(r.bd3()?), miter: v3(r.bd3()?), params: Vec::new() };
        for _ in 0..elements {
            let k = u32::from(r.bs()?);
            let k = bounded(r, k, u32::try_from(2 * MAX_MLINE_CUTS + 2).unwrap_or(u32::MAX), 2, "multiline segment parameter")?;
            let mut list = Vec::with_capacity(k);
            for _ in 0..k {
                let x = r.bd()?;
                list.push(if x.is_finite() { x } else { 0.0 });
            }
            let k = u32::from(r.bs()?);
            for _ in 0..bounded(r, k, 1 << 16, 2, "multiline fill parameter")? {
                let _fill = r.bd()?;
            }
            if v.params.len() < MAX_MLINE_ELEMENTS {
                v.params.push(list);
            }
        }
        vertices.push(v);
    }
    let scale = if scale.is_finite() { scale } else { 1.0 };
    Ok(Shape { body: Body::MLine(scale, just, flags, vertices), thickness: 0.0, normal })
}

/// Most clip boundary vertices of an image, wipeout or underlay that are read.
const MAX_CLIP: u32 = 100_000;

/// IMAGE and WIPEOUT (AcDbRasterImage, §20.4.80): placement, size in pixels, display settings
/// and clip boundary. A wipeout's boundary becomes its outline in the drawing.
fn raster(r: &mut ObjectReader, wipeout: bool) -> DwgResult<Shape> {
    let _class_version = r.bl()?;
    let insert = v3(r.bd3()?);
    let u = v3(r.bd3()?);
    let v = v3(r.bd3()?);
    let [w, h] = r.rd2()?;
    let display = r.bs()?;
    let clipping = r.b()?;
    let pct = |v: u8| v.min(100);
    let (brightness, contrast, fade) = (pct(r.rc()?), pct(r.rc()?), pct(r.rc()?));
    if r.version() >= V::R2010 {
        let _clip_inside = r.b()?;
    }
    let clip_type = r.bs()?;
    let mut clip = Vec::new();
    if clip_type == 1 {
        for _ in 0..2 {
            let [x, y] = r.rd2()?;
            clip.push(Vec2::new(x, y));
        }
    } else {
        let n = r.bl()?;
        for _ in 0..bounded(r, n, MAX_CLIP, 128, "clip vertex")? {
            let [x, y] = r.rd2()?;
            clip.push(Vec2::new(x, y));
        }
    }
    let normal = UP;
    if wipeout {
        // Clip vertices are in pixel space: origin at the image's top-left corner, y pointing down.
        let boundary = clip.into_iter().map(|q| insert.xy() + u.xy() * (q.x + 0.5) + v.xy() * (0.5 - q.y)).collect();
        return Ok(ready(EntityKind::Wipeout(Wipeout { boundary }), 0.0, normal));
    }
    let def = Image::default();
    let image = Image { insert, u, v, size: Vec2::new(w, h), clip, clipping, display, brightness, contrast, fade, ..def };
    Ok(Shape { body: Body::Raster(Box::new(image)), thickness: 0.0, normal })
}

/// PDFUNDERLAY (an underlay reference): plane, insertion point, rotation, scales, flags,
/// contrast, fade and clip boundary. The definition comes first among its references.
fn underlay(r: &mut ObjectReader) -> DwgResult<Shape> {
    let normal = v3(r.bd3()?);
    let insert = v3(r.bd3()?);
    let rotation = r.bd()?;
    let (sx, sy, _sz) = (r.bd()?, r.bd()?, r.bd()?);
    let flags = r.rc()?;
    let pct = |v: u8| v.min(100);
    let (contrast, fade) = (pct(r.rc()?), pct(r.rc()?));
    let n = r.bl()?;
    let mut clip = Vec::new();
    for _ in 0..bounded(r, n, MAX_CLIP, 128, "underlay clip vertex")? {
        let [x, y] = r.rd2()?;
        clip.push(Vec2::new(x, y));
    }
    let image = crate::dxf_underlay::reference(insert, (Some(sx), Some(sy)), rotation, i64::from(flags), (contrast, fade), clip);
    Ok(Shape { body: Body::Underlay(Box::new(image)), thickness: 0.0, normal })
}

/// VIEWPORT (§20.4.38), AutoCAD 2000 and later: the sheet rectangle, the view it shows (its
/// centre is in display coordinates around the view target), the twist and the status flags.
/// Before 2000 the view is in the ACAD extended data: such a viewport is left unread.
fn viewport(r: &mut ObjectReader) -> DwgResult<Option<Shape>> {
    let ver = r.version();
    let center = v3(r.bd3()?);
    let width = r.bd()?;
    let height = r.bd()?;
    if ver < V::R2000 {
        return Ok(None);
    }
    let [tx, ty, _] = r.bd3()?;
    let _direction = r.bd3()?;
    let twist = r.bd()?;
    let view_height = r.bd()?;
    let _lens = r.bd()?;
    let _clip = (r.bd()?, r.bd()?);
    let _snap_angle = r.bd()?;
    let [cx, cy] = r.rd2()?;
    let _snap = (r.rd2()?, r.rd2()?);
    let _grid = r.rd2()?;
    let _circle_zoom = r.bs()?;
    if ver >= V::R2007 {
        let _grid_major = r.bs()?;
    }
    let frozen = r.bl()?;
    let status = r.bl()?;
    let _style_sheet = r.text()?;
    let _render_mode = r.rc()?;
    let _ucs = (r.b()?, r.b()?, r.bd3()?, r.bd3()?, r.bd3()?, r.bd()?, r.bs()?);
    if ver >= V::R2004 {
        let _shade_plot = r.bs()?;
    }
    if ver >= V::R2007 {
        let _lights = (r.b()?, r.rc()?, r.bd()?, r.bd()?);
        let _ambient = r.color()?;
    }
    let twist = if twist.is_finite() { draftcraft_geom::norm_angle(twist) } else { 0.0 };
    let v = Viewport {
        center,
        width,
        height,
        view_center: crate::dxf_ext::dcs_center_to_model(Vec2::new(cx, cy), twist, Vec2::new(tx, ty)),
        view_height,
        // Numbered in its layout once the layouts are read (DWG keeps no number).
        id: 0,
        locked: status & 16384 != 0,
        frozen_layers: Vec::new(),
        layer_colors: Vec::new(),
        clip: None,
        on: status & 131_072 == 0,
        twist,
    };
    Ok(Some(Shape { body: Body::Viewport(Box::new(v), frozen, status), thickness: 0.0, normal: UP }))
}

/// What one of a multileader's references names, in the order the file holds them.
#[derive(Clone, Copy)]
enum MlRef {
    /// Not kept in the drawing (a line type, an arrowhead).
    Skip,
    Style,
    TextStyle,
    ContextTextStyle,
    Block,
    ContextBlock,
    /// The attribute definition of block attribute value `n`.
    AttDef(usize),
}

/// Most leader roots, lines, points, breaks and attribute values of one multileader that are
/// read: a file with more is not a multileader DraftCraft opens.
const MAX_MLEADER_ITEMS: u32 = 100_000;

/// A count read from the file, refused above `max` or above what the record can hold.
fn bounded(r: &mut ObjectReader, n: u32, max: u32, min_bits: usize, what: &str) -> DwgResult<usize> {
    if n > max {
        return Err(draftcraft_dwg::records::Error::format(format!("too many {what}s: {n}")));
    }
    r.count(n, min_bits, what)
}

/// LEADER (§20.4.47): its vertices, arrowhead and path type. The annotation and the hook line are
/// drawn from the dimension style, as the DXF reader has them.
fn leader(r: &mut ObjectReader) -> DwgResult<Shape> {
    let ver = r.version();
    let _unknown = r.b()?;
    let annotation_type = r.bs()?;
    let path_type = r.bs()?;
    let n = r.bl()?;
    let n = bounded(r, n, MAX_MLEADER_ITEMS, 6, "leader vertex")?;
    let mut vertices = Vec::with_capacity(n);
    for _ in 0..n {
        vertices.push(v3(r.bd3()?));
    }
    let _origin = r.bd3()?;
    let normal = v3(r.bd3()?);
    let x_direction = r.bd3()?;
    let _block_offset = r.bd3()?;
    if ver >= V::R14 {
        let _annotation_offset = r.bd3()?;
    }
    if ver <= V::R14 {
        let _dimgap = r.bd()?;
    }
    // The annotation's height and width, up to AutoCAD 2007: from 2010 on the record has
    // neither (as acadrust reads and writes it; not yet seen in a file AutoCAD saved).
    if ver <= V::R2007 {
        let _text_height = r.bd()?;
        let _text_width = r.bd()?;
    }
    let hook_on_x = r.b()?;
    let arrow = r.b()?;
    if ver <= V::R14 {
        let _arrowhead_type = r.bs()?;
        let _dimasz = r.bd()?;
        let _ = (r.b()?, r.b()?, r.bs()?, r.bs()?, r.b()?, r.b()?);
    } else {
        let _ = (r.bs()?, r.b()?, r.b()?);
    }
    let l = Leader { vertices, arrow, spline: path_type == 1, style: "Standard".into() };
    // A straight leader to text (annotation type 0) may end in a hook line ([`add_hook_lines`]).
    let x = Vec2::new(x_direction[0], x_direction[1]);
    let hook = (annotation_type == 0 && path_type == 0 && x.is_finite() && x.len() > 1e-12).then(|| (hook_on_x, x.normalized()));
    Ok(Shape { body: Body::Leader(Box::new(l), hook), thickness: 0.0, normal })
}

/// TOLERANCE (§20.4.49): the frame's insertion point, direction and contents. AutoCAD R13 and
/// R14 store a short, the text height and the gap first.
fn tolerance(r: &mut ObjectReader) -> DwgResult<Shape> {
    if r.version() <= V::R14 {
        let _ = (r.bs()?, r.bd()?, r.bd()?);
    }
    let insert = v3(r.bd3()?);
    let dir = v3(r.bd3()?);
    let normal = v3(r.bd3()?);
    // A line break of the frame is `^J` in the drawing, as in AutoCAD's DXF; the DWG stores the
    // character itself.
    let text = frame_text(&r.text()?);
    let dir = if dir.xy().is_finite() && dir.xy().len() > 1e-12 { dir } else { Vec3::new(1.0, 0.0, 0.0) };
    let t = Tolerance { insert, dir, style: "Standard".into(), text };
    Ok(Shape { body: Body::Tolerance(Box::new(t)), thickness: 0.0, normal })
}

/// MULTILEADER (§20.4.87, AutoCAD 2007 and later): the annotation context (the leader roots and
/// their lines, the text or block content and the plane), then the entity's own values. The
/// values go where the DXF reader puts the same groups, and the drawing's multileader is built
/// from them by its rules.
fn mleader(r: &mut ObjectReader) -> DwgResult<Shape> {
    use crate::dxf_mleader::{P, Parsed, Root};
    let ver = r.version();
    let finite = |v: f64| v.is_finite().then_some(v);
    let mut p = Parsed::default();
    let mut refs = Vec::new();
    if ver >= V::R2010 {
        let _class_version = r.bs()?;
    }
    let n_roots = r.bl()?;
    for _ in 0..bounded(r, n_roots, 256, 8, "leader root")? {
        let _valid = r.b()?;
        let _unknown = r.b()?;
        let connection = v3(r.bd3()?);
        let _direction = r.bd3()?;
        let n_breaks = r.bl()?;
        for _ in 0..bounded(r, n_breaks, MAX_MLEADER_ITEMS, 12, "leader break")? {
            let _ = (r.bd3()?, r.bd3()?);
        }
        let _index = r.bl()?;
        let landing_distance = r.bd()?;
        let n_lines = r.bl()?;
        let mut lines = Vec::new();
        for _ in 0..bounded(r, n_lines, MAX_MLEADER_ITEMS, 6, "leader line")? {
            let n_points = r.bl()?;
            let n_points = bounded(r, n_points, MAX_MLEADER_ITEMS, 6, "leader line point")?;
            let mut points = Vec::with_capacity(n_points);
            for _ in 0..n_points {
                points.push(v3(r.bd3()?));
            }
            let n_info = r.bl()?;
            for _ in 0..bounded(r, n_info, MAX_MLEADER_ITEMS, 4, "leader line break")? {
                let _segment = r.bl()?;
                let n = r.bl()?;
                for _ in 0..bounded(r, n, MAX_MLEADER_ITEMS, 12, "leader line break point")? {
                    let _ = (r.bd3()?, r.bd3()?);
                }
            }
            let _index = r.bl()?;
            if ver >= V::R2010 {
                let _path_type = r.bs()?;
                let _color = r.color()?;
                refs.push(MlRef::Skip);
                let _lineweight = r.bl()?;
                let _arrow_size = r.bd()?;
                refs.push(MlRef::Skip);
                let _overrides = r.bl()?;
            }
            lines.push(points);
        }
        if ver >= V::R2010 {
            let _attachment_direction = r.bs()?;
        }
        p.roots.push(Root { landing: P::at(connection), dogleg: finite(landing_distance), lines });
    }
    let _scale = r.bd()?;
    p.base = P::at(v3(r.bd3()?));
    p.text_height = finite(r.bd()?);
    p.arrow_ctx = finite(r.bd()?);
    let _landing_gap = r.bd()?;
    p.left_attach = Some(i64::from(r.bs()?));
    p.right_attach = Some(i64::from(r.bs()?));
    let _text_alignment = r.bs()?;
    let _block_connection = r.bs()?;
    let has_text = r.b()?;
    p.has_text = Some(has_text);
    if has_text {
        p.text = Some(r.text()?);
        let _normal = r.bd3()?;
        refs.push(MlRef::ContextTextStyle);
        p.text_location = P::at(v3(r.bd3()?));
        p.text_direction = P::at(v3(r.bd3()?));
        p.text_rotation = finite(r.bd()?);
        p.text_width = finite(r.bd()?);
        let _boundary_height = r.bd()?;
        p.line_spacing = finite(r.bd()?);
        p.line_spacing_style = Some(i64::from(r.bs()?));
        let _color = r.color()?;
        p.side = Some(i64::from(r.bs()?));
        let _flow = r.bs()?;
        let _background_color = r.color()?;
        let _background_scale = r.bd()?;
        let _background_transparency = r.bl()?;
        let _ = (r.b()?, r.b()?);
        let _column_type = r.bs()?;
        let _auto_height = r.b()?;
        let _ = (r.bd()?, r.bd()?);
        let _flow_reversed = r.b()?;
        let n = r.bl()?;
        for _ in 0..bounded(r, n, MAX_MLEADER_ITEMS, 2, "multileader column")? {
            let _size = r.bd()?;
        }
        let _ = (r.b()?, r.b()?);
    } else {
        let has_block = r.b()?;
        p.has_block = Some(has_block);
        if has_block {
            refs.push(MlRef::ContextBlock);
            let _normal = r.bd3()?;
            p.block_location = P::at(v3(r.bd3()?));
            p.block_scale_ctx = P::at(v3(r.bd3()?));
            p.block_rotation_ctx = finite(r.bd()?);
            let _color = r.color()?;
            for _ in 0..16 {
                let _matrix = r.bd()?;
            }
        }
    }
    let _ = (r.bd3()?, r.bd3()?, r.bd3()?, r.b()?);
    if ver >= V::R2010 {
        let _ = (r.bs()?, r.bs()?);
    }
    // The entity's own values.
    refs.push(MlRef::Style);
    let _overrides = r.bl()?;
    p.leader_type = Some(i64::from(r.bs()?));
    let _line_color = r.color()?;
    refs.push(MlRef::Skip);
    let _lineweight = r.bl()?;
    let _landing = r.b()?;
    p.dogleg_on = Some(r.b()?);
    p.dogleg = finite(r.bd()?);
    refs.push(MlRef::Skip);
    p.arrow = finite(r.bd()?);
    p.content = Some(i64::from(r.bs()?));
    refs.push(MlRef::TextStyle);
    let _ = (r.bs()?, r.bs()?, r.bs()?, r.bs()?);
    let _text_color = r.color()?;
    let _frame = r.b()?;
    refs.push(MlRef::Block);
    let _block_color = r.color()?;
    p.block_scale = P::at(v3(r.bd3()?));
    p.block_rotation = finite(r.bd()?);
    let _connection = r.bs()?;
    let _annotative = r.b()?;
    if ver < V::R2010 {
        let n = r.bl()?;
        for _ in 0..bounded(r, n, MAX_MLEADER_ITEMS, 1, "multileader arrowhead")? {
            let _default = r.b()?;
            refs.push(MlRef::Skip);
        }
    }
    let n = r.bl()?;
    for i in 0..bounded(r, n, MAX_MLEADER_ITEMS, 4, "multileader block attribute")? {
        refs.push(MlRef::AttDef(i));
        p.attribs.push((String::new(), r.text()?));
        let _ = (r.bs()?, r.bd()?);
    }
    let _ = (r.b()?, r.bs()?, r.bs()?, r.bd()?);
    if ver >= V::R2010 {
        let _ = (r.bs()?, r.bs()?, r.bs()?);
    }
    if ver >= V::R2013 {
        let _extend_to_text = r.b()?;
    }
    Ok(Shape { body: Body::MLeader(Box::new(p), refs), thickness: 0.0, normal: UP })
}

/// A pattern definition line as the file holds it: angle in degrees, base point, offset, dashes.
type RawPatternLine = (f64, Vec2, Vec2, Vec<f64>);

/// Most loops, edges, vertices, pattern lines, dashes or seed points of one hatch that are read.
const MAX_HATCH_ITEMS: u32 = 1 << 20;

/// HATCH (§20.4.57): the gradient (AutoCAD 2004 and later), the boundary loops, the pattern and
/// the seed points. Edges become the drawing's loop vertices by the DXF reader's rules.
fn hatch(r: &mut ObjectReader) -> DwgResult<Shape> {
    let ver = r.version();
    let mut gradient = None;
    if ver >= V::R2004 {
        let is_gradient = r.bl()? != 0;
        let _reserved = r.bl()?;
        let angle = r.bd()?;
        let shift = r.bd()?;
        let _single_color = r.bl()?;
        let _tint = r.bd()?;
        let n = r.bl()?;
        let n = r.count(n, 14, "gradient colour")?;
        let mut colors = Vec::with_capacity(n);
        for _ in 0..n {
            let _value = r.bd()?;
            let _unknown = r.bs()?;
            let rgb = r.bl()?;
            let _ignored = r.rc()?;
            colors.push(rgb);
        }
        let name = r.text()?;
        if is_gradient {
            // A colour word: its top byte says how (0xC0 ByLayer, 0xC1 ByBlock, 0xC3 a colour
            // index in the low byte, 0xC8 none), otherwise the low bytes are RGB.
            let color = |w: Option<&u32>, d: draftcraft_color::Color| {
                w.map_or(d, |&w| match w >> 24 {
                    0xC0 => draftcraft_color::Color::ByLayer,
                    0xC1 => draftcraft_color::Color::ByBlock,
                    0xC3 => draftcraft_color::Color::from_aci(i16::from(u8::try_from(w & 0xff).unwrap_or(7))),
                    0xC8 => d,
                    _ => draftcraft_color::Color::True(draftcraft_color::Rgb::from_u32(w & 0xff_ffff)),
                })
            };
            let d = crate::dxf_read::default_gradient();
            gradient = Some(Gradient {
                name: if name.is_empty() { d.name } else { name },
                color1: color(colors.first(), d.color1),
                color2: color(colors.get(1), d.color2),
                angle: if angle.is_finite() { angle } else { 0.0 },
                centered: shift.abs() < 1e-9,
            });
        }
    }
    let elevation = r.bd()?;
    let normal = v3(r.bd3()?);
    let pattern = r.text()?;
    let solid_fill = r.b()?;
    let associative = r.b()?;
    let n_loops = r.bl()?;
    let n_loops = r.count(n_loops.min(MAX_HATCH_ITEMS), 6, "hatch loop")?;
    let mut loops = Vec::with_capacity(n_loops);
    let mut sources = Vec::with_capacity(n_loops);
    let mut derived = false;
    for _ in 0..n_loops {
        let flags = r.bl()?;
        derived |= flags & 4 != 0;
        let mut vertices = Vec::new();
        if flags & 2 != 0 {
            // A polyline loop: vertices with or without bulges.
            let has_bulges = r.b()?;
            let _closed = r.b()?;
            let n = r.bl()?;
            let n = r.count(n.min(MAX_HATCH_ITEMS), 128, "hatch polyline vertex")?;
            for _ in 0..n {
                let [x, y] = r.rd2()?;
                let bulge = if has_bulges { r.bd()? } else { 0.0 };
                vertices.push(PolyVertex::with_bulge(Vec2::new(x, y), bulge));
            }
        } else {
            let n = r.bl()?;
            let n = r.count(n.min(MAX_HATCH_ITEMS), 8, "hatch edge")?;
            for _ in 0..n {
                hatch_edge(r, &mut vertices)?;
            }
        }
        crate::dxf_read::close_loop(&mut vertices);
        loops.push(HatchLoop { vertices, outer: flags & 1 != 0, sources: Vec::new() });
        sources.push(r.bl()?);
    }
    let style = r.bs()?;
    let _pattern_type = r.bs()?;
    let (mut angle, mut scale, mut raw) = (0.0, 1.0, Vec::new());
    if !solid_fill {
        angle = r.bd()?;
        scale = r.bd()?;
        let _double = r.b()?;
        let n = u32::from(r.bs()?);
        let n = r.count(n, 10, "hatch pattern line")?;
        for _ in 0..n {
            let line_angle = r.bd()?;
            let [bx, by] = r.bd2()?;
            let [ox, oy] = r.bd2()?;
            let n_dashes = u32::from(r.bs()?);
            let n_dashes = r.count(n_dashes, 2, "hatch dash")?;
            let mut dashes = Vec::with_capacity(n_dashes);
            for _ in 0..n_dashes {
                dashes.push(r.bd()?);
            }
            raw.push((line_angle.to_degrees(), Vec2::new(bx, by), Vec2::new(ox, oy), dashes));
        }
    }
    if derived {
        let _pixel_size = r.bd()?;
    }
    let n_seeds = r.bl()?;
    for _ in 0..r.count(n_seeds.min(MAX_HATCH_ITEMS), 128, "hatch seed point")? {
        let _seed = r.rd2()?;
    }
    let solid = solid_fill || pattern.eq_ignore_ascii_case("SOLID");
    if solid {
        raw.clear();
    }
    let h = Hatch {
        pattern: if pattern.is_empty() { "SOLID".into() } else { pattern.to_ascii_uppercase() },
        solid,
        loops,
        scale: if scale.is_finite() { scale } else { 1.0 },
        angle: if angle.is_finite() { angle } else { 0.0 },
        associative,
        style: u8::try_from(style).unwrap_or(0),
        elevation: if elevation.is_finite() { elevation } else { 0.0 },
        gradient,
        origin: Vec2::ZERO,
        background: None,
        // Made once the origin is known: it is in the extended data.
        pattern_lines: Vec::new(),
    };
    Ok(Shape { body: Body::Hatch(Box::new(h), sources, raw), thickness: 0.0, normal })
}

/// One edge of a hatch's boundary loop, onto its vertices.
fn hatch_edge(r: &mut ObjectReader, vertices: &mut Vec<PolyVertex>) -> DwgResult<()> {
    let p2 = |r: &mut ObjectReader| -> DwgResult<Vec2> {
        let [x, y] = r.rd2()?;
        Ok(Vec2::new(x, y))
    };
    match r.rc()? {
        1 => {
            let (a, b) = (p2(r)?, p2(r)?);
            crate::dxf_read::push_line(vertices, a, b);
        }
        2 => {
            let center = p2(r)?;
            let (radius, start, end, ccw) = (r.bd()?, r.bd()?, r.bd()?, r.b()?);
            crate::dxf_read::push_arc(vertices, center, radius, start, end, ccw);
        }
        3 => {
            let (center, major) = (p2(r)?, p2(r)?);
            let (ratio, start, end, ccw) = (r.bd()?, r.bd()?, r.bd()?, r.b()?);
            crate::dxf_read::push_ellipse(vertices, center, major, ratio, start, end, ccw);
        }
        4 => {
            let degree = usize::try_from(r.bl()?).unwrap_or(3);
            let rational = r.b()?;
            let _periodic = r.b()?;
            let n_knots = r.bl()?;
            let n_control = r.bl()?;
            let n_knots = r.count(n_knots.min(MAX_HATCH_ITEMS), 2, "hatch spline knot")?;
            let n_control = r.count(n_control.min(MAX_HATCH_ITEMS), 128, "hatch spline control point")?;
            let mut knots = Vec::with_capacity(n_knots);
            for _ in 0..n_knots {
                knots.push(r.bd()?);
            }
            let (mut control, mut weights) = (Vec::with_capacity(n_control), Vec::new());
            for _ in 0..n_control {
                control.push(p2(r)?);
                if rational {
                    weights.push(r.bd()?);
                }
            }
            let mut fit = Vec::new();
            if r.version() >= V::R2010 {
                let n_fit = r.bl()?;
                let n_fit = r.count(n_fit.min(MAX_HATCH_ITEMS), 128, "hatch spline fit point")?;
                for _ in 0..n_fit {
                    fit.push(p2(r)?);
                }
                if n_fit > 0 {
                    let _start_tangent = p2(r)?;
                    let _end_tangent = p2(r)?;
                }
            }
            crate::dxf_read::push_spline(vertices, degree, knots, control, weights, &fit);
        }
        other => return Err(draftcraft_dwg::records::Error::format(format!("unknown hatch edge type {other}"))),
    }
    Ok(())
}

/// DIMENSION (§20.4.20 to §20.4.26): the fields every dimension has, then its kind's points.
fn dimension(r: &mut ObjectReader, type_name: &str) -> DwgResult<Shape> {
    let ver = r.version();
    if ver >= V::R2010 {
        let _version = r.rc()?;
    }
    let normal = v3(r.bd3()?);
    let [mx, my] = r.rd2()?;
    let elevation = r.bd()?;
    // Bit 0 clear: the text was placed by the user (DXF group 70, 128).
    let flags = r.rc()?;
    let text = r.text()?;
    let text_rotation = r.bd()?;
    let _horizontal_direction = r.bd()?;
    let _insert_scale = r.bd3()?;
    let _insert_rotation = r.bd()?;
    let mut measurement = 0.0;
    if ver >= V::R2000 {
        let _attachment = r.bs()?;
        let _line_spacing_style = r.bs()?;
        let _line_spacing = r.bd()?;
        measurement = r.bd()?;
    }
    if ver >= V::R2007 {
        let _unknown = r.b()?;
        let _flip_arrow_1 = r.b()?;
        let _flip_arrow_2 = r.b()?;
    }
    let [ix, iy] = r.rd2()?;
    let p = |r: &mut ObjectReader| -> DwgResult<Vec3> { Ok(v3(r.bd3()?)) };
    let defpt;
    let (mut p13, mut p14, mut p15, mut p16) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
    let mut oblique = 0.0;
    let kind = match type_name {
        "DIMENSION_ORDINATE" => {
            defpt = p(r)?;
            p13 = p(r)?;
            p14 = p(r)?;
            DimKind::Ordinate { x_type: r.rc()? & 1 != 0 }
        }
        "DIMENSION_LINEAR" | "DIMENSION_ALIGNED" => {
            p13 = p(r)?;
            p14 = p(r)?;
            defpt = p(r)?;
            oblique = r.bd()?;
            if type_name == "DIMENSION_LINEAR" { DimKind::Linear { rotation: r.bd()? } } else { DimKind::Aligned }
        }
        "DIMENSION_ANG_3PT" => {
            defpt = p(r)?;
            p13 = p(r)?;
            p14 = p(r)?;
            p15 = p(r)?;
            DimKind::Angular3P
        }
        "DIMENSION_ANG_2LN" => {
            let [ax, ay] = r.rd2()?;
            p16 = Vec3::new(ax, ay, elevation);
            p13 = p(r)?;
            p14 = p(r)?;
            p15 = p(r)?;
            defpt = p(r)?;
            DimKind::Angular
        }
        "DIMENSION_RADIUS" => {
            // The centre (10), then the point on the arc (15).
            defpt = p(r)?;
            p15 = p(r)?;
            let _leader_length = r.bd()?;
            DimKind::Radius
        }
        _ => {
            // DIMENSION_DIAMETER: the far end of the chord (15) first, then its near end (10).
            p15 = p(r)?;
            defpt = p(r)?;
            let _leader_length = r.bd()?;
            DimKind::Diameter
        }
    };
    // The text midpoint is the one point of a dimension in its object coordinate system.
    let mid = Vec3::new(mx, my, elevation);
    let dim = Dimension {
        kind,
        defpt,
        text_mid: if normal.z < 0.0 { ocs_n(normal, mid) } else { mid },
        p13,
        p14,
        p15,
        p16,
        text,
        style: "Standard".into(),
        measurement,
        text_rotation,
        user_text_pos: flags & 1 == 0,
        block: None,
        insertion_point: Vec3::new(ix, iy, 0.0),
        overrides: Default::default(),
        assoc: Vec::new(),
        marks: Default::default(),
    };
    Ok(Shape { body: Body::Dimension(Box::new(dim), oblique), thickness: 0.0, normal })
}

/// An object's extended data as the DXF groups it would be in a DXF file, for the DXF reader's
/// rules that read properties from them (a dimension's overrides, DraftCraft's own data).
/// References are written as their handles in hexadecimal, as a DXF holds them.
pub(super) fn xdata_tags(blocks: &[XDataBlock], apps: &HashMap<u64, String>) -> Vec<draftcraft_dxf::Tag> {
    use draftcraft_dxf::Tag;
    let mut out = Vec::new();
    for b in blocks {
        let Some(name) = apps.get(&b.app) else { continue };
        out.push(Tag::s(1001, name.clone()));
        for v in &b.items {
            let pt = |out: &mut Vec<Tag>, base: i32, p: &[f64; 3]| {
                out.extend([Tag::f(base, p[0]), Tag::f(base + 10, p[1]), Tag::f(base + 20, p[2])]);
            };
            match v {
                XDataItem::Text(t) => out.push(Tag::s(1000, t.clone())),
                XDataItem::Open => out.push(Tag::s(1002, "{")),
                XDataItem::Close => out.push(Tag::s(1002, "}")),
                XDataItem::Layer(h) => out.push(Tag::s(1003, format!("{h:X}"))),
                XDataItem::Bytes(bytes) => out.push(Tag::s(1004, bytes.iter().map(|x| format!("{x:02X}")).collect::<String>())),
                XDataItem::Handle(h) => out.push(Tag::s(1005, format!("{h:X}"))),
                XDataItem::Point(p) => pt(&mut out, 1010, p),
                XDataItem::Position(p) => pt(&mut out, 1011, p),
                XDataItem::Displacement(p) => pt(&mut out, 1012, p),
                XDataItem::Direction(p) => pt(&mut out, 1013, p),
                XDataItem::Real(x) => out.push(Tag::f(1040, *x)),
                XDataItem::Distance(x) => out.push(Tag::f(1041, *x)),
                XDataItem::ScaleFactor(x) => out.push(Tag::f(1042, *x)),
                XDataItem::Int16(i) => out.push(Tag::i(1070, i64::from(*i))),
                XDataItem::Int32(i) => out.push(Tag::i(1071, i64::from(*i))),
                XDataItem::Undecoded(_) => {}
            }
        }
    }
    out
}

/// The kind of a polyline, with what its own record holds.
#[derive(Clone, Copy, Debug)]
enum PolyKind {
    /// DXF flags, default start and end width, elevation.
    TwoD {
        flags: u16,
        start_width: f64,
        end_width: f64,
        elevation: f64,
    },
    ThreeD {
        closed: bool,
    },
    Pface,
    /// DXF flags, M and N, and the smoothed surface's M and N.
    Mesh {
        flags: u16,
        size: (u16, u16),
        smooth: (u16, u16),
    },
}

/// A vertex of a polyline, or a face of a polyface mesh.
#[derive(Clone, Copy, Debug, Default)]
struct Vertex {
    flags: u8,
    point: Vec3,
    start_width: f64,
    end_width: f64,
    bulge: f64,
    face: Option<[i16; 4]>,
}

/// The objects a polyline or a block reference owns: listed (2004 and later) or its first and
/// last (up to 2000), and the SEQEND that ends them.
fn owned_list(r: &mut ObjectReader, count: u32) -> DwgResult<Attribs> {
    let (mut listed, mut chain) = (Vec::new(), None);
    if r.version() <= V::R2000 {
        chain = Some((r.reference()?, r.reference()?));
    } else {
        // The handles are in their own stream: each read fails at its end, so the count only
        // needs a bound.
        for _ in 0..usize::try_from(count).unwrap_or(MAX_ENTITIES).min(MAX_ENTITIES) {
            listed.push(r.reference()?);
        }
    }
    Ok(Attribs { listed, chain, seqend: r.reference()? })
}

/// POLYLINE_2D (§20.4.15).
fn polyline_2d(r: &mut ObjectReader) -> DwgResult<Shape> {
    let flags = r.bs()?;
    let _curve_type = r.bs()?;
    let start_width = r.bd()?;
    let end_width = r.bd()?;
    let thickness = r.thickness()?;
    let elevation = r.bd()?;
    let normal = v3(r.extrusion()?);
    let owned = if r.version() >= V::R2004 { r.bl()? } else { 0 };
    Ok(Shape { body: Body::Polyline(PolyKind::TwoD { flags, start_width, end_width, elevation }, owned), thickness, normal })
}

/// POLYLINE_3D (§20.4.16).
fn polyline_3d(r: &mut ObjectReader) -> DwgResult<Shape> {
    let _curve_flags = r.rc()?;
    let closed = r.rc()? & 1 != 0;
    let owned = if r.version() >= V::R2004 { r.bl()? } else { 0 };
    Ok(Shape { body: Body::Polyline(PolyKind::ThreeD { closed }, owned), thickness: 0.0, normal: UP })
}

/// POLYLINE_PFACE (§20.4.29).
fn polyline_pface(r: &mut ObjectReader) -> DwgResult<Shape> {
    let _vertices = r.bs()?;
    let _faces = r.bs()?;
    let owned = if r.version() >= V::R2004 { r.bl()? } else { 0 };
    Ok(Shape { body: Body::Polyline(PolyKind::Pface, owned), thickness: 0.0, normal: UP })
}

/// POLYLINE_MESH (§20.4.30).
fn polyline_mesh(r: &mut ObjectReader) -> DwgResult<Shape> {
    let flags = r.bs()?;
    let _curve_type = r.bs()?;
    let size = (r.bs()?, r.bs()?);
    let smooth = (r.bs()?, r.bs()?);
    let owned = if r.version() >= V::R2004 { r.bl()? } else { 0 };
    Ok(Shape { body: Body::Polyline(PolyKind::Mesh { flags, size, smooth }, owned), thickness: 0.0, normal: UP })
}

/// VERTEX_2D (§20.4.10): a negative start width means both widths are its size.
fn vertex_2d(r: &mut ObjectReader) -> DwgResult<Shape> {
    let flags = r.rc()?;
    let point = v3(r.bd3()?);
    let sw = r.bd()?;
    let (start_width, end_width) = if sw < 0.0 { (-sw, -sw) } else { (sw, r.bd()?) };
    let bulge = r.bd()?;
    if r.version() >= V::R2010 {
        let _id = r.bl()?;
    }
    let _tangent = r.bd()?;
    let v = Vertex { flags, point, start_width, end_width, bulge, face: None };
    Ok(Shape { body: Body::Vertex(v), thickness: 0.0, normal: UP })
}

/// VERTEX_3D, VERTEX_MESH and VERTEX_PFACE (§20.4.11 to §20.4.13): flags and a point.
fn vertex_3d(r: &mut ObjectReader) -> DwgResult<Shape> {
    let flags = r.rc()?;
    let point = v3(r.bd3()?);
    Ok(Shape { body: Body::Vertex(Vertex { flags, point, ..Vertex::default() }), thickness: 0.0, normal: UP })
}

/// VERTEX_PFACE_FACE (§20.4.14): the four vertex numbers of a face, negative for a hidden edge.
fn pface_face(r: &mut ObjectReader) -> DwgResult<Shape> {
    let mut face = [0i16; 4];
    for f in &mut face {
        *f = i16::from_le_bytes(r.bs()?.to_le_bytes());
    }
    Ok(Shape { body: Body::Vertex(Vertex { flags: 128, face: Some(face), ..Vertex::default() }), thickness: 0.0, normal: UP })
}

/// SPLINE (§20.4.44): a spline defined by its fit points or by its control points and knots. From
/// AutoCAD 2013 on, flags and the knot parameterisation say which.
fn spline(r: &mut ObjectReader) -> DwgResult<Shape> {
    spline_with(r, &mut None)
}

/// HELIX (§20.4.?): a spline's data, then the helix: its version, the axis's base point, the
/// start point, the axis's direction, the radius, the turns, the height of a turn, whether it
/// turns counter-clockwise and what stays fixed when it is edited. The drawing's helix by the
/// DXF reader's rule, from the same groups (the spline gives the top's radius by its last point).
fn helix(r: &mut ObjectReader) -> DwgResult<Shape> {
    let mut last = None;
    spline_with(r, &mut last)?;
    let (_major, _maintenance) = (r.bl()?, r.bl()?);
    let (base, start, axis) = (r.bd3()?, r.bd3()?, r.bd3()?);
    let (radius, turns, turn_height) = (r.bd()?, r.bd()?, r.bd()?);
    let ccw = r.b()?;
    let constraint = r.rc()?;
    use draftcraft_dxf::Tag;
    let point = |code: i32, p: [f64; 3]| [Tag::f(code, p[0]), Tag::f(code + 10, p[1]), Tag::f(code + 20, p[2])];
    let mut tags = vec![Tag::s(100, "AcDbSpline")];
    tags.extend(last.into_iter().flat_map(|p| point(10, p)));
    tags.push(Tag::s(100, "AcDbHelix"));
    tags.extend(point(10, base));
    tags.extend(point(11, start));
    tags.extend(point(12, axis));
    tags.extend([Tag::f(40, radius), Tag::f(41, turns), Tag::f(42, turn_height), Tag::i(290, i64::from(ccw)), Tag::i(280, i64::from(constraint))]);
    Ok(ready(EntityKind::Helix(crate::dxf_read::helix(&tags)), 0.0, UP))
}

/// A spline's data; `last` takes the last control point, or fit point, as the file has it.
fn spline_with(r: &mut ObjectReader, last: &mut Option<[f64; 3]>) -> DwgResult<Shape> {
    let ver = r.version();
    let mut scenario = r.bl()?;
    let mut closed = false;
    let mut knot_param = None;
    if ver >= V::R2013 {
        let flags = r.bl()?;
        let param = r.bl()?;
        if flags & 1 != 0 {
            scenario = 2;
        }
        if param == 15 {
            scenario = 1;
        }
        closed = flags & 4 != 0;
        knot_param = Some(match param {
            1 => draftcraft_geom::KnotParam::SqrtChord,
            2 => draftcraft_geom::KnotParam::Uniform,
            _ => draftcraft_geom::KnotParam::Chord,
        });
    }
    let degree = usize::try_from(r.bl()?).unwrap_or(3);
    let (mut knots, mut control, mut weights, mut fit) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut start_tangent, mut end_tangent, mut tolerance) = (None, None, 0.0);
    let flat = |p: [f64; 3]| Vec2::new(p[0], p[1]);
    // A tangent the file leaves unset is stored as zero.
    let tangent = |p: [f64; 3]| (p != [0.0; 3]).then(|| flat(p));
    match scenario {
        2 => {
            tolerance = r.bd()?;
            start_tangent = tangent(r.bd3()?);
            end_tangent = tangent(r.bd3()?);
            let n = r.bl()?;
            let n = r.count(n, 6, "SPLINE fit point")?;
            for _ in 0..n {
                let p = r.bd3()?;
                *last = Some(p);
                fit.push(flat(p));
            }
            // From AutoCAD 2013 on the file says how the knots are spaced, and AutoCAD's tangents
            // are the curve's end derivatives, length included: the control points are AutoCAD's
            // (its DXF lists them). Tangents of unit length are directions, as DraftCraft's writer
            // (acadrust) stores them and the drawing holds them: the drawing's own fit.
            if let Some(knots) = knot_param.filter(|_| !closed && fit.len() >= 2) {
                let opts = draftcraft_geom::FitOptions { knots, start_tangent, end_tangent, tolerance: tolerance.max(0.0) };
                let unit = |t: Option<Vec2>| t.is_none_or(|v| (v.len() - 1.0).abs() <= 1e-9);
                let sp = if unit(start_tangent) && unit(end_tangent) {
                    draftcraft_geom::Spline::fit_with(&fit, false, opts)
                } else {
                    draftcraft_geom::Spline::fit_with_derivatives(&fit, opts, start_tangent, end_tangent)
                };
                return Ok(ready(EntityKind::Spline(sp), 0.0, UP));
            }
        }
        1 => {
            let _rational = r.b()?;
            closed = r.b()?;
            let _periodic = r.b()?;
            let _knot_tolerance = r.bd()?;
            let _control_tolerance = r.bd()?;
            let n_knots = r.bl()?;
            let n_control = r.bl()?;
            let weighted = r.b()?;
            let n_knots = r.count(n_knots, 2, "SPLINE knot")?;
            let n_control = r.count(n_control, 6, "SPLINE control point")?;
            for _ in 0..n_knots {
                knots.push(r.bd()?);
            }
            for _ in 0..n_control {
                let p = r.bd3()?;
                *last = Some(p);
                control.push(flat(p));
                if weighted {
                    weights.push(r.bd()?);
                }
            }
        }
        other => return Err(draftcraft_dwg::records::Error::format(format!("unknown spline scenario {other}"))),
    }
    let sp = spline_from(SplineParts { degree, knots, control, weights, fit, start_tangent, end_tangent, tolerance, closed });
    Ok(ready(EntityKind::Spline(sp), 0.0, UP))
}

/// What an entity type's fields give: the body, and the thickness and extrusion of the types that
/// have them.
struct Shape {
    body: Body,
    thickness: f64,
    normal: Vec3,
}

const UP: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 1.0 };

fn ready(kind: EntityKind, thickness: f64, normal: Vec3) -> Shape {
    Shape { body: Body::Ready(Box::new(kind)), thickness, normal }
}

/// LINE (§20.4.17). From AutoCAD 2000 on each end's coordinates are stored against the start's.
fn line(r: &mut ObjectReader) -> DwgResult<Shape> {
    let (a, b) = if r.version() <= V::R14 {
        (v3(r.bd3()?), v3(r.bd3()?))
    } else {
        let flat = r.b()?;
        let ax = r.rd()?;
        let bx = r.dd(ax)?;
        let ay = r.rd()?;
        let by = r.dd(ay)?;
        let (az, bz) = if flat {
            (0.0, 0.0)
        } else {
            let az = r.rd()?;
            (az, r.dd(az)?)
        };
        (Vec3::new(ax, ay, az), Vec3::new(bx, by, bz))
    };
    let thickness = r.thickness()?;
    let normal = v3(r.extrusion()?);
    Ok(ready(EntityKind::Line(Line { a, b }), thickness, normal))
}

/// POINT (§20.4.27).
fn point(r: &mut ObjectReader) -> DwgResult<Shape> {
    let p = v3(r.bd3()?);
    let thickness = r.thickness()?;
    let normal = v3(r.extrusion()?);
    let angle = r.bd()?;
    Ok(ready(EntityKind::Point(Point { p, angle }), thickness, normal))
}

/// CIRCLE and ARC (§20.4.18, §20.4.19): the centre in the object coordinate system.
fn circle_or_arc(r: &mut ObjectReader, arc: bool) -> DwgResult<Shape> {
    let center = v3(r.bd3()?);
    let radius = r.bd()?.abs();
    let thickness = r.thickness()?;
    let normal = v3(r.extrusion()?);
    let center = ocs_n(normal, center);
    if !arc {
        return Ok(ready(EntityKind::Circle(Circle { center, radius }), thickness, normal));
    }
    let mut a = Arc { center, radius, start: r.bd()?, end: r.bd()? };
    // In a mirrored coordinate system the arc runs the other way.
    if normal.z < 0.0 {
        let (s, e) = (std::f64::consts::PI - a.end, std::f64::consts::PI - a.start);
        a.start = draftcraft_geom::norm_angle(s);
        a.end = draftcraft_geom::norm_angle(e);
    }
    Ok(ready(EntityKind::Arc(a), thickness, normal))
}

/// ELLIPSE (§20.4.35).
fn ellipse(r: &mut ObjectReader) -> DwgResult<Shape> {
    let center = v3(r.bd3()?);
    let major = v3(r.bd3()?);
    let normal = v3(r.bd3()?);
    let mut e = Ellipse { center, major, ratio: r.bd()?.clamp(1e-9, 1.0), start: r.bd()?, end: r.bd()? };
    // Centre and major axis are in world coordinates, but the minor axis is extrusion × major:
    // mirrored, a partial ellipse runs the other way.
    if normal.z < 0.0 && (draftcraft_geom::ccw_sweep(e.start, e.end) - std::f64::consts::TAU).abs() > 1e-9 {
        let (s, t) = (-e.end, -e.start);
        e.start = draftcraft_geom::norm_angle(s);
        e.end = draftcraft_geom::norm_angle(t);
    }
    Ok(ready(EntityKind::Ellipse(e), 0.0, normal))
}

/// LWPOLYLINE (§20.4.85): flags saying which fields are stored, then the counts and the lists.
fn lwpolyline(r: &mut ObjectReader) -> DwgResult<Shape> {
    let ver = r.version();
    let flags = r.bs()?;
    let const_width = if flags & 4 != 0 { r.bd()? } else { 0.0 };
    let elevation = if flags & 8 != 0 { r.bd()? } else { 0.0 };
    let thickness = if flags & 2 != 0 { r.bd()? } else { 0.0 };
    let normal = if flags & 1 != 0 { v3(r.bd3()?) } else { UP };
    let points = r.bl()?;
    let bulges = if flags & 16 != 0 { r.bl()? } else { 0 };
    let ids = if ver >= V::R2010 && flags & 1024 != 0 { r.bl()? } else { 0 };
    let widths = if flags & 32 != 0 { r.bl()? } else { 0 };
    let points = r.count(points, 4, "LWPOLYLINE point")?;
    let bulges = r.count(bulges, 2, "LWPOLYLINE bulge")?;
    let ids = r.count(ids, 2, "LWPOLYLINE vertex id")?;
    let widths = r.count(widths, 4, "LWPOLYLINE width")?;
    let mut vertices: Vec<PolyVertex> = Vec::with_capacity(points);
    let mut prev = Vec2::ZERO;
    for i in 0..points {
        // From AutoCAD 2000 on each point is stored against the one before.
        let p = if ver <= V::R14 || i == 0 {
            let [x, y] = r.rd2()?;
            Vec2::new(x, y)
        } else {
            let x = r.dd(prev.x)?;
            Vec2::new(x, r.dd(prev.y)?)
        };
        prev = p;
        vertices.push(PolyVertex { p, start_width: 0.0, end_width: 0.0, bulge: 0.0 });
    }
    for i in 0..bulges {
        let b = r.bd()?;
        if let Some(v) = vertices.get_mut(i) {
            v.bulge = b;
        }
    }
    for _ in 0..ids {
        r.bl()?;
    }
    for i in 0..widths {
        let (s, e) = (r.bd()?, r.bd()?);
        if let Some(v) = vertices.get_mut(i) {
            (v.start_width, v.end_width) = (s, e);
        }
    }
    // The vertices are in the polyline's own coordinate system: mirrored, x runs the other way
    // and the arcs bulge the other way.
    if normal.z < 0.0 {
        for v in &mut vertices {
            v.p.x = -v.p.x;
            v.bulge = -v.bulge;
        }
    }
    let p = LwPolyline { vertices, closed: flags & 512 != 0, const_width, elevation, plinegen: flags & 256 != 0 };
    Ok(ready(EntityKind::LwPolyline(p), thickness, normal))
}

/// The fields of a text, which attributes and attribute definitions start with too.
struct TextFields {
    text: Text,
    align: Vec3,
    insert: Vec3,
    thickness: f64,
    normal: Vec3,
}

/// TEXT (§20.4.3).
fn text(r: &mut ObjectReader) -> DwgResult<Shape> {
    let f = text_fields(r)?;
    Ok(Shape { body: Body::Text(Box::new(f.text), Some(f.align), f.insert), thickness: f.thickness, normal: f.normal })
}

/// The fields of a TEXT. From AutoCAD 2000 on a flag byte says which fields have their default
/// and are left out.
fn text_fields(r: &mut ObjectReader) -> DwgResult<TextFields> {
    let (mut elevation, mut oblique, mut rotation, mut width_factor) = (0.0, 0.0, 0.0, 1.0);
    let (mut h_align, mut v_align) = (0u16, 0u16);
    let (insertion, alignment, height, value, normal, thickness);
    if r.version() <= V::R14 {
        elevation = r.bd()?;
        insertion = r.rd2()?;
        alignment = r.rd2()?;
        normal = v3(r.bd3()?);
        thickness = r.bd()?;
        oblique = r.bd()?;
        rotation = r.bd()?;
        height = r.bd()?;
        width_factor = r.bd()?;
        value = r.text()?;
        let _generation = r.bs()?;
        h_align = r.bs()?;
        v_align = r.bs()?;
    } else {
        let flags = r.rc()?;
        if flags & 0x01 == 0 {
            elevation = r.rd()?;
        }
        insertion = r.rd2()?;
        // Not stored (flag 0x02): the alignment point is (0, 0). AutoCAD's own files show it: the
        // R14 file stores (0, 0) for a left-aligned text, and the 2018 file of the same drawing
        // leaves the same text's point out (its insertion point is not (0, 0)).
        alignment = if flags & 0x02 == 0 { [r.dd(insertion[0])?, r.dd(insertion[1])?] } else { [0.0, 0.0] };
        normal = v3(r.extrusion()?);
        thickness = r.thickness()?;
        if flags & 0x04 == 0 {
            oblique = r.rd()?;
        }
        if flags & 0x08 == 0 {
            rotation = r.rd()?;
        }
        height = r.rd()?;
        if flags & 0x10 == 0 {
            width_factor = r.rd()?;
        }
        value = r.text()?;
        if flags & 0x20 == 0 {
            let _generation = r.bs()?;
        }
        if flags & 0x40 == 0 {
            h_align = r.bs()?;
        }
        if flags & 0x80 == 0 {
            v_align = r.bs()?;
        }
    }
    let halign = match h_align {
        1 => HAlign::Center,
        2 => HAlign::Right,
        3 => HAlign::Aligned,
        4 => HAlign::Middle,
        5 => HAlign::Fit,
        _ => HAlign::Left,
    };
    let valign = match v_align {
        1 => VAlign::Bottom,
        2 => VAlign::Middle,
        3 => VAlign::Top,
        _ => VAlign::Baseline,
    };
    let at = |p: [f64; 2]| Vec3::new(p[0], p[1], elevation);
    let plain = halign == HAlign::Left && valign == VAlign::Baseline;
    let t = Text {
        insert: at(insertion),
        align_pt: (!plain).then(|| at(alignment)),
        height: height.max(1e-9),
        value,
        rotation,
        width_factor,
        oblique,
        style: "Standard".into(),
        halign,
        valign,
    };
    Ok(TextFields { text: t, align: at(alignment), insert: at(insertion), thickness, normal })
}

/// ATTRIB and ATTDEF (§20.4.4, §20.4.5): a text's fields, then the attribute's own.
fn attribute(r: &mut ObjectReader, definition: bool, partial: &mut Vec<&'static str>) -> DwgResult<Shape> {
    let ver = r.version();
    let f = text_fields(r)?;
    if ver >= V::R2010 {
        let _version = r.rc()?;
    }
    let (mut multiline, mut embedded) = (None, None);
    if ver >= V::R2018 {
        // 1 a single line; 2 a multiline attribute, 4 a multiline definition, whose text is an
        // MTEXT embedded here: an entity's common properties (its own copy, as AutoCAD's files
        // show), the MTEXT's fields, then the number of secondary attributes. As in the DXF
        // reader, the attribute's value is the text's plain reading.
        let kind = r.rc()?;
        if kind > 1 {
            embedded = Some(embedded_entity_head(r)?);
            let Shape { body: Body::MText(m), .. } = mtext(r, partial)? else {
                return Err(draftcraft_dwg::records::Error::format("multiline attribute without its text"));
            };
            if r.bs()? != 0 {
                partial.push("multiline attribute's secondary attributes");
            }
            let read_as = draftcraft_fonts::plain_mtext(&m.contents);
            multiline = Some(MultilineAttrib { mtext: *m, read_as });
        }
    }
    let tag = r.text()?;
    let _field_length = r.bs()?;
    let flags = r.rc()?;
    let lock_position = if ver >= V::R2007 { r.b()? } else { false };
    let mut prompt = String::new();
    if definition {
        if ver >= V::R2010 {
            let _version = r.rc()?;
        }
        prompt = r.text()?;
    }
    let mut text = f.text;
    if let Some(m) = &multiline {
        text.value.clone_from(&m.read_as);
    }
    let a = Attrib {
        tag,
        text,
        invisible: flags & 1 != 0,
        constant: flags & 2 != 0,
        prompt,
        props: AttribProps::default(),
        multiline,
        modes: AttribModes { verify: flags & 4 != 0, preset: flags & 8 != 0, lock_position },
    };
    Ok(Shape { body: Body::Attrib(Box::new(a), Some(f.align), f.insert, definition, embedded), thickness: f.thickness, normal: f.normal })
}

/// The common properties an entity embedded in another has (AutoCAD 2018's multiline attribute
/// text): its mode, reactors, extension dictionary and binary data flags, colour, linetype scale,
/// linetype, plot style and material flags, shadows, visual styles, visibility and lineweight.
/// The attribute's own properties are the ones the drawing keeps.
fn embedded_entity_head(r: &mut ObjectReader) -> DwgResult<EmbeddedRefs> {
    let mode = r.bb()?;
    let reactors = r.bl()?;
    let no_extension_dictionary = r.b()?;
    let _binary_data = r.b()?;
    let color = r.bs()?;
    if color & 0x8000 != 0 && color & 0x4000 == 0 {
        let _true_color = r.bl()?;
    }
    if color & 0x2000 != 0 {
        let _transparency = r.bl()?;
    }
    let _linetype_scale = r.bd()?;
    // Linetype, material and plot style: 3 when the entity names one by a handle.
    let (linetype, material, plot_style) = (r.bb()? == 3, r.bb()? == 3, r.bb()? == 3);
    let _shadows = r.rc()?;
    let visual_styles = usize::from(r.b()?) + usize::from(r.b()?) + usize::from(r.b()?);
    let _invisible = r.bs()?;
    let _lineweight = r.rc()?;
    // Its handles, in the order the handle stream holds them: an owner (when the mode names
    // none), the reactors, an extension dictionary, the layer, then the ones flagged above.
    let handles = usize::from(mode == 0)
        + usize::try_from(reactors).unwrap_or(usize::MAX).min(MAX_EMBEDDED_REACTORS)
        + usize::from(!no_extension_dictionary)
        + 1
        + usize::from(linetype)
        + usize::from(material)
        + usize::from(plot_style)
        + visual_styles;
    Ok(EmbeddedRefs { handles })
}

/// The most reactors an embedded entity's handles are skipped for.
const MAX_EMBEDDED_REACTORS: usize = 4096;

/// What the handle stream holds for an entity embedded in another: `handles` of its own (owner,
/// reactors, extension dictionary, layer, linetype, material, plot style, visual styles), which
/// the drawing takes from the entity that holds it, then the embedded text's style.
#[derive(Clone, Copy, Debug)]
struct EmbeddedRefs {
    handles: usize,
}

/// A multiline attribute's text: its own handles (AutoCAD's files: a null owner, the layer, the
/// linetype, the material), then its text style.
fn attrib_text(a: &mut Attrib, embedded: Option<EmbeddedRefs>, r: &mut ObjectReader, styles: &HashMap<u64, String>) -> DwgResult<()> {
    let (Some(e), Some(m)) = (embedded, a.multiline.as_mut()) else { return Ok(()) };
    for _ in 0..e.handles {
        r.reference()?;
    }
    let h = r.reference()?;
    m.mtext.style = styles.get(&h).cloned().unwrap_or_else(|| "Standard".into());
    Ok(())
}

/// MTEXT (§20.4.46).
fn mtext(r: &mut ObjectReader, partial: &mut Vec<&'static str>) -> DwgResult<Shape> {
    let ver = r.version();
    let insert = v3(r.bd3()?);
    let normal = v3(r.bd3()?);
    let x_axis = v3(r.bd3()?);
    let width = r.bd()?;
    let defined_height = if ver >= V::R2007 { r.bd()? } else { 0.0 };
    let height = r.bd()?;
    let attach = r.bs()?;
    let _direction = r.bs()?;
    let _extents_height = r.bd()?;
    let extents_width = r.bd()?;
    let contents = r.text()?;
    let (mut line_spacing, mut line_spacing_exact) = (1.0, false);
    if ver >= V::R2000 {
        line_spacing_exact = r.bs()? == 2;
        line_spacing = r.bd()?;
        let _unknown = r.b()?;
    }
    // From AutoCAD 2004 on a background fill: its flags and, when it is filled, its scale,
    // colour and transparency. The drawing has no background fill.
    let mut columns = None;
    if ver >= V::R2004 {
        let background = r.bl()?;
        if background != 0 {
            partial.push("MTEXT background fill");
        }
        if background & 0x03 != 0 {
            let _scale = r.bd()?;
            let _color = r.color()?;
            let _transparency = r.bl()?;
        }
        // AutoCAD 2018: a text that is not annotative carries its columns (an annotative one has
        // them in its context data). Read only after the background flags AutoCAD's files show,
        // and kept only when they end where the record does.
        if ver >= V::R2018 && background & !0x13 == 0 && r.b()? {
            columns = mtext_columns(r, defined_height, extents_width)?;
            if columns.is_some() && r.remaining_bits() != 0 {
                columns = None;
                partial.push("MTEXT columns");
            }
        }
    }
    let mut rotation = Vec2::new(x_axis.x, x_axis.y).angle();
    let mut attach = u8::try_from(attach.clamp(1, 9)).unwrap_or(1);
    // Mirrored, the glyphs run the other way: drawn readable with the same footprint, the text
    // turns round and left and right attachment swap.
    if normal.z < 0.0 {
        rotation = draftcraft_geom::norm_angle(rotation + std::f64::consts::PI);
        attach = match attach {
            1 | 4 | 7 => attach + 2,
            3 | 6 | 9 => attach - 2,
            a => a,
        };
    }
    let t = MText {
        insert,
        height: height.max(1e-9),
        // An MTEXT's width is one column's, as the DXF reader has it.
        width: match &columns {
            Some((_, w)) if *w > 0.0 => *w,
            _ => width.max(0.0),
        },
        attach,
        rotation,
        style: "Standard".into(),
        contents,
        line_spacing,
        line_spacing_exact,
        columns: columns.map(|(c, _)| c),
    };
    Ok(Shape { body: Body::MText(Box::new(t)), thickness: 0.0, normal })
}

/// The columns of an AutoCAD 2018 MTEXT that is not annotative, and their width: its own copy of
/// the text's attachment, direction, insertion, size and extents (the MTEXT's own fields have
/// them), then the column type and, with columns, their count, width, gutter, auto height and
/// flow flags and (dynamic columns of manual heights) each column's height. By the DXF reader's
/// rules for the same groups of AutoCAD's DXF (group 101): a height AutoCAD stores negative (the
/// text's extents, negated) is 0 there, and dynamic auto-height columns store no count, which
/// follows from the extents.
fn mtext_columns(r: &mut ObjectReader, defined_height: f64, extents_width: f64) -> DwgResult<Option<(MTextColumns, f64)>> {
    let _version = r.bs()?;
    let _default = r.b()?;
    let _attach = r.bl()?;
    let _x_axis = r.bd3()?;
    let _insert = r.bd3()?;
    let (_rect_width, rect_height) = (r.bd()?, r.bd()?);
    let _extents = (r.bd()?, r.bd()?);
    let kind = match r.bl()? {
        1 => MTextColumnType::Static,
        2 => MTextColumnType::Dynamic,
        _ => return Ok(None),
    };
    let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
    let count = r.bl()?;
    let width = finite(r.bd()?).max(0.0);
    let gutter = finite(r.bd()?).max(0.0);
    let auto_height = r.b()?;
    let flow_reversed = r.b()?;
    let manual = kind == MTextColumnType::Dynamic && !auto_height;
    let mut heights = Vec::new();
    if manual {
        let max = u32::try_from(mtext_dxf::MAX_COLUMNS).unwrap_or(u32::MAX);
        for _ in 0..bounded(r, count, max, 2, "MTEXT column height")? {
            heights.push(finite(r.bd()?).max(0.0));
        }
    }
    let count = match i64::from(count) {
        n if n > 0 => n,
        _ if width + gutter > 0.0 && extents_width > 0.0 => ((extents_width + gutter) / (width + gutter)).round() as i64,
        _ => 1,
    };
    let defined = [defined_height, rect_height, heights.first().copied().unwrap_or(0.0)].into_iter().map(finite).find(|h| *h > 0.0).unwrap_or(0.0);
    let c = MTextColumns {
        kind,
        count: u16::try_from(count.clamp(1, mtext_dxf::MAX_COLUMNS as i64)).unwrap_or(1),
        gutter,
        height: if manual { heights.iter().copied().fold(0.0, f64::max) } else { defined },
        auto_height,
        heights: if manual { heights } else { Vec::new() },
        flow_reversed,
    };
    Ok(Some((c, width)))
}

/// INSERT and MINSERT (§20.4.7, §20.4.8). From AutoCAD 2000 on the scale is stored against what it
/// usually is: all three equal, or 1.
fn insert(r: &mut ObjectReader, minsert: bool) -> DwgResult<Shape> {
    let ver = r.version();
    let mut p = v3(r.bd3()?);
    let mut scale = if ver <= V::R14 {
        v3(r.bd3()?)
    } else {
        match r.bb()? {
            0 => {
                let x = r.rd()?;
                let y = r.dd(x)?;
                Vec3::new(x, y, r.dd(x)?)
            }
            1 => {
                let y = r.dd(1.0)?;
                Vec3::new(1.0, y, r.dd(1.0)?)
            }
            2 => {
                let x = r.rd()?;
                Vec3::new(x, x, x)
            }
            _ => Vec3::new(1.0, 1.0, 1.0),
        }
    };
    let mut rotation = r.bd()?;
    let normal = v3(r.bd3()?);
    let has_attributes = r.b()?;
    let owned = if ver >= V::R2004 && has_attributes { r.bl()? } else { 0 };
    let (mut cols, mut rows, mut col_spacing, mut row_spacing) = (1u32, 1u32, 0.0, 0.0);
    if minsert {
        cols = u32::from(r.bs()?);
        rows = u32::from(r.bs()?);
        col_spacing = r.bd()?;
        row_spacing = r.bd()?;
    }
    // A mirrored block reference: in the world that is the mirrored point, a negative x scale,
    // the opposite rotation and a negative column spacing. A truly tilted one is left as read.
    if normal.z < 0.0 && normal.x.hypot(normal.y) <= 1e-3 * normal.z.abs() {
        p = ocs_n(normal, p);
        scale.x = -scale.x;
        rotation = -rotation;
        col_spacing = -col_spacing;
    }
    let i = Insert {
        block: String::new(),
        insert: p,
        scale,
        rotation,
        attribs: Vec::new(),
        cols: cols.clamp(1, 10_000),
        rows: rows.clamp(1, 10_000),
        col_spacing,
        row_spacing,
        clip: None,
    };
    Ok(Shape { body: Body::Insert(Box::new(i), has_attributes, owned), thickness: 0.0, normal })
}

/// SOLID and TRACE (§20.4.31, §20.4.32): four corners in the object coordinate system.
fn solid(r: &mut ObjectReader, trace: bool) -> DwgResult<Shape> {
    let thickness = r.thickness()?;
    let elevation = r.bd()?;
    let mut flat = [[0.0; 2]; 4];
    for c in &mut flat {
        *c = r.rd2()?;
    }
    let normal = v3(r.extrusion()?);
    let corners = flat.map(|q| ocs_n(normal, Vec3::new(q[0], q[1], elevation)));
    let kind = if trace { EntityKind::Trace(Solid { corners }) } else { EntityKind::Solid(Solid { corners }) };
    Ok(ready(kind, thickness, normal))
}

/// 3DFACE (§20.4.33). From AutoCAD 2000 on each corner is stored against the one before.
fn face3d(r: &mut ObjectReader) -> DwgResult<Shape> {
    let mut corners = [Vec3::ZERO; 4];
    let mut edges = 0u16;
    if r.version() <= V::R14 {
        for c in &mut corners {
            *c = v3(r.bd3()?);
        }
        edges = r.bs()?;
    } else {
        let no_flags = r.b()?;
        let flat = r.b()?;
        let (x, y) = (r.rd()?, r.rd()?);
        let mut prev = Vec3::new(x, y, if flat { 0.0 } else { r.rd()? });
        if let Some(first) = corners.first_mut() {
            *first = prev;
        }
        for c in corners.iter_mut().skip(1) {
            let x = r.dd(prev.x)?;
            let y = r.dd(prev.y)?;
            prev = Vec3::new(x, y, r.dd(prev.z)?);
            *c = prev;
        }
        if !no_flags {
            edges = r.bs()?;
        }
    }
    Ok(ready(EntityKind::Face3d(Face3d { corners, hidden_edges: u8::try_from(edges & 0x0f).unwrap_or(0) }), 0.0, UP))
}

/// XLINE and RAY (§20.4.43, §20.4.42).
fn ray_line(r: &mut ObjectReader, ray: bool) -> DwgResult<Shape> {
    let line = RayLine { base: v3(r.bd3()?), dir: v3(r.bd3()?) };
    Ok(ready(if ray { EntityKind::Ray(line) } else { EntityKind::XLine(line) }, 0.0, UP))
}

/// The drawing's entity of a polyline from its vertices, by the DXF reader's rules: a 2D polyline
/// is a lightweight polyline, a 3D one a 3D polyline, a polyface mesh keeps the faces that name
/// three vertices or more, and a polygon mesh becomes 3D faces, the first in the polyline's place
/// and the rest after it. `None` for a mesh with no face.
fn polyline_kinds(kind: PolyKind, vertices: &[Vertex], normal: Vec3) -> (Option<EntityKind>, Vec<EntityKind>) {
    // Spline frame control points (flag 16) are not points of the curve.
    let points = || vertices.iter().filter(|v| v.face.is_none() && v.flags & 16 == 0);
    match kind {
        PolyKind::TwoD { flags, start_width, end_width, elevation } => {
            // The vertices are in the polyline's own coordinate system: mirrored like LWPOLYLINE.
            let m = if normal.z < 0.0 { -1.0 } else { 1.0 };
            let vertices = points()
                .map(|v| PolyVertex {
                    p: Vec2::new(m * v.point.x, v.point.y),
                    bulge: m * v.bulge,
                    start_width: v.start_width,
                    end_width: v.end_width,
                })
                .collect();
            let p = LwPolyline {
                vertices,
                closed: flags & 1 != 0,
                const_width: start_width.min(end_width).max(0.0),
                elevation,
                plinegen: flags & 128 != 0,
            };
            (Some(EntityKind::LwPolyline(p)), Vec::new())
        }
        PolyKind::ThreeD { closed } => (Some(EntityKind::Polyline3d(Polyline3d { points: points().map(|v| v.point).collect(), closed })), Vec::new()),
        PolyKind::Pface => {
            let mut mesh = PolyfaceMesh { vertices: points().map(|v| v.point).collect(), faces: Vec::new() };
            let faces: Vec<[i32; 4]> = vertices.iter().filter_map(|v| v.face).map(|f| f.map(i32::from)).collect();
            mesh.faces = faces.into_iter().filter(|f| mesh.face3d(f).is_some()).collect();
            if mesh.faces.is_empty() { (None, Vec::new()) } else { (Some(EntityKind::PolyfaceMesh(mesh)), Vec::new()) }
        }
        PolyKind::Mesh { flags, size, smooth } => {
            let verts: Vec<Vec3> = points().map(|v| v.point).collect();
            let pair = |(a, b): (u16, u16)| Some((i64::from(a), i64::from(b)));
            let mut faces = mesh_faces_of(&verts, pair(size), pair(smooth), i64::from(flags)).into_iter().map(EntityKind::Face3d);
            (faces.next(), faces.collect())
        }
    }
}

/// One entity's type fields, or `None` for a type read in a later step.
fn shape(r: &mut ObjectReader, type_name: &str, partial: &mut Vec<&'static str>) -> DwgResult<Option<Shape>> {
    Ok(Some(match type_name {
        "LINE" => line(r)?,
        "POINT" => point(r)?,
        "CIRCLE" => circle_or_arc(r, false)?,
        "ARC" => circle_or_arc(r, true)?,
        "ELLIPSE" => ellipse(r)?,
        "LWPOLYLINE" => lwpolyline(r)?,
        "TEXT" => text(r)?,
        "MTEXT" => mtext(r, partial)?,
        "INSERT" => insert(r, false)?,
        "ATTRIB" => attribute(r, false, partial)?,
        "POLYLINE_2D" => polyline_2d(r)?,
        "POLYLINE_3D" => polyline_3d(r)?,
        "POLYLINE_PFACE" => polyline_pface(r)?,
        "POLYLINE_MESH" => polyline_mesh(r)?,
        "VERTEX_2D" => vertex_2d(r)?,
        "VERTEX_3D" | "VERTEX_MESH" | "VERTEX_PFACE" => vertex_3d(r)?,
        "VERTEX_PFACE_FACE" => pface_face(r)?,
        "SPLINE" => spline(r)?,
        "HATCH" => hatch(r)?,
        "LEADER" => leader(r)?,
        "TOLERANCE" => tolerance(r)?,
        "MULTILEADER" => mleader(r)?,
        "VIEWPORT" => return viewport(r),
        "IMAGE" => raster(r, false)?,
        "MLINE" => mline(r)?,
        "WIPEOUT" => raster(r, true)?,
        "PDFUNDERLAY" => underlay(r)?,
        "DIMENSION_ORDINATE" | "DIMENSION_LINEAR" | "DIMENSION_ALIGNED" | "DIMENSION_ANG_3PT" | "DIMENSION_ANG_2LN" | "DIMENSION_RADIUS"
        | "DIMENSION_DIAMETER" => dimension(r, type_name)?,
        "ATTDEF" => attribute(r, true, partial)?,
        "MINSERT" => insert(r, true)?,
        "SOLID" => solid(r, false)?,
        "TRACE" => solid(r, true)?,
        "3DFACE" => face3d(r)?,
        "XLINE" => ray_line(r, false)?,
        "RAY" => ray_line(r, true)?,
        _ => return more_shapes(r, type_name),
    }))
}

/// The entity types added after the first: a helix, and the types the drawing keeps as their
/// DXF groups without modelling them.
fn more_shapes(r: &mut ObjectReader, type_name: &str) -> DwgResult<Option<Shape>> {
    match type_name {
        "HELIX" => helix(r).map(Some),
        "OLE2FRAME" => ole2frame(r).map(Some),
        "3DSOLID" => modeler(r, "3DSOLID"),
        "REGION" => modeler(r, "REGION"),
        "BODY" => modeler(r, "BODY"),
        _ => Ok(None),
    }
}

/// A table (ACAD_TABLE): its block reference's fields, then its own, read with its references
/// (they sit between its fields). A table from another writer waits for its style's text height.
fn table_entity(r: &mut ObjectReader, o: &ObjectRef, n: &Names) -> DwgResult<(Decoded, EntityRefs)> {
    let Shape { body: Body::Insert(ins, has_attributes, _), normal, .. } = insert(r, false)? else {
        return Err(draftcraft_dwg::records::Error::format("table without its block reference"));
    };
    let refs = r.entity_refs()?;
    let block = r.reference()?;
    if has_attributes {
        return Err(draftcraft_dwg::records::Error::format("table with attributes"));
    }
    let t = super::table::read(r)?;
    let mut partial = t.partial;
    let tags = xdata_tags(&r.xdata(), &n.tables.apps);
    let ours = crate::dxf_ext::xdata(&tags, crate::dxf_ext::APP).iter().any(|t| t.code == 1000 && t.str() == "TABLE");
    let size = |v: f64| if v.is_finite() && v > 0.0 { v } else { 0.27 };
    let kind = crate::dxf_read::table_from(
        ins.insert,
        t.col_widths.into_iter().map(size).collect(),
        t.row_heights.into_iter().map(size).collect(),
        t.cells,
        &tags,
    );
    let mut c = common(r, &refs, n, &mut partial);
    partial.retain(|p| *p != DRAFTCRAFT_DATA);
    c.extrusion = normal;
    let mut e = Entity::new(Handle(o.handle), EntityKind::Table(kind));
    e.common = c;
    let fix = (!ours).then(|| n.objects.table_styles.get(&t.style).cloned());
    let table = Some((n.blocks.get(&block).cloned(), fix));
    let d = Decoded { entity: Some(e), table, partial, ..Decoded::default() };
    Ok((d, refs))
}

/// One entity. The type's own fields come first, then its references.
fn entity(r: &mut ObjectReader, o: &ObjectRef, n: &Names) -> DwgResult<(Decoded, EntityRefs)> {
    if o.type_name == "ACAD_TABLE" {
        return table_entity(r, o, n);
    }
    let mut partial = Vec::new();
    let Some(Shape { body, thickness, normal }) = shape(r, &o.type_name, &mut partial)? else {
        // A type read in a later step: only its place is known.
        let refs = r.entity_refs()?;
        return Ok((Decoded { partial, ..Decoded::default() }, refs));
    };
    let refs = r.entity_refs()?;
    let style = |r: &mut ObjectReader| -> DwgResult<String> {
        let h = r.reference()?;
        Ok(n.tables.styles.get(&h).cloned().unwrap_or_else(|| "Standard".into()))
    };
    let (mut attrib, mut attribs, mut polyline, mut vertex, mut mleader_attribs) = (None, None, None, None, None);
    let mut leader_hook = None;
    let mut paper_view = false;
    let mut held = Vec::new();
    let kind = match body {
        Body::Ready(k) => match *k {
            // A region DraftCraft wrote is closed polylines, the first with the region's other
            // loops in its extended data, the others marked as its parts: by the DXF reader's rule.
            EntityKind::LwPolyline(pl) => {
                let tags = xdata_tags(&r.xdata(), &n.tables.apps);
                let kind = crate::dxf_region::from_polyline(&tags, pl);
                if kind.is_none() {
                    held.push(o.handle);
                }
                kind
            }
            k => Some(k),
        },
        Body::Unknown(dxf_type, tags) => Some(EntityKind::Unknown(draftcraft_doc::Unknown { dxf_type: dxf_type.into(), tags })),
        Body::Text(mut t, align, insert) => {
            t.style = style(r)?;
            if normal.z < 0.0 {
                mirror_text(&mut t, align.map(|a| ocs_n(normal, a)), ocs_n(normal, insert));
            }
            Some(EntityKind::Text(*t))
        }
        Body::MText(mut t) => {
            t.style = style(r)?;
            Some(EntityKind::MText(*t))
        }
        Body::Insert(mut i, has_attributes, owned) => {
            let block = r.reference()?;
            if has_attributes {
                attribs = Some(owned_list(r, owned)?);
            }
            n.blocks.get(&block).map(|name| {
                i.block.clone_from(name);
                EntityKind::Insert(*i)
            })
        }
        Body::Polyline(kind, owned) => {
            let list = owned_list(r, owned)?;
            let mut c = common(r, &refs, n, &mut partial);
            c.thickness = thickness;
            c.extrusion = normal;
            polyline = Some((kind, c, list));
            None
        }
        Body::Dimension(mut dm, oblique) => {
            let style = r.reference()?;
            let block = r.reference()?;
            dm.style = n.tables.dim_styles.get(&style).cloned().unwrap_or_else(|| "Standard".into());
            dm.block = n.blocks.get(&block).cloned().filter(|b| !b.is_empty());
            // The overrides, associativity and marks the extended data holds, by the DXF
            // reader's rules for them.
            let tags = xdata_tags(&r.xdata(), &n.tables.apps);
            let (styles, blocks, ltypes) = &n.by_hex;
            dm.overrides = crate::dxf_ext::read_dstyle(&tags, styles, blocks, ltypes);
            dm.assoc = crate::dxf_ext::read_assoc(&tags);
            if let Some(center) = crate::dxf_ext::read_arclen(&tags) {
                dm.kind = DimKind::ArcLength;
                dm.p15 = center;
            }
            dm.marks = crate::dxf_ext::read_marks(&tags);
            // The extension lines' angle relative to the dimension line (DIMEDIT Oblique), as
            // a DXF holds it in group 52, in degrees.
            let base = match dm.kind {
                DimKind::Linear { rotation } => Some(rotation),
                DimKind::Aligned => Some((dm.p14.xy() - dm.p13.xy()).angle()),
                _ => None,
            };
            if let Some(base) = base
                && oblique.is_finite()
                && oblique.to_degrees().rem_euclid(180.0) != 0.0
            {
                dm.marks.oblique = Some(draftcraft_geom::norm_angle(base + oblique));
            }
            Some(EntityKind::Dimension(*dm))
        }
        Body::Hatch(mut h, counts, raw) => {
            // The boundary objects each loop names, then what the extended data holds.
            for (l, count) in h.loops.iter_mut().zip(counts) {
                for _ in 0..count.min(MAX_HATCH_ITEMS) {
                    let s = r.reference()?;
                    if s != 0 {
                        l.sources.push(Handle(s));
                    }
                }
            }
            let tags = xdata_tags(&r.xdata(), &n.tables.apps);
            let (origin, background) = crate::dxf_read::hatch_xdata(&tags);
            h.origin = origin;
            h.background = background;
            h.pattern_lines = crate::dxf_read::pattern_lines_from(raw, h.scale, h.angle, origin);
            if h.gradient.is_none() {
                h.gradient = crate::dxf_read::xdata_gradient(&tags);
            }
            Some(EntityKind::Hatch(*h))
        }
        Body::MLine(scale, just, flags, vertices) => {
            let style = r.reference()?;
            let name = n.objects.mline_styles.get(&style).cloned().unwrap_or_default();
            Some(EntityKind::MLine(crate::dxf_mline::build(name, scale, draftcraft_doc::MLineJust::from_dxf(just), flags, vertices)))
        }
        Body::Raster(mut i) => {
            let def = r.reference()?;
            let reactor = r.reference()?;
            if let Some(d) = n.objects.images.get(&def) {
                i.path.clone_from(&d.path);
                i.name.clone_from(&d.name);
                if let Some(p) = d.pixel_size {
                    i.pixel_size = p;
                }
                i.resolution_units = d.units;
                held.extend([def, reactor].into_iter().filter(|h| *h != 0));
            }
            Some(EntityKind::Image(*i))
        }
        Body::Underlay(mut i) => {
            let def = r.reference()?;
            if let Some(d) = n.objects.pdfs.get(&def) {
                i.path.clone_from(&d.path);
                i.name.clone_from(&d.name);
                if let Some(u) = &mut i.underlay {
                    u.page.clone_from(&d.page);
                }
                held.push(def);
            }
            Some(EntityKind::Image(*i))
        }
        Body::Viewport(mut v, frozen, status) => {
            // The frozen layers it names, then those DraftCraft keeps in its extended data (the
            // ones without a layer record), each once; then the clipping boundary.
            let mut handles = Vec::new();
            for _ in 0..frozen.min(1 << 16) {
                handles.push(r.reference()?);
            }
            let clip = r.reference()?;
            if r.version() == V::R2000 {
                let _header = r.reference()?;
            }
            let tags = xdata_tags(&r.xdata(), &n.tables.apps);
            let named = handles.iter().filter_map(|h| n.tables.layers.get(h)).cloned();
            let mut seen = HashSet::new();
            v.frozen_layers = named.chain(crate::dxf_ext::read_frozen(&tags)).filter(|l| seen.insert(l.to_ascii_uppercase())).collect();
            v.layer_colors = crate::dxf_ext::read_layer_colors(&tags);
            v.clip = (clip != 0 && status & 65536 != 0).then_some(Handle(clip));
            // The sheet viewport DraftCraft's writer adds is numbered with the others, then
            // left implicit again, as the DXF reader does.
            paper_view = crate::dxf_ext::is_paper_view(&tags);
            Some(EntityKind::Viewport(*v))
        }
        Body::Leader(mut l, hook) => {
            let annotation = r.reference()?;
            let style = r.reference()?;
            l.style = n.tables.dim_styles.get(&style).cloned().unwrap_or_else(|| "Standard".into());
            leader_hook = hook.filter(|_| annotation != 0);
            Some(EntityKind::Leader(*l))
        }
        Body::Tolerance(mut t) => {
            let style = r.reference()?;
            t.style = n.tables.dim_styles.get(&style).cloned().unwrap_or_else(|| "Standard".into());
            Some(EntityKind::Tolerance(*t))
        }
        Body::MLeader(mut p, slots) => {
            let (mut style, mut text_style, mut block) = (None, [None, None], [None, None]);
            for slot in slots {
                let h = r.reference()?;
                match slot {
                    MlRef::Skip => {}
                    MlRef::Style => style = Some(h),
                    MlRef::ContextTextStyle => text_style[0] = Some(h),
                    MlRef::TextStyle => text_style[1] = Some(h),
                    MlRef::ContextBlock => block[0] = Some(h),
                    MlRef::Block => block[1] = Some(h),
                    MlRef::AttDef(i) => {
                        if let Some(a) = p.attribs.get_mut(i) {
                            a.0 = format!("{h:X}");
                        }
                    }
                }
            }
            let mut m = crate::dxf_mleader::build(&p);
            if let Some(name) = style.and_then(|h| n.objects.mleader_styles.get(&h)).filter(|s| !s.is_empty()) {
                m.style.clone_from(name);
            }
            if let Some(t) = &mut m.text
                && let Some(name) = text_style.iter().flatten().find_map(|h| n.tables.styles.get(h)).filter(|s| !s.is_empty())
            {
                t.style.clone_from(name);
            }
            if m.block.is_some() {
                match block.iter().flatten().find_map(|h| n.blocks.get(h)).filter(|s| !s.is_empty()) {
                    Some(name) => {
                        if let Some(b) = &mut m.block {
                            b.block.clone_from(name);
                        }
                    }
                    None => m.block = None,
                }
            }
            if !p.attribs.is_empty() && m.block.is_some() {
                mleader_attribs = Some(std::mem::take(&mut p.attribs));
            }
            Some(EntityKind::MLeader(m))
        }
        Body::Vertex(v) => {
            vertex = Some(v);
            None
        }
        Body::Attrib(mut a, align, insert, definition, embedded) => {
            a.text.style = style(r)?;
            attrib_text(&mut a, embedded, r, &n.tables.styles)?;
            if normal.z < 0.0 {
                mirror_text(&mut a.text, align.map(|p| ocs_n(normal, p)), ocs_n(normal, insert));
            }
            if definition {
                Some(EntityKind::AttDef(*a))
            } else {
                let mut c = common(r, &refs, n, &mut partial);
                c.thickness = thickness;
                c.extrusion = normal;
                attrib = Some((*a, c));
                None
            }
        }
    };
    let entity = kind.map(|kind| {
        let mut c = common(r, &refs, n, &mut partial);
        // A dimension's ACAD extended data is its overrides, read above: as the DXF reader does,
        // it is not kept as other applications' data too.
        if matches!(kind, EntityKind::Dimension(_)) {
            c.xdata = without_app(std::mem::take(&mut c.xdata), "ACAD");
        }
        c.thickness = thickness;
        c.extrusion = normal;
        let mut e = Entity::new(Handle(o.handle), kind);
        if let EntityKind::Unknown(u) = &mut e.kind {
            let own = std::mem::take(&mut u.tags);
            u.tags = unknown_head(o.handle, &c);
            u.tags.extend(own);
            // Its extended data is among its groups too, as in a DXF file.
            u.tags.extend(c.xdata.iter().cloned());
        }
        e.common = c;
        e
    });
    if entity.as_ref().is_some_and(|e| reads_own_data(&e.kind)) {
        partial.retain(|p| *p != DRAFTCRAFT_DATA);
    }
    Ok((Decoded { entity, attrib, attribs, polyline, vertex, mleader_attribs, paper_view, held, leader_hook, partial, ..Decoded::default() }, refs))
}

/// An entity kept as its DXF groups names its owner, as every entity of a DXF file does.
fn name_owner(dec: &mut Decoded, record: u64) {
    if let Some(EntityKind::Unknown(u)) = dec.entity.as_mut().map(|e| &mut e.kind)
        && record != 0
    {
        let at = usize::from(u.tags.first().is_some_and(|t| t.code == 5));
        u.tags.insert(at, RawTag { code: 330, value: format!("{record:X}") });
    }
}

/// The groups an entity kept without modelling it starts with, as a DXF file has them: its
/// handle, then the properties every entity has (its owner is added once its place is known).
fn unknown_head(handle: u64, c: &Common) -> Vec<RawTag> {
    let tag = |code: i32, value: String| RawTag { code, value };
    let mut t = vec![tag(5, format!("{handle:X}")), tag(100, "AcDbEntity".into())];
    t.push(tag(8, c.layer.clone()));
    if !c.linetype.eq_ignore_ascii_case("bylayer") {
        t.push(tag(6, c.linetype.clone()));
    }
    match c.color {
        draftcraft_color::Color::ByLayer => {}
        draftcraft_color::Color::True(rgb) => {
            t.push(tag(62, draftcraft_color::nearest_aci(rgb).to_string()));
            t.push(tag(420, rgb.to_u32().to_string()));
        }
        color => t.push(tag(62, color.to_aci().to_string())),
    }
    if c.lineweight != Lineweight::ByLayer {
        t.push(tag(370, c.lineweight.to_dxf().to_string()));
    }
    if (c.ltscale - 1.0).abs() > 1e-12 {
        t.push(tag(48, draftcraft_dxf::Value::Real(c.ltscale).as_str()));
    }
    if !c.visible {
        t.push(tag(60, "1".into()));
    }
    t
}

/// The compound file an OLE object's data holds starts with these bytes; what is before them is
/// AutoCAD's own wrapper.
const COMPOUND_FILE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// The frame an OLE object's data starts with, when it has one: after a two-byte marker, the
/// frame's corners (upper left, upper right, lower right, lower left), which are the corners of
/// a rectangle along the axes. Returns the upper left and lower right ones.
fn ole_frame(data: &[u8]) -> Option<([f64; 3], [f64; 3])> {
    let end = data.windows(COMPOUND_FILE.len()).position(|w| w == COMPOUND_FILE).unwrap_or(data.len());
    let wrapper = data.get(..end)?;
    if wrapper.len() < 98 {
        return None;
    }
    let mut v = [0.0f64; 12];
    for (i, x) in v.iter_mut().enumerate() {
        *x = f64::from_le_bytes(wrapper.get(2 + i * 8..10 + i * 8)?.try_into().ok()?);
    }
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-6 * (1.0 + a.abs().max(b.abs()));
    let [ulx, uly, ulz, urx, ury, _, lrx, lry, lrz, llx, lly, _] = v;
    let rectangle = close(uly, ury) && close(lry, lly) && close(ulx, llx) && close(urx, lrx);
    (v.iter().all(|x| x.is_finite() && x.abs() < 1e15) && rectangle).then_some(([ulx, uly, ulz], [lrx, lry, lrz]))
}

/// Most bytes of ACIS text read for one solid.
const MAX_ACIS_TEXT: usize = 256 << 20;

/// 3DSOLID, REGION and BODY (§20.4.39 to 41): the solid modeler's description of the shape, as
/// text up to AutoCAD 2010. The drawing keeps the entity as the groups a DXF file has: each line
/// of the text a group 1, a line longer than 255 characters continued in groups 3. The text is
/// stored scrambled in both formats, the same way, and is kept as stored.
/// `None` for the forms not read yet: the binary description, and the files from AutoCAD 2013
/// on, which keep the description in a section of its own.
fn modeler(r: &mut ObjectReader, dxf_type: &'static str) -> DwgResult<Option<Shape>> {
    if r.version() >= V::R2013 {
        return Ok(None);
    }
    let tag = |code: i32, value: String| RawTag { code, value };
    let mut t = vec![tag(100, "AcDbModelerGeometry".into())];
    let empty = r.b()?;
    if !empty {
        let _unknown = r.b()?;
        if r.bs()? != 1 {
            return Ok(None);
        }
        let mut text: Vec<u8> = Vec::new();
        loop {
            let n = r.bl()?;
            if n == 0 {
                break;
            }
            let n = r.count(n, 8, "ACIS text byte")?;
            if text.len().saturating_add(n) > MAX_ACIS_TEXT {
                return Err(draftcraft_dwg::records::Error::format("ACIS text is implausibly long"));
            }
            text.extend(r.bytes(n)?);
        }
        t.push(tag(70, "1".into()));
        let text: String = text.iter().map(|b| char::from(*b)).collect();
        for line in text.split('\n').map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty()) {
            // A DXF string writes a caret as a caret and a space.
            let chars: Vec<char> = line.replace('^', "^ ").chars().collect();
            for (i, part) in chars.chunks(255).enumerate() {
                t.push(tag(if i == 0 { 1 } else { 3 }, part.iter().collect()));
            }
        }
    }
    Ok(Some(Shape { body: Body::Unknown(dxf_type, t), thickness: 0.0, normal: UP }))
}

/// OLE2FRAME (§20.4.74): an embedded or linked object. Its type, its space and its data, with
/// (from AutoCAD 2000) whether it keeps its proportions. The drawing keeps it as the groups a
/// DXF file has: the frame's corners come from the data's wrapper, and the data is the DXF
/// file's binary data as it is.
fn ole2frame(r: &mut ObjectReader) -> DwgResult<Shape> {
    let ver = r.version();
    let kind = r.bs()?;
    let mode = if ver >= V::R2000 { r.bs()? } else { 0 };
    let n = r.bl()?;
    let n = r.count(n, 8, "OLE data byte")?;
    let data = r.bytes(n)?;
    let lock_aspect = if ver >= V::R2000 { r.rc()? } else { 0 };
    // Without a wrapper: the corners a DXF file of such an object has.
    let (upper_left, lower_right) = ole_frame(&data).unwrap_or(([1.0, 1.0, 0.0], [0.0, 0.0, 0.0]));
    let tag = |code: i32, value: String| RawTag { code, value };
    let real = |v: f64| draftcraft_dxf::Value::Real(v).as_str();
    let mut t = vec![tag(100, "AcDbOle2Frame".into()), tag(70, "2".into())];
    for (base, p) in [(10, upper_left), (11, lower_right)] {
        t.extend([tag(base, real(p[0])), tag(base + 10, real(p[1])), tag(base + 20, real(p[2]))]);
    }
    t.extend([tag(71, kind.to_string()), tag(72, mode.to_string()), tag(73, lock_aspect.to_string())]);
    if !data.is_empty() {
        t.push(tag(90, data.len().to_string()));
        t.extend(data.chunks(127).map(|c| tag(310, c.iter().map(|b| format!("{b:02X}")).collect())));
    }
    t.push(tag(1, "OLE".into()));
    Ok(Shape { body: Body::Unknown("OLE2FRAME", t), thickness: 0.0, normal: Vec3::new(0.0, 0.0, 1.0) })
}

/// Each layout's viewports numbered in their order, the first (the sheet) 1, as a DXF file
/// numbers them (group 69): DWG keeps no number. The sheet viewports DraftCraft's writer added
/// are numbered, then taken out: the drawing leaves them implicit.
fn number_viewports(d: &mut Drawing, paper_views: &HashSet<Handle>) {
    for l in &mut d.layouts {
        let handles: Vec<Handle> = l.entities.iter().filter(|e| matches!(e.kind, EntityKind::Viewport(_))).map(|e| e.handle).collect();
        for (i, h) in handles.into_iter().enumerate() {
            let id = u32::try_from(i + 1).unwrap_or(u32::MAX);
            l.entities.modify(h, |e| {
                if let EntityKind::Viewport(v) = &mut e.kind {
                    v.id = id;
                }
            });
            if paper_views.contains(&h) {
                l.entities.remove(h);
            }
        }
    }
}

/// The tables' blocks taken out (the drawing rebuilds a table's geometry, as the DXF reader has
/// it), and a table from another writer given its style's name and text height.
fn finish_tables(d: &mut Drawing, tables: Vec<(Handle, Option<String>, Option<Option<String>>)>) {
    for (h, block, fix) in tables {
        if let Some(b) = block {
            d.blocks.remove(&b);
        }
        let Some(name) = fix else { continue };
        let height = {
            let style = name.as_deref().unwrap_or("Standard");
            d.table_styles
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(style))
                .map_or(draftcraft_doc::TableStyle::default().text_height, |s| s.text_height)
        };
        let _ = d.modify_entity(h, |e| {
            if let EntityKind::Table(t) = &mut e.kind {
                if let Some(name) = name {
                    t.style = name;
                }
                t.text_height = height;
            }
        });
    }
}

/// A leader to text ends in a hook line when its last segment is more than 15 degrees from the
/// text's horizontal (AutoCAD's documented landing rule): a segment of the arrow size (DIMASZ ×
/// DIMSCALE of its style) along the x direction, towards the end, opposite the x direction unless
/// the hook flag says along it. The DWG stores the leader without it; AutoCAD's DXF lists its
/// first point before the last vertex (group 75 set), which is how the drawing holds it. Seen in
/// AutoCAD's own sample (LEADER 512 of ACadSharp's samples, every version).
fn add_hook_lines(d: &mut Drawing, hooks: &[(Handle, bool, Vec2)]) {
    if hooks.is_empty() {
        return;
    }
    let by: HashMap<Handle, (bool, Vec2)> = hooks.iter().map(|&(h, on_x, x)| (h, (on_x, x))).collect();
    let dimscale = d.header.f64("DIMSCALE", 1.0);
    let start_of = |e: &Entity| -> Option<Vec3> {
        let (&(on_x, x), EntityKind::Leader(l)) = (by.get(&e.handle)?, &e.kind) else { return None };
        let style = d.dim_style(&l.style).cloned().unwrap_or_default();
        hook_start(&l.vertices, x, on_x, style.arrow_size * style.effective_scale(dimscale))
    };
    let spaces: Vec<(Handle, Vec3)> =
        d.model.iter().chain(d.layouts.iter().flat_map(|l| l.entities.iter())).filter_map(|e| start_of(e.as_ref()).map(|p| (e.handle, p))).collect();
    let mut in_blocks: Vec<(String, Handle, Vec3)> = Vec::new();
    for (name, b) in &d.blocks {
        in_blocks.extend(b.entities.iter().filter_map(|e| start_of(e.as_ref()).map(|p| (name.clone(), e.handle, p))));
    }
    let insert = |e: &mut Entity, p: Vec3| {
        if let EntityKind::Leader(l) = &mut e.kind {
            let at = l.vertices.len().saturating_sub(1);
            l.vertices.insert(at, p);
        }
    };
    for (h, p) in spaces {
        let _ = d.modify_entity(h, |e| insert(e, p));
    }
    for (name, h, p) in in_blocks {
        if let Some(b) = d.blocks.get_mut(&name) {
            std::sync::Arc::make_mut(b).entities.modify(h, |e| insert(e, p));
        }
    }
}

/// The first point of a leader's hook line, which goes before its last vertex: `len` from the
/// end along the text's x direction `x`, the other way unless `on_x`. `None` when the last
/// segment is within 15 degrees of that direction (it is the landing itself) or the leader is
/// degenerate.
fn hook_start(vertices: &[Vec3], x: Vec2, on_x: bool, len: f64) -> Option<Vec3> {
    let a = vertices.get(vertices.len().checked_sub(2)?)?;
    let b = vertices.last()?;
    let seg = b.xy() - a.xy();
    if !(len.is_finite() && len > 0.0 && x.is_finite() && seg.is_finite() && seg.len() > 1e-12 && x.len() > 1e-12) {
        return None;
    }
    let x = x.normalized();
    let off = seg.normalized().dot(x).clamp(-1.0, 1.0).abs().acos();
    if off <= 15f64.to_radians() + 1e-9 {
        return None;
    }
    let p = b.xy() + x * (if on_x { len } else { -len });
    Some(Vec3::new(p.x, p.y, b.z))
}

/// A tolerance frame's text as the drawing holds it: each line break `^J`, as in AutoCAD's DXF
/// (the DWG stores the character itself, which the DXF writer could only make a space).
fn frame_text(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace(['\r', '\n'], "^J").chars().take(MAX_FRAME_CHARS).collect()
}

/// The colour book names of the entities placed (`read`), kept as the DXF reader keeps them.
fn keep_color_books(d: &mut Drawing, books: Vec<(Handle, (String, String))>, read: &HashSet<u64>) {
    for (h, book) in books {
        if read.contains(&h.0) {
            std::sync::Arc::make_mut(&mut d.foreign).entities.entry(h).or_default().color_book = Some(book);
        }
    }
}

/// What decoded entities wait for, by the handle of the entity: attributes for their block
/// references, vertices for their polylines, multileaders for their blocks' attribute definitions.
#[derive(Default)]
struct Waiting {
    attribs: HashMap<u64, (Attrib, Common)>,
    lists: Vec<(u64, Attribs)>,
    vertices: HashMap<u64, Vertex>,
    polylines: Vec<(u64, PolyKind, Common, Attribs)>,
    mleader_attribs: Vec<(Handle, crate::dxf_mleader::Pending)>,
    paper_views: HashSet<Handle>,
    tables: Vec<(Handle, Option<String>, Option<Option<String>>)>,
    color_books: Vec<(Handle, (String, String))>,
    hooks: Vec<(Handle, bool, Vec2)>,
}

impl Waiting {
    /// Keep what one decoded entity waits for; returns the entity, when it is one already.
    fn take(&mut self, h: u64, dec: Decoded) -> Option<Entity> {
        if let Some(a) = dec.attrib {
            self.attribs.insert(h, a);
        }
        if let Some(list) = dec.attribs {
            self.lists.push((h, list));
        }
        if let Some(v) = dec.vertex {
            self.vertices.insert(h, v);
        }
        if let Some(attribs) = dec.mleader_attribs {
            self.mleader_attribs.push((Handle(h), crate::dxf_mleader::Pending { style: None, attribs }));
        }
        if let Some((kind, c, list)) = dec.polyline {
            self.polylines.push((h, kind, c, list));
        }
        if dec.paper_view {
            self.paper_views.insert(Handle(h));
        }
        if let Some((block, fix)) = dec.table {
            self.tables.push((Handle(h), block, fix));
        }
        if let Some(book) = dec.color_book {
            self.color_books.push((Handle(h), book));
        }
        if let Some((on_x, x)) = dec.leader_hook {
            self.hooks.push((Handle(h), on_x, x));
        }
        dec.entity
    }
}

/// Read the entities into `d`, each into the space or block that owns it, in the block's order.
/// Returns the handles of the objects read.
pub(super) fn read(
    file: &DwgFile,
    objects: &[ObjectRef],
    tables: &Tables,
    blocks: &Blocks,
    objects_named: &super::objects::Named,
    d: &mut Drawing,
    unread: &mut Unread,
) -> (HashSet<u64>, HashMap<u64, Link>) {
    let by_hex = (super::tables::by_hex(&tables.styles), super::tables::by_hex(&blocks.names), super::tables::by_hex(&tables.linetypes));
    let mut links: HashMap<u64, Link> = HashMap::new();
    let names = Names { tables, blocks: &blocks.names, objects: objects_named, by_hex };
    let mut read = HashSet::new();
    let mut decoded: HashMap<u64, Entity> = HashMap::new();
    let mut places: HashMap<u64, Place> = HashMap::new();
    let mut waiting = Waiting::default();
    // The entities after a polyline's own, from its polygon mesh's faces, by the polyline.
    let mut extras: HashMap<u64, Vec<Entity>> = HashMap::new();
    // The object after each one in the file, for a chain that leaves the next entity unsaid.
    let mut following: HashMap<u64, u64> = HashMap::new();
    let entities: Vec<&ObjectRef> = objects.iter().filter(|o| o.is_entity && !blocks.read.contains(&o.handle)).take(MAX_ENTITIES).collect();
    for pair in entities.windows(2) {
        if let [a, b] = pair {
            following.insert(a.handle, b.handle);
        }
    }
    for o in &entities {
        let Ok((mut dec, refs)) = file.read_object(o).and_then(|mut r| entity(&mut r, o, &names)) else {
            if reads(&o.type_name) {
                unread.count(&format!("{} (not decoded)", o.type_name));
            }
            continue;
        };
        if refs.xdictionary != 0 || !refs.reactors.is_empty() || refs.material != 0 {
            links.insert(o.handle, Link { xdictionary: refs.xdictionary, reactors: refs.reactors.clone(), material: refs.material });
        }
        let reopened = file.read_object(o).ok();
        let record = match reopened.as_ref().and_then(|r| r.entity()).map(|h| h.space) {
            Some(EntitySpace::Model) => blocks.model,
            Some(EntitySpace::Paper) => blocks.paper,
            _ => refs.owner,
        };
        places.insert(o.handle, Place { record, next: refs.next });
        name_owner(&mut dec, record);
        dec.color_book = reopened.as_ref().and_then(|r| color_book(r, &refs, &names));
        for what in &dec.partial {
            unread.count(what);
        }
        read.extend(dec.held.iter().copied());
        match waiting.take(o.handle, dec) {
            Some(e) => {
                decoded.insert(o.handle, e);
            }
            None if reads(&o.type_name) && o.type_name == "INSERT" => unread.count(&format!("{} of an unknown block", o.type_name)),
            None => {}
        }
    }
    // Each polyline's vertices: the ones it lists (2004 and later) or the chain from its first to
    // its last (up to 2000). A polyline whose vertices are not all read is left out whole.
    let walk = |list: &Attribs, places: &HashMap<u64, Place>| -> Vec<u64> {
        let mut handles = list.listed.clone();
        if let Some((first, last)) = list.chain {
            let mut at = first;
            while at != 0 && handles.len() < MAX_ENTITIES {
                handles.push(at);
                if at == last {
                    break;
                }
                at = places.get(&at).and_then(|p| p.next).or_else(|| following.get(&at).copied()).unwrap_or(0);
            }
        }
        handles
    };
    let Waiting { mut attribs, lists, mut vertices, polylines, mleader_attribs, paper_views, tables, color_books, hooks } = waiting;
    for (h, kind, c, list) in polylines {
        let handles = walk(&list, &places);
        let Some(found) = handles.iter().map(|v| vertices.remove(v)).collect::<Option<Vec<_>>>() else {
            unread.count("polyline whose vertices were not all read");
            continue;
        };
        let (main, more) = polyline_kinds(kind, &found, c.extrusion);
        let Some(main) = main else {
            unread.count("polyline mesh without faces");
            continue;
        };
        let mut e = Entity::new(Handle(h), main);
        e.common = c;
        let more: Vec<Entity> = more
            .into_iter()
            .map(|k| {
                let mut x = Entity::new(d.new_handle(), k);
                x.common = e.common.clone();
                x
            })
            .collect();
        decoded.insert(h, e);
        extras.insert(h, more);
        read.extend(handles);
        read.insert(list.seqend);
    }
    for _ in vertices.keys() {
        unread.count("vertex of no polyline");
    }
    // Each block reference's attributes: the ones it lists (2004 and later) or the chain from its
    // first to its last (up to 2000). A reference whose attributes are not all read is left out
    // whole, so it never loses one silently.
    for (h, list) in lists {
        let handles = walk(&list, &places);
        let Some(e) = decoded.get_mut(&h) else { continue };
        let Some(found) = handles.iter().map(|a| attribs.remove(a)).collect::<Option<Vec<_>>>() else {
            decoded.remove(&h);
            unread.count("INSERT whose attributes were not all read");
            continue;
        };
        let layer = e.common.layer.clone();
        if let EntityKind::Insert(i) = &mut e.kind {
            for (mut a, c) in found {
                a.props = attrib_props(&c, &layer);
                i.attribs.push(a);
            }
        }
        read.extend(handles);
        read.insert(list.seqend);
    }
    for _ in attribs.keys() {
        unread.count("ATTRIB of no block reference");
    }
    // Each block's entities in its own order: the list it stores (AutoCAD 2004 and later) or the
    // chain from its first entity; then any entity that names the block as its owner and was in
    // neither, in file order.
    let mut placed: HashSet<u64> = HashSet::new();
    let mut layers: HashSet<String> = d.layers.iter().map(|l| l.name.to_ascii_lowercase()).collect();
    // Each record's entities in file order, gathered once (not searched for per record: a file
    // has a block for every dimension).
    let mut by_record: HashMap<u64, Vec<u64>> = HashMap::new();
    for o in &entities {
        if let Some(p) = places.get(&o.handle) {
            by_record.entry(p.record).or_default().push(o.handle);
        }
    }
    for rec in &blocks.records {
        let mut order: Vec<u64> = rec.owned.clone();
        let mut at = rec.first;
        let mut steps = 0usize;
        while at != 0 && steps < MAX_ENTITIES {
            steps += 1;
            let Some(place) = places.get(&at).filter(|p| p.record == rec.handle) else { break };
            order.push(at);
            at = place.next.or_else(|| following.get(&at).copied()).unwrap_or(0);
        }
        let listed: HashSet<u64> = order.iter().copied().collect();
        order.extend(by_record.get(&rec.handle).into_iter().flatten().copied().filter(|h| !listed.contains(h)));
        for h in order {
            if !placed.insert(h) {
                continue;
            }
            let Some(e) = decoded.remove(&h) else { continue };
            read.insert(h);
            let more = extras.remove(&h).unwrap_or_default();
            match &rec.space {
                Space::Model => {
                    // A layer the table doesn't have is added once (the layers are many in some
                    // drawings: not a search of the list per entity).
                    if !layers.contains(&e.common.layer.to_ascii_lowercase()) {
                        d.ensure_layer(&e.common.layer);
                        layers.insert(e.common.layer.to_ascii_lowercase());
                    }
                    d.model.push(e);
                    for x in more {
                        d.model.push(x);
                    }
                }
                Space::Layout(name) => {
                    if let Some(l) = d.layouts.iter_mut().find(|l| &l.name == name) {
                        l.entities.push(e);
                        for x in more {
                            l.entities.push(x);
                        }
                    }
                }
                Space::Block(name) => {
                    if let Some(b) = d.blocks.get_mut(name) {
                        let b = std::sync::Arc::make_mut(b);
                        b.entities.push(e);
                        for x in more {
                            b.entities.push(x);
                        }
                    }
                }
                Space::Xref => {}
            }
        }
    }
    number_viewports(d, &paper_views);
    finish_tables(d, tables);
    keep_color_books(d, color_books, &read);
    add_hook_lines(d, &hooks);
    // The multileaders' block attribute values, placed like the attribute definitions of their
    // blocks, by the DXF reader's rule.
    crate::dxf_mleader::apply(d, &mleader_attribs, &HashMap::new());
    // An entity no block owns (a sub-entity of a type not read yet) stays unread.
    (read, links)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AutoCAD's round-trip data of a file saved as 2000 (CIRCLE 99F of ACadSharp's sample, saved
    /// by AutoCAD): the colour sequence is read and left out, with the material's, and the ACAD
    /// data around them is kept.
    #[test]
    fn round_trip_sequences_are_read_and_left_out() {
        let items = vec![
            XDataItem::Text("RTTcAl".into()),
            XDataItem::Int16(-16342),
            XDataItem::Int32(-1_025_339_392),
            XDataItem::Text("RAL CLASSIC$RAL 1006".into()),
            XDataItem::Text("RTMaterial".into()),
            XDataItem::Handle(0x97),
            XDataItem::Text("MTEXTBEGIN".into()),
            XDataItem::Int16(73),
        ];
        let (kept, color) = round_trip(&items);
        assert_eq!(color, Some((-16342, -1_025_339_392, "RAL CLASSIC$RAL 1006")));
        assert_eq!(kept, vec![&XDataItem::Text("MTEXTBEGIN".into()), &XDataItem::Int16(73)]);
        // 0xC02A: colour 42 of a book; 0xC2E29000: the true colour E29000.
        assert_eq!(u32::from_ne_bytes((-1_025_339_392i32).to_ne_bytes()), 0xC2E2_9000);
        assert_eq!(-16342i16 & 0x01FF, 42);
    }

    /// AutoCAD's LEADER 512 (ACadSharp's samples): the DWG stores three vertices, AutoCAD's DXF
    /// four, the hook line's first point 2.5 (DIMASZ of ISO-25) before the last, opposite the x
    /// direction (hook flag clear).
    #[test]
    fn a_leader_to_text_gets_autocads_hook_line() {
        let v = [
            Vec3::new(717.868_715_432_339_7, 2.941_179_455_067_987, 0.0),
            Vec3::new(729.268_117_078_097_2, 48.538_786_038_098_59, 0.0),
            Vec3::new(740.398_750_025_284_9, 14.071_812_402_255_56, 0.0),
        ];
        let p = hook_start(&v, Vec2::new(1.0, 0.0), false, 2.5).expect("a hook line");
        assert!((p.x - 737.898_750_025_284_9).abs() < 1e-9 && (p.y - 14.071_812_402_255_56).abs() < 1e-9, "{p:?}");
        // The hook flag set: the other way.
        assert!(hook_start(&v, Vec2::new(1.0, 0.0), true, 2.5).is_some_and(|p| (p.x - 742.898_750_025_284_9).abs() < 1e-9));
        // A last segment within 15 degrees of the text's direction is the landing itself.
        let flat = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 2.0, 0.0)];
        assert_eq!(hook_start(&flat, Vec2::new(1.0, 0.0), false, 2.5), None);
        assert_eq!(hook_start(&v[..1], Vec2::new(1.0, 0.0), false, 2.5), None);
        assert_eq!(hook_start(&v, Vec2::new(f64::NAN, 0.0), false, 2.5), None);
        assert_eq!(hook_start(&v, Vec2::new(1.0, 0.0), false, 0.0), None);
    }

    /// AutoCAD's TOLERANCE 823 stores "99", a line break and "a"; its DXF has `99^Ja`.
    #[test]
    fn a_frame_line_break_is_caret_j() {
        assert_eq!(frame_text("99\na"), "99^Ja");
        assert_eq!(frame_text("1\r\n2\r3"), "1^J2^J3");
        assert_eq!(frame_text("one line"), "one line");
    }

    /// A sequence that is not the round-trip data's shape is other data, kept as it is.
    #[test]
    fn other_acad_data_is_kept() {
        let items = vec![XDataItem::Text("RTTcAl".into()), XDataItem::Int16(1), XDataItem::Text("not a colour".into())];
        let (kept, color) = round_trip(&items);
        assert_eq!(color, None);
        assert_eq!(kept.len(), 3);
    }
}
