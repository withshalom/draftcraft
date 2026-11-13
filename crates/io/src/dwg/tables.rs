//! The symbol tables of a DWG file, into the drawing: application names, linetypes, text styles
//! and layers. Each record is decoded field by field in the order the format stores it (Open
//! Design Alliance specification, chapter 20) straight into the drawing's own type.

use std::collections::{HashMap, HashSet};

use draftcraft_color::{Color, Rgb};
use draftcraft_doc::{DashElement, DimStyle, Drawing, HVal, Layer, Linetype, Lineweight, NamedView, TextStyle, Ucs, VportConfig, VportTile};
use draftcraft_dwg::DwgVersion as V;
use draftcraft_dwg::file::{ColorValue, DwgFile, ObjectReader, ObjectRef, XDataItem};
use draftcraft_dwg::records::Result as DwgResult;
use draftcraft_dxf::Tag;
use draftcraft_geom::{Vec2, Vec3};

use super::Unread;
use crate::dxf_read::style_font_of;

/// Most records of one table that are read: more than any real drawing has.
const MAX_RECORDS: usize = 1 << 20;
/// Most elements AutoCAD allows in one linetype pattern.
const MAX_DASHES: usize = 12;

/// The names of the table records read, by handle, for the references other objects make.
#[derive(Default)]
pub(super) struct Tables {
    pub apps: HashMap<u64, String>,
    pub linetypes: HashMap<u64, String>,
    /// Text styles; an unnamed shape style is here under its shape file.
    pub styles: HashMap<u64, String>,
    pub layers: HashMap<u64, String>,
    pub dim_styles: HashMap<u64, String>,
    pub ucss: HashMap<u64, String>,
    /// The linetype table's records in its order, without ByLayer and ByBlock (the order a
    /// multiline style's linetype index counts in, before AutoCAD 2018).
    pub linetype_order: Vec<u64>,
    /// The handles of the objects this category read.
    pub read: HashSet<u64>,
    /// Each layer's plot style placeholder, named by the plot style dictionary the objects read.
    pub layer_plot_styles: Vec<(String, u64)>,
}

/// A colour of the file as the drawing's colour.
pub(super) fn color(c: ColorValue) -> Option<Color> {
    match c {
        ColorValue::ByLayer => Some(Color::ByLayer),
        ColorValue::ByBlock => Some(Color::ByBlock),
        ColorValue::Index(i) => Some(Color::from_aci(i16::from(i))),
        ColorValue::Rgb(r, g, b) => Some(Color::True(Rgb::from_u32(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)))),
        ColorValue::None => None,
    }
}

/// The fields every table record starts with: its name, then the "referenced" flag, the index of
/// the xref it came from and the "depends on an xref" flag. From AutoCAD 2007 on only the index
/// is stored.
fn record_name(r: &mut ObjectReader) -> DwgResult<String> {
    let name = r.text()?;
    if r.has_string_stream() {
        r.bs()?;
    } else {
        r.b()?;
        r.bs()?;
        r.b()?;
    }
    Ok(name)
}

/// The extended data `app` attached to the object.
fn xdata<'a>(blocks: &'a [draftcraft_dwg::file::XDataBlock], apps: &HashMap<u64, String>, app: &str) -> &'a [XDataItem] {
    blocks.iter().find(|b| apps.get(&b.app).is_some_and(|n| n.eq_ignore_ascii_case(app))).map_or(&[], |b| &b.items)
}

/// Put `item` in `list`, in place of the record of the same name (in any case) or at the end.
/// `index` holds the list's positions by lower-case name, kept as records go in: one index per
/// table, made on its first use, so a table of many records (100 000 layers) goes in at once,
/// not with a search of the list per record.
fn put<T>(list: &mut Vec<T>, index: &mut HashMap<String, usize>, item: T, name: impl Fn(&T) -> &str) {
    if index.is_empty() && !list.is_empty() {
        *index = list.iter().enumerate().map(|(i, x)| (name(x).to_ascii_lowercase(), i)).collect();
    }
    let key = name(&item).to_ascii_lowercase();
    match index.get(&key).and_then(|i| list.get_mut(*i)) {
        Some(slot) => *slot = item,
        None => {
            index.insert(key, list.len());
            list.push(item);
        }
    }
}

/// Read the tables into `d`. A record that can't be decoded is counted in `unread`.
pub(super) fn read(file: &DwgFile, objects: &[ObjectRef], d: &mut Drawing, unread: &mut Unread) -> Tables {
    let mut t = Tables::default();
    let of = |name: &'static str| objects.iter().filter(move |o| o.type_name == name).take(MAX_RECORDS);
    // Application names first: extended data names its application by this record.
    for o in of("APPID") {
        if let Ok(name) = file.read_object(o).and_then(|mut r| record_name(&mut r)) {
            // The record is its name, which the drawing keeps with each piece of extended data.
            t.apps.insert(o.handle, name);
            t.read.insert(o.handle);
        }
    }
    // Text styles before linetypes, whose text and shape elements name one.
    let mut index = HashMap::new();
    for o in of("STYLE") {
        match file.read_object(o).and_then(|mut r| style(&mut r, &t.apps)) {
            Ok((st, shape_file)) => {
                t.styles.insert(o.handle, if st.name.trim().is_empty() { shape_file } else { st.name.clone() });
                // An unnamed style is a shape file that linetypes refer to, not a text style.
                if !st.name.is_empty() {
                    put(&mut d.text_styles, &mut index, st, |x| &x.name);
                }
                t.read.insert(o.handle);
            }
            Err(_) => unread.count("STYLE (not decoded)"),
        }
    }
    for o in objects.iter().filter(|o| o.type_name == "LTYPE_CONTROL").take(1) {
        if let Ok(list) = file.read_object(o).and_then(|mut r| control_entries(&mut r)) {
            t.linetype_order = list;
        }
    }
    let mut have: HashSet<String> = d.linetypes.iter().map(|x| x.name.to_ascii_lowercase()).collect();
    for o in of("LTYPE") {
        match file.read_object(o).and_then(|mut r| linetype(&mut r, &t.styles)) {
            Ok(lt) => {
                t.linetypes.insert(o.handle, lt.name.clone());
                // A name the drawing already has (ByLayer, ByBlock, Continuous) keeps the
                // drawing's own record, as the DXF reader does.
                if !lt.name.is_empty() && have.insert(lt.name.to_ascii_lowercase()) {
                    d.linetypes.push(lt);
                }
                t.read.insert(o.handle);
            }
            Err(_) => unread.count("LTYPE (not decoded)"),
        }
    }
    let mut index = HashMap::new();
    for o in of("LAYER") {
        match file.read_object(o).and_then(|mut r| layer(&mut r, &t)) {
            Ok((l, plot_style, book)) => {
                t.layers.insert(o.handle, l.name.clone());
                let name_of_layer = l.name.clone();
                if !l.name.is_empty() {
                    if let Some(book) = book {
                        std::sync::Arc::make_mut(&mut d.foreign)
                            .records
                            .entry(("LAYER".into(), l.name.trim().to_ascii_uppercase()))
                            .or_default()
                            .color_book = Some(book);
                    }
                    put(&mut d.layers, &mut index, l, |x| &x.name);
                }
                // The plot style is named by the plot style dictionary, read with the objects.
                if plot_style != 0 {
                    t.layer_plot_styles.push((name_of_layer, plot_style));
                }
                t.read.insert(o.handle);
            }
            Err(_) => unread.count("LAYER (not decoded)"),
        }
    }
    t
}

/// STYLE (specification §20.4.56). Returns the style and its font file, which names an unnamed
/// shape style.
fn style(r: &mut ObjectReader, apps: &HashMap<u64, String>) -> DwgResult<(TextStyle, String)> {
    let name = record_name(r)?;
    // The shape file flag, then vertical: AutoCAD's files show it (ACadSharp's samples, saved by
    // AutoCAD: the unnamed styles of ltypeshp.shx and a .shx of shapes, flag 1 in AutoCAD's DXF,
    // have the first bit set). BlackCAD reads them the other way round.
    let _shape_file = r.b()?;
    let vertical = r.b()?;
    let height = r.bd()?;
    let width_factor = r.bd()?;
    let oblique = r.bd()?;
    let generation = r.rc()?;
    let _last_height = r.bd()?;
    let font_file = r.text()?;
    let big_font = r.text()?;
    let x = r.xdata();
    // The TrueType typeface AutoCAD keeps in the style's ACAD data, and DraftCraft's own font name.
    let face = xdata(&x, apps, "ACAD").iter().find_map(|v| if let XDataItem::Text(s) = v { Some(s.trim().to_string()) } else { None });
    let own = xdata(&x, apps, crate::dxf_ext::APP).windows(2).find_map(|w| match w {
        [XDataItem::Text(m), XDataItem::Text(n)] if m == "FONT" => Some(n.clone()),
        _ => None,
    });
    // Annotative: the second integer of the AnnotativeData list.
    let a = xdata(&x, apps, "AcadAnnotative");
    let start = a.iter().position(|v| matches!(v, XDataItem::Text(s) if s == "AnnotativeData"));
    let annotative = a
        .iter()
        .skip(start.map_or(a.len(), |i| i + 1))
        .filter_map(|v| if let XDataItem::Int16(i) = v { Some(*i) } else { None })
        .nth(1)
        .is_some_and(|i| i != 0);
    let ok = |v: f64, d: f64| if v.is_finite() { v } else { d };
    let st = TextStyle {
        name,
        font: style_font_of(font_file.clone(), face, own),
        big_font,
        height: ok(height, 0.0).max(0.0),
        width_factor: if width_factor.is_finite() && width_factor > 0.0 { width_factor } else { 1.0 },
        oblique: ok(oblique, 0.0),
        backwards: generation & 2 != 0,
        upside_down: generation & 4 != 0,
        vertical,
        annotative,
    };
    Ok((st, font_file))
}

/// LTYPE (specification §20.4.58): the name, the description and the pattern, whose text and
/// shape elements each name a style.
/// A table's control object (§20.4.53): its records, in table order. The ByLayer and ByBlock
/// linetypes come after these and are not among them.
fn control_entries(r: &mut ObjectReader) -> DwgResult<Vec<u64>> {
    let n = r.bl()?;
    if n > 1 << 20 {
        return Err(draftcraft_dwg::records::Error::format(format!("too many table records: {n}")));
    }
    r.common_refs()?;
    (0..n).map(|_| r.reference()).collect()
}

fn linetype(r: &mut ObjectReader, styles: &HashMap<u64, String>) -> DwgResult<Linetype> {
    let name = record_name(r)?;
    let description = r.text()?;
    let _pattern_length = r.bd()?;
    let _alignment = r.rc()?;
    let n = usize::from(r.rc()?).min(MAX_DASHES);
    // Per element: the pattern element and its flags (1 absolute rotation, 2 text, 4 shape) with
    // the shape number, or the place of its text in the text area.
    let mut elements: Vec<(DashElement, u16, u16)> = Vec::with_capacity(n);
    for _ in 0..n {
        let length = r.bd()?;
        let code = r.bs()?;
        let [x, y] = r.rd2()?;
        let scale = r.bd()?;
        let rotation = r.bd()?;
        let flags = r.bs()?;
        let mut el = DashElement::dash(if length.is_finite() { length } else { 0.0 });
        if flags & 6 != 0 {
            el.absolute = flags & 1 != 0;
            if scale.is_finite() {
                el.scale = scale;
            }
            if rotation.is_finite() {
                el.rotation = rotation;
            }
            if x.is_finite() && y.is_finite() {
                el.offset = Vec2::new(x, y);
            }
            if flags & 2 != 0 {
                el.text = Some(String::new());
            } else {
                el.shape = Some(code);
            }
        }
        elements.push((el, flags, code));
    }
    // The text of the text elements: an area of 0-ended strings, 256 bytes up to AutoCAD 2004,
    // and 512 bytes of UTF-16 from 2007 when some element is text.
    let has_text = elements.iter().any(|(_, f, _)| f & 2 != 0);
    if r.version() <= V::R2004 {
        if let Ok(area) = r.bytes(256) {
            for (el, _, at) in elements.iter_mut().filter(|(_, f, _)| f & 2 != 0) {
                let from = area.get(usize::from(*at)..).unwrap_or(&[]);
                let end = from.iter().position(|&b| b == 0).unwrap_or(from.len());
                el.text = Some(r.ansi(from.get(..end).unwrap_or(&[])));
            }
        }
    } else if has_text && let Ok(area) = r.bytes(512) {
        for (el, _, at) in elements.iter_mut().filter(|(_, f, _)| f & 2 != 0) {
            let from = area.get(usize::from(*at)..).unwrap_or(&[]);
            let units: Vec<u16> = from.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&u| u != 0).collect();
            el.text = Some(String::from_utf16_lossy(&units));
        }
    }
    r.common_refs()?;
    let _xref_block = r.reference()?;
    // One style per element; only a text or shape element uses its own.
    let mut pattern = Vec::with_capacity(elements.len());
    for (mut el, flags, _) in elements {
        let style = r.reference().unwrap_or(0);
        if flags & 6 != 0 {
            el.style = styles.get(&style).filter(|n| !n.trim().is_empty()).cloned();
        }
        pattern.push(el);
    }
    Ok(Linetype { name, description, pattern })
}

/// LAYER (specification §20.4.54). Returns the layer and the handle of its plot style.
fn layer(r: &mut ObjectReader, t: &Tables) -> DwgResult<(Layer, u64, Option<(String, String)>)> {
    let name = record_name(r)?;
    let mut l = Layer { name, ..Layer::default() };
    let before_2000 = r.version() <= V::R14;
    let mut flags = 0u16;
    if before_2000 {
        l.frozen = r.b()?;
        let _on = r.b()?;
        l.vp_freeze_new = r.b()?;
        l.locked = r.b()?;
    } else {
        flags = r.bs()?;
        l.frozen = flags & 0x01 != 0;
        l.vp_freeze_new = flags & 0x04 != 0;
        l.locked = flags & 0x08 != 0;
        l.plot = flags & 0x10 != 0;
        l.lineweight = Lineweight::from_dxf(i16::try_from(super::header::lineweight(i64::from((flags & 0x03E0) >> 5))).unwrap_or(-3));
    }
    let named = r.named_color()?;
    let (c, index) = (named.color, named.index);
    // Up to R14 a negative colour number marks a layer that is off. From AutoCAD 2000 on it is
    // the 0x02 bit of the flags: set on a layer that is off, in AutoCAD's own files.
    l.on = if before_2000 { index >= 0 } else { flags & 0x02 == 0 };
    l.color = match (c, index) {
        (ColorValue::Rgb(..), _) => color(c).unwrap_or(Color::Index(7)),
        (ColorValue::Index(i), _) => Color::Index(i.max(1)),
        // Before AutoCAD 2004 the colour is its index field.
        (_, i) if i != 0 => Color::Index(u8::try_from(i.unsigned_abs().clamp(1, 255)).unwrap_or(7)),
        _ => Color::Index(7),
    };
    let x = r.xdata();
    if let Some(pct) = xdata(&x, &t.apps, crate::dxf_ext::LAYER_TRANSPARENCY_APP)
        .iter()
        .find_map(|v| if let XDataItem::Int32(a) = v { crate::dxf_ext::transparency_from_dxf(i64::from(*a)) } else { None })
    {
        l.transparency = pct;
    }
    if let Some(desc) = xdata(&x, &t.apps, crate::dxf_ext::LAYER_DESCRIPTION_APP)
        .iter()
        .filter_map(|v| if let XDataItem::Text(s) = v { Some(s) } else { None })
        .nth(1)
    {
        l.description.clone_from(desc);
    }
    r.common_refs()?;
    let _xref_block = r.reference()?;
    let plot_style = if before_2000 { 0 } else { r.reference()? };
    if r.version() >= V::R2007 {
        let _material = r.reference()?;
    }
    let lt = r.reference()?;
    if let Some(n) = t.linetypes.get(&lt) {
        l.linetype.clone_from(n);
    }
    let book = color_book(l.color, named.name, named.book);
    Ok((l, plot_style, book))
}

/// What a colour book colour keeps for saving, as the DXF reader keeps it
/// ([`draftcraft_doc::ForeignExtra::color_book`]): the true colour (group 420) and the book and
/// colour names, `$` between (group 430: `RAL CLASSIC$RAL 1006`). `None` for a colour that is not
/// a true colour of a book.
pub(super) fn color_book(color: Color, name: Option<String>, book: Option<String>) -> Option<(String, String)> {
    let Color::True(rgb) = color else { return None };
    let name = name.filter(|n| !n.trim().is_empty())?;
    let full = match book.filter(|b| !b.trim().is_empty()) {
        Some(b) => format!("{b}${name}"),
        None => name,
    };
    Some((rgb.to_u32().to_string(), full))
}

/// A colour as an AutoCAD Color Index number (0 ByBlock, 256 ByLayer), as a dimension style holds
/// its colours. Before AutoCAD 2004 the number is the colour's index field.
fn aci((c, index): (ColorValue, i16)) -> i64 {
    match c {
        ColorValue::ByBlock => 0,
        ColorValue::Index(i) => i64::from(i),
        ColorValue::Rgb(r, g, b) => i64::from(draftcraft_color::nearest_aci(Rgb::from_u32(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)))),
        ColorValue::ByLayer | ColorValue::None if index != 0 => i64::from(index.unsigned_abs().min(256)),
        ColorValue::ByLayer | ColorValue::None => 256,
    }
}

/// The names of records by handle as the DXF reader's tables hold them: by the handle in
/// hexadecimal.
pub(super) fn by_hex(names: &HashMap<u64, String>) -> HashMap<String, String> {
    names.iter().map(|(h, n)| (format!("{h:X}"), n.clone())).collect()
}

/// The second group of tables: dimension styles, views, UCSs and viewport configurations.
/// `blocks` names the block records, for a dimension style's arrowheads.
pub(super) fn read_more(file: &DwgFile, objects: &[ObjectRef], blocks: &HashMap<u64, String>, t: &mut Tables, d: &mut Drawing, unread: &mut Unread) {
    let of = |name: &'static str| objects.iter().filter(move |o| o.type_name == name).take(MAX_RECORDS);
    let (styles, mut arrows, ltypes) = (by_hex(&t.styles), by_hex(blocks), by_hex(&t.linetypes));
    let mut index = HashMap::new();
    for o in of("DIMSTYLE") {
        match file.read_object(o).and_then(|mut r| dim_style(&mut r, &t.apps, &styles, &mut arrows, &ltypes)) {
            Ok(st) => {
                t.dim_styles.insert(o.handle, st.name.clone());
                if !st.name.is_empty() {
                    put(&mut d.dim_styles, &mut index, st, |x| &x.name);
                }
                t.read.insert(o.handle);
            }
            Err(_) => unread.count("DIMSTYLE (not decoded)"),
        }
    }
    let mut index = HashMap::new();
    for o in of("UCS") {
        match file.read_object(o).and_then(|mut r| ucs(&mut r)) {
            Ok(u) => {
                t.ucss.insert(o.handle, u.name.clone());
                let finite = [u.origin, u.x_axis, u.y_axis].iter().all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite());
                if !u.name.is_empty() && finite {
                    put(&mut d.ucss, &mut index, u, |x| &x.name);
                }
                t.read.insert(o.handle);
            }
            Err(_) => unread.count("UCS (not decoded)"),
        }
    }
    let mut index = HashMap::new();
    for o in of("VIEW") {
        match file.read_object(o).and_then(|mut r| view(&mut r, &t.apps)) {
            Ok(v) => {
                if let Some(v) = v {
                    put(&mut d.views, &mut index, v, |x| &x.name);
                }
                t.read.insert(o.handle);
            }
            Err(_) => unread.count("VIEW (not decoded)"),
        }
    }
    let mut active = false;
    for o in of("VPORT") {
        match file.read_object(o).and_then(|mut r| vport(&mut r)) {
            Ok(v) => {
                // The first *Active viewport holds the current snap and grid settings.
                if !active && v.name.eq_ignore_ascii_case(VportConfig::ACTIVE) {
                    active = true;
                    if v.snap_spacing.is_finite() && v.snap_spacing.x > 0.0 && v.snap_spacing.y > 0.0 {
                        d.header.set("SNAPUNIT", HVal::Point(v.snap_spacing.to3(0.0)));
                    }
                    if v.grid_spacing.is_finite() && v.grid_spacing.x >= 0.0 && v.grid_spacing.y >= 0.0 {
                        d.header.set("GRIDUNIT", HVal::Point(v.grid_spacing.to3(0.0)));
                    }
                    if v.snap_base.is_finite() {
                        d.header.set("SNAPBASE", HVal::Point(v.snap_base.to3(0.0)));
                    }
                    if v.snap_rotation.is_finite() {
                        d.header.set_f64("SNAPANG", v.snap_rotation.to_degrees());
                    }
                }
                if let Some(tile) = v.tile {
                    let name = if v.name.eq_ignore_ascii_case(VportConfig::ACTIVE) { VportConfig::ACTIVE.to_string() } else { v.name };
                    match d.vports.iter_mut().find(|c| c.name.eq_ignore_ascii_case(&name)) {
                        Some(c) if c.tiles.len() < MAX_VPORT_TILES => c.tiles.push(tile),
                        Some(_) => {}
                        None => d.vports.push(VportConfig { name, tiles: vec![tile] }),
                    }
                }
                t.read.insert(o.handle);
            }
            Err(_) => unread.count("VPORT (not decoded)"),
        }
    }
    // One *Active viewport is no tiling: the view is the editor's, kept only for its twist.
    d.vports.retain(|c| c.tiles.len() > usize::from(c.is_active()) || c.tiles.first().is_some_and(|t| t.twist != 0.0));
}

/// Most tiles a viewport configuration keeps.
const MAX_VPORT_TILES: usize = 64;

/// DIMSTYLE (specification §20.4.68). Each variable is set in the drawing's style through its DXF
/// group code, by the same table the DXF reader uses, so a variable means the same from either
/// format.
fn dim_style(
    r: &mut ObjectReader,
    apps: &HashMap<u64, String>,
    styles: &HashMap<String, String>,
    arrows: &mut HashMap<String, String>,
    ltypes: &HashMap<String, String>,
) -> DwgResult<DimStyle> {
    let name = record_name(r)?;
    let ver = r.version();
    let old = ver <= V::R14;
    let mut tags: Vec<Tag> = Vec::with_capacity(80);
    let int = |tags: &mut Vec<Tag>, code: i32, v: i64| tags.push(Tag::i(code, v));
    let real = |tags: &mut Vec<Tag>, code: i32, v: f64| tags.push(Tag::f(code, v));
    if old {
        for code in [71, 72, 73, 74, 75, 76, 170, 172, 173, 174, 175] {
            let v = r.b()?;
            int(&mut tags, code, i64::from(v));
        }
        let altd = r.rc()?;
        int(&mut tags, 171, i64::from(altd));
        let zin = r.rc()?;
        int(&mut tags, 78, i64::from(zin));
        for code in [281, 282] {
            let v = r.b()?;
            int(&mut tags, code, i64::from(v));
        }
        let tolj = r.rc()?;
        int(&mut tags, 283, i64::from(tolj));
        let just = r.rc()?;
        int(&mut tags, 280, i64::from(just));
        let fit = r.rc()?;
        int(&mut tags, 287, i64::from(fit));
        let upt = r.b()?;
        int(&mut tags, 288, i64::from(upt));
        for code in [284, 285, 286, 77] {
            let v = r.rc()?;
            int(&mut tags, code, i64::from(v));
        }
        // DIMUNIT, DIMAUNIT, DIMDEC, DIMTDEC, DIMALTU, DIMALTTD.
        for code in [270, 275, 271, 272, 273, 274] {
            let v = r.bs()?;
            int(&mut tags, code, i64::from(v));
        }
    } else {
        let post = r.text()?;
        tags.push(Tag::s(3, post));
        let apost = r.text()?;
        tags.push(Tag::s(4, apost));
    }
    for code in [40, 41, 42, 43, 44, 45, 46, 47, 48] {
        let v = r.bd()?;
        real(&mut tags, code, v);
    }
    if ver >= V::R2007 {
        let fxl = r.bd()?;
        real(&mut tags, 49, fxl);
        let jogang = r.bd()?;
        real(&mut tags, 50, jogang);
        let tfill = r.bs()?;
        int(&mut tags, 69, i64::from(tfill));
        let c = r.color()?;
        int(&mut tags, 70, aci(c));
    }
    if !old {
        for code in [71, 72, 73, 74, 75, 76] {
            let v = r.b()?;
            int(&mut tags, code, i64::from(v));
        }
        for code in [77, 78, 79] {
            let v = r.bs()?;
            int(&mut tags, code, i64::from(v));
        }
        if ver >= V::R2007 {
            let arcsym = r.bs()?;
            int(&mut tags, 90, i64::from(arcsym));
        }
    }
    for code in [140, 141, 142, 143, 144, 145, 146, 147] {
        let v = r.bd()?;
        real(&mut tags, code, v);
    }
    // Before AutoCAD 2000 the arrowhead blocks are named, not referred to.
    let mut named = |tags: &mut Vec<Tag>, code: i32, name: String| {
        if !name.is_empty() {
            let key = format!("@{name}");
            arrows.insert(key.clone(), name);
            tags.push(Tag::s(code, key));
        }
    };
    if old {
        let post = r.text()?;
        tags.push(Tag::s(3, post));
        let apost = r.text()?;
        tags.push(Tag::s(4, apost));
        for code in [342, 343, 344] {
            let n = r.text()?;
            named(&mut tags, code, n);
        }
    } else {
        let altrnd = r.bd()?;
        real(&mut tags, 148, altrnd);
        let alt = r.b()?;
        int(&mut tags, 170, i64::from(alt));
        let altd = r.bs()?;
        int(&mut tags, 171, i64::from(altd));
        for code in [172, 173, 174, 175] {
            let v = r.b()?;
            int(&mut tags, code, i64::from(v));
        }
    }
    for code in [176, 177, 178] {
        let c = r.color()?;
        int(&mut tags, code, aci(c));
    }
    let mut newer = Vec::new();
    if !old {
        for code in [179, 271, 272, 273, 274, 275, 276, 277, 278, 279, 280] {
            let v = r.bs()?;
            int(&mut tags, code, i64::from(v));
        }
        for code in [281, 282] {
            let v = r.b()?;
            int(&mut tags, code, i64::from(v));
        }
        for code in [283, 284, 285, 286] {
            let v = r.bs()?;
            int(&mut tags, code, i64::from(v));
        }
        let upt = r.b()?;
        int(&mut tags, 288, i64::from(upt));
        let fit = r.bs()?;
        int(&mut tags, 289, i64::from(fit));
        if ver >= V::R2007 {
            let fxlon = r.b()?;
            int(&mut tags, 290, i64::from(fxlon));
        }
        if ver >= V::R2010 {
            let txtdirection = r.b()?;
            newer.push(Tag::i(294, i64::from(txtdirection)));
            let altmzf = r.bd()?;
            newer.push(Tag::f(295, altmzf));
            let altmzs = r.text()?;
            newer.push(Tag::s(296, altmzs));
            let mzf = r.bd()?;
            newer.push(Tag::f(297, mzf));
            let mzs = r.text()?;
            newer.push(Tag::s(298, mzs));
        }
        for code in [371, 372] {
            let v = r.bs()?;
            int(&mut tags, code, i64::from(v as i16));
        }
    }
    let _flag = r.b();
    let x = r.xdata();
    let a = xdata(&x, apps, "AcadAnnotative");
    let start = a.iter().position(|v| matches!(v, XDataItem::Text(s) if s == "AnnotativeData"));
    let annotative = a
        .iter()
        .skip(start.map_or(a.len(), |i| i + 1))
        .filter_map(|v| if let XDataItem::Int16(i) = v { Some(*i) } else { None })
        .nth(1)
        .is_some_and(|i| i != 0);
    r.common_refs()?;
    let _xref_block = r.reference()?;
    let mut href = |tags: &mut Vec<Tag>, code: i32| -> DwgResult<()> {
        let h = r.reference()?;
        tags.push(Tag::s(code, format!("{h:X}")));
        Ok(())
    };
    href(&mut tags, 340)?;
    if !old {
        for code in [341, 342, 343, 344] {
            href(&mut tags, code)?;
        }
        if ver >= V::R2007 {
            for code in [345, 346, 347] {
                href(&mut tags, code)?;
            }
        }
    }
    tags.extend(newer);
    let mut fields = serde_json::Map::new();
    for tag in &tags {
        let Some((field, kind)) = crate::dxf_ext::dim_field(tag.code) else { continue };
        if let Some(v) = crate::dxf_ext::decode(kind, tag, styles, arrows, ltypes) {
            fields.insert(field.to_string(), v);
        }
    }
    // The variables newer than the file's version, which DraftCraft keeps in its extended data
    // for older files: as the DXF reader takes them, after the style's own fields.
    fields.extend(crate::dxf_ext::read_newer_dimvars(&super::entities::xdata_tags(&x, apps), styles, arrows, ltypes));
    let mut st = DimStyle { name, annotative, ..DimStyle::default() };
    st.apply_fields(&fields);
    Ok(st)
}

/// UCS (specification §20.4.62).
fn ucs(r: &mut ObjectReader) -> DwgResult<Ucs> {
    let name = record_name(r)?;
    let v = |p: [f64; 3]| Vec3::new(p[0], p[1], p[2]);
    let origin = v(r.bd3()?);
    let x_axis = v(r.bd3()?);
    let y_axis = v(r.bd3()?);
    Ok(Ucs { name, origin, x_axis, y_axis })
}

/// VIEW (specification §20.4.60). `None` for a view the drawing can't hold (no size).
fn view(r: &mut ObjectReader, apps: &HashMap<u64, String>) -> DwgResult<Option<NamedView>> {
    let name = record_name(r)?;
    let ver = r.version();
    let height = r.bd()?;
    let width = r.bd()?;
    let [cx, cy] = r.rd2()?;
    let target = r.bd3()?;
    let _direction = r.bd3()?;
    let twist = r.bd()?;
    let _lens = r.bd()?;
    let _front = r.bd()?;
    let _back = r.bd()?;
    for _ in 0..4 {
        r.b()?;
    }
    if ver >= V::R2000 {
        r.rc()?;
    }
    if ver >= V::R2007 {
        r.b()?;
        r.rc()?;
        r.bd()?;
        r.bd()?;
        r.color()?;
    }
    let paper = r.b()?;
    let x = r.xdata();
    let own = xdata(&x, apps, crate::dxf_ext::APP);
    let field = |key: &str| {
        own.windows(2).find_map(|w| match w {
            [XDataItem::Text(k), XDataItem::Text(v)] if k == key => Some(v.clone()),
            _ => None,
        })
    };
    let twist = if twist.is_finite() { draftcraft_geom::norm_angle(twist) } else { 0.0 };
    let center = crate::dxf_ext::dcs_center_to_model(Vec2::new(cx, cy), twist, Vec2::new(target[0], target[1]));
    let ok =
        !name.is_empty() && height.is_finite() && width.is_finite() && height > 0.0 && width > 0.0 && center.x.is_finite() && center.y.is_finite();
    Ok(ok.then(|| NamedView {
        name,
        center,
        height,
        width,
        layer_state: field("LAYERSTATE"),
        twist,
        layout: paper.then(|| field("LAYOUT").unwrap_or_default()),
    }))
}

/// What a VPORT record gives the drawing.
struct Vport {
    name: String,
    tile: Option<VportTile>,
    snap_spacing: Vec2,
    grid_spacing: Vec2,
    snap_base: Vec2,
    snap_rotation: f64,
}

/// VPORT (specification §20.4.64): one tile of the viewport configuration it names.
fn vport(r: &mut ObjectReader) -> DwgResult<Vport> {
    let name = record_name(r)?.trim().to_string();
    let ver = r.version();
    let height = r.bd()?;
    // The file holds the view's width: the aspect ratio times the height (AutoCAD's *Active
    // viewport: 2988.21 for a height of 1100.22, where its DXF has the ratio 2.716).
    let width = r.bd()?;
    let aspect = if height.is_finite() && height > 0.0 { width / height } else { 0.0 };
    let [cx, cy] = r.rd2()?;
    let target = r.bd3()?;
    let _direction = r.bd3()?;
    let twist = r.bd()?;
    let _lens = r.bd()?;
    let _front = r.bd()?;
    let _back = r.bd()?;
    for _ in 0..4 {
        r.b()?;
    }
    if ver >= V::R2000 {
        r.rc()?;
    }
    if ver >= V::R2007 {
        r.b()?;
        r.rc()?;
        r.bd()?;
        r.bd()?;
        r.color()?;
    }
    let p = |[x, y]: [f64; 2]| Vec2::new(x, y);
    let min = p(r.rd2()?);
    let max = p(r.rd2()?);
    let _ucsfollow = r.b()?;
    let _circle_zoom = r.bs()?;
    let _fast_zoom = r.b()?;
    r.bb()?;
    let _grid_on = r.b()?;
    let grid_spacing = p(r.rd2()?);
    let _snap_on = r.b()?;
    let _snap_style = r.b()?;
    let _isopair = r.bs()?;
    let snap_rotation = r.bd()?;
    let snap_base = p(r.rd2()?);
    let snap_spacing = p(r.rd2()?);
    let twist = if twist.is_finite() { draftcraft_geom::norm_angle(twist) } else { 0.0 };
    let center = crate::dxf_ext::dcs_center_to_model(Vec2::new(cx, cy), twist, Vec2::new(target[0], target[1]));
    let unit = |v: f64| v.is_finite() && (-1e-9..=1.0 + 1e-9).contains(&v);
    let ok = !name.is_empty()
        && [min.x, min.y, max.x, max.y].into_iter().all(unit)
        && max.x > min.x + 1e-6
        && max.y > min.y + 1e-6
        && center.is_finite()
        && height.is_finite()
        && height > 0.0;
    let clamp = |p: Vec2| Vec2::new(p.x.clamp(0.0, 1.0), p.y.clamp(0.0, 1.0));
    let tile = ok.then(|| VportTile {
        min: clamp(min),
        max: clamp(max),
        center,
        height,
        aspect: if aspect.is_finite() && aspect > 0.0 { aspect } else { 0.0 },
        twist,
    });
    Ok(Vport { name, tile, snap_spacing, grid_spacing, snap_base, snap_rotation })
}
