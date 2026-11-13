//! The objects of a DWG file that DraftCraft reads into the drawing's own model. So far: the
//! DIMASSOC objects that make a dimension follow the geometry it measures.

use std::collections::{HashMap, HashSet};

use draftcraft_doc::Drawing;
use draftcraft_dwg::DwgVersion as V;
use draftcraft_dwg::file::{DwgFile, ObjectReader, ObjectRef};
use draftcraft_dwg::records::Result as DwgResult;
use draftcraft_dxf::Tag;

use super::Unread;
use super::tables::Tables;

/// Most references one DIMASSOC point names.
const MAX_REFS: u32 = 64;

/// Most point references one DIMASSOC holds.
const MAX_POINT_REFS: usize = 16;

/// DIMASSOC (the specification does not describe it; its layout is the one AutoCAD's files and
/// acadrust's agree on), as the DXF groups it is in a DXF file: the dimension it belongs to,
/// which of its points are associated, and for each point its references, each with its object
/// snap, the objects it names (with their subentity when it snaps to one), where it snapped, the
/// objects an intersection snap names too, and whether another reference for the same point
/// follows.
fn dimassoc(r: &mut ObjectReader) -> DwgResult<Vec<Tag>> {
    r.common_refs()?;
    let flags = r.bl()?;
    let trans = r.b()?;
    let rotated = r.rc()?;
    let dimension = r.reference()?;
    let mut tags = vec![
        Tag::s(100, "AcDbDimAssoc"),
        Tag::s(330, format!("{dimension:X}")),
        Tag::i(90, i64::from(flags)),
        Tag::i(70, i64::from(trans)),
        Tag::i(71, i64::from(rotated)),
    ];
    let objects = |r: &mut ObjectReader| -> DwgResult<Vec<u64>> {
        let n = r.bl()?;
        if n > MAX_REFS {
            return Err(draftcraft_dwg::records::Error::format("DIMASSOC: object count"));
        }
        (0..n).map(|_| r.reference()).collect()
    };
    // A subentity: its type, marker and the external reference paths to it.
    let subentity = |r: &mut ObjectReader| -> DwgResult<(u32, u32)> {
        let (kind, marker) = (r.bl()?, r.bl()?);
        let n = r.bl()?;
        if n > MAX_REFS {
            return Err(draftcraft_dwg::records::Error::format("DIMASSOC: path count"));
        }
        for _ in 0..n {
            let _path = r.text()?;
        }
        Ok((kind, marker))
    };
    let mut refs = 0usize;
    for bit in 0..4 {
        if flags & (1 << bit) == 0 {
            continue;
        }
        loop {
            if refs >= MAX_POINT_REFS {
                return Err(draftcraft_dwg::records::Error::format("DIMASSOC: too many point references"));
            }
            refs += 1;
            let class = r.text()?;
            let osnap = r.rc()?;
            let mains = objects(r)?;
            let (kind, marker) = if osnap != 0 && !mains.is_empty() { subentity(r)? } else { (0, 0) };
            let param = r.bd()?;
            let [x, y, z] = r.bd3()?;
            let mut others = Vec::new();
            let mut other_sub = None;
            if matches!(osnap, 6 | 11) {
                others = objects(r)?;
                if !others.is_empty() {
                    other_sub = Some(subentity(r)?);
                }
            }
            let more = r.b()?;
            tags.push(Tag::s(1, class));
            tags.push(Tag::i(72, i64::from(osnap)));
            tags.extend(mains.into_iter().map(|m| Tag::s(331, format!("{m:X}"))));
            tags.push(Tag::i(73, i64::from(kind)));
            tags.push(Tag::i(91, i64::from(marker)));
            tags.push(Tag::f(40, param));
            tags.extend([Tag::f(10, x), Tag::f(20, y), Tag::f(30, z)]);
            tags.extend(others.into_iter().map(|o| Tag::s(332, format!("{o:X}"))));
            if let Some((kind, marker)) = other_sub {
                tags.push(Tag::i(74, i64::from(kind)));
                tags.push(Tag::i(92, i64::from(marker)));
            }
            tags.push(Tag::i(75, i64::from(more)));
            if !more {
                break;
            }
        }
    }
    Ok(tags)
}

/// Most entries of one dictionary that are read.
const MAX_ENTRIES: u32 = 1 << 20;

/// DICTIONARY (§20.4.56): its entries, by name, each with the handle of the object it holds.
fn dictionary(r: &mut ObjectReader) -> DwgResult<Vec<(String, u64)>> {
    r.common_refs()?;
    let n = r.bl()?;
    // From AutoCAD 2007 on the names are in the string stream: an entry may take no data bits.
    let min_bits = if r.has_string_stream() { 0 } else { 2 };
    if n > MAX_ENTRIES {
        return Err(draftcraft_dwg::records::Error::format(format!("too many dictionary entries: {n}")));
    }
    let n = r.count(n, min_bits, "dictionary entry")?;
    if r.version() == draftcraft_dwg::DwgVersion::R14 {
        let _unknown = r.rc()?;
    } else if r.version() >= draftcraft_dwg::DwgVersion::R2000 {
        let _cloning = r.bs()?;
        let _hard_owner = r.rc()?;
    }
    let mut names = Vec::with_capacity(n);
    for _ in 0..n {
        names.push(r.text()?);
    }
    names.into_iter().map(|name| Ok((name, r.reference()?))).collect()
}

/// The entries of the dictionary named `key` in the dictionary at `root` (the named-object
/// dictionary): each object's handle, with its name. Empty when either can't be read.
pub(super) fn named_entries(file: &DwgFile, objects: &[ObjectRef], root: u64, key: &str) -> HashMap<u64, String> {
    let Some((_, inner)) = entries(file, objects, root).into_iter().find(|(name, _)| name.eq_ignore_ascii_case(key)) else {
        return HashMap::new();
    };
    entries_by_handle(file, objects, inner)
}

/// The entries of the dictionary at `handle`, by name; empty when it can't be read.
fn entries(file: &DwgFile, objects: &[ObjectRef], handle: u64) -> Vec<(String, u64)> {
    objects
        .iter()
        .find(|o| o.handle == handle && o.type_name == "DICTIONARY")
        .and_then(|o| file.read_object(o).and_then(|mut r| dictionary(&mut r)).ok())
        .unwrap_or_default()
}

/// The names the dictionary at `handle` gives its objects, by the object's handle.
pub(super) fn entries_by_handle(file: &DwgFile, objects: &[ObjectRef], handle: u64) -> HashMap<u64, String> {
    entries(file, objects, handle).into_iter().map(|(name, h)| (h, name)).collect()
}

/// What entities need from the file's objects: names that dictionaries give objects, and the
/// definitions images and underlays point to, by the object's handle.
#[derive(Default)]
pub(super) struct Named {
    /// Multileader style names (the ACAD_MLEADERSTYLE dictionary).
    pub mleader_styles: HashMap<u64, String>,
    /// Table style names, by TABLESTYLE handle (the ACAD_TABLESTYLE dictionary).
    pub table_styles: HashMap<u64, String>,
    /// Multiline style names, by MLINESTYLE handle (the name is in the object itself).
    pub mline_styles: HashMap<u64, String>,
    pub images: HashMap<u64, ImageDef>,
    pub pdfs: HashMap<u64, PdfDef>,
    /// Colour book colours (DBCOLOR), which entities with such a colour point to.
    pub colors: HashMap<u64, BookColor>,
}

/// A DBCOLOR object: the colour of a colour book, with what saving keeps of it
/// ([`super::tables::color_book`]).
pub(super) struct BookColor {
    pub color: draftcraft_color::Color,
    pub book: Option<(String, String)>,
}

/// DBCOLOR: one CMC colour, with its name and its book's (AutoCAD's own files store nothing else).
fn book_color(r: &mut ObjectReader) -> DwgResult<Option<BookColor>> {
    r.common_refs()?;
    let c = r.named_color()?;
    let Some(color) = super::tables::color(c.color) else { return Ok(None) };
    Ok(Some(BookColor { color, book: super::tables::color_book(color, c.name, c.book) }))
}

/// An IMAGEDEF: the image file, its name (the ACAD_IMAGE_DICT dictionary), the size of one
/// pixel when the file says, and the resolution unit.
pub(super) struct ImageDef {
    pub path: String,
    pub name: String,
    pub pixel_size: Option<draftcraft_geom::Vec2>,
    pub units: u8,
}

/// A PDFDEFINITION: the PDF file, the page, and its name (the ACAD_PDFDEFINITIONS dictionary).
pub(super) struct PdfDef {
    pub path: String,
    pub page: String,
    pub name: String,
}

/// IMAGEDEF (§20.4.81): class version, size in pixels, file, loaded, resolution unit, pixel size.
fn image_def(r: &mut ObjectReader) -> DwgResult<ImageDef> {
    r.common_refs()?;
    let _class_version = r.bl()?;
    let _size = r.rd2()?;
    let path = r.text()?;
    let _loaded = r.b()?;
    let units = r.rc()?;
    let [x, y] = r.rd2()?;
    let pixel_size = (x.is_finite() && y.is_finite() && x > 0.0 && y > 0.0).then(|| draftcraft_geom::Vec2::new(x, y));
    Ok(ImageDef { path, name: String::new(), pixel_size, units })
}

/// PDFDEFINITION (an underlay definition): the file and the page.
fn pdf_def(r: &mut ObjectReader) -> DwgResult<PdfDef> {
    r.common_refs()?;
    Ok(PdfDef { path: r.text()?, page: r.text()?, name: String::new() })
}

/// The names and definitions entities need, from the named-object dictionary at `root`.
pub(super) fn named(file: &DwgFile, objects: &[ObjectRef], root: u64) -> Named {
    let image_names = named_entries(file, objects, root, "ACAD_IMAGE_DICT");
    let pdf_names = named_entries(file, objects, root, "ACAD_PDFDEFINITIONS");
    let mut n = Named {
        mleader_styles: named_entries(file, objects, root, "ACAD_MLEADERSTYLE"),
        table_styles: named_entries(file, objects, root, "ACAD_TABLESTYLE"),
        ..Named::default()
    };
    for o in objects {
        match o.type_name.as_str() {
            "IMAGEDEF" => {
                if let Ok(mut def) = file.read_object(o).and_then(|mut r| image_def(&mut r)) {
                    def.name = image_names.get(&o.handle).cloned().unwrap_or_default();
                    n.images.insert(o.handle, def);
                }
            }
            "DBCOLOR" => {
                if let Ok(Some(c)) = file.read_object(o).and_then(|mut r| book_color(&mut r)) {
                    n.colors.insert(o.handle, c);
                }
            }
            "MLINESTYLE" => {
                if let Ok(name) = file.read_object(o).and_then(|mut r| {
                    r.common_refs()?;
                    r.text()
                }) {
                    n.mline_styles.insert(o.handle, name);
                }
            }
            "PDFDEFINITION" => {
                if let Ok(mut def) = file.read_object(o).and_then(|mut r| pdf_def(&mut r)) {
                    def.name = pdf_names.get(&o.handle).cloned().unwrap_or_default();
                    n.pdfs.insert(o.handle, def);
                }
            }
            _ => {}
        }
    }
    n
}

/// MLEADERSTYLE: its leader, landing, arrow, text and block settings, in the order AutoCAD's own
/// files hold them (ACadSharp's samples: every version reads to the end of the record, with the
/// values of AutoCAD's DXF). From 2010 on a version comes first and the attachment directions
/// last; 2013 adds a flag. The drawing keeps what the DXF reader keeps: the arrow size, text
/// height, landing gap, dogleg length, text style and whether it is annotative.
fn mleader_style(r: &mut ObjectReader, name: &str, tables: &Tables) -> DwgResult<draftcraft_doc::MLeaderStyle> {
    use draftcraft_doc::MLeaderStyle;
    let def = MLeaderStyle::default();
    let ver = r.version();
    r.common_refs()?;
    if ver >= V::R2010 {
        let _version = r.bs()?;
    }
    let _content = (r.bs()?, r.bs()?, r.bs()?);
    let _max_points = r.bl()?;
    let _angles = (r.bd()?, r.bd()?);
    let _leader_type = r.bs()?;
    let _line_color = r.color()?;
    let _line_type = r.reference()?;
    let _line_weight = r.bl()?;
    let _landing = r.b()?;
    let landing_gap = r.bd()?;
    let _dogleg = r.b()?;
    let dogleg = r.bd()?;
    let _description = r.text()?;
    let _arrow_block = r.reference()?;
    let arrow_size = r.bd()?;
    let _text_default = r.text()?;
    let text_style = r.reference()?;
    let _attachments = (r.bs()?, r.bs()?, r.bs()?, r.bs()?);
    let _text_color = r.color()?;
    let text_height = r.bd()?;
    let _frame = r.b()?;
    let _align_left = r.b()?;
    let _align_space = r.bd()?;
    let _block = r.reference()?;
    let _block_color = r.color()?;
    let _block_scale = r.bd3()?;
    let _block_scale_on = r.b()?;
    let _block_rotation = (r.bd()?, r.b()?);
    let _block_connection = r.bs()?;
    let _scale = r.bd()?;
    let annotative = r.b()?;
    let _changed = r.b()?;
    let _break_size = r.bd()?;
    if ver >= V::R2010 {
        let _attachment = (r.bs()?, r.bs()?, r.bs()?);
    }
    if ver >= V::R2013 {
        let _text_extended = r.b()?;
    }
    let num = |v: f64, d: f64| if v.is_finite() && v >= 0.0 { v } else { d };
    Ok(MLeaderStyle {
        name: name.to_string(),
        arrow_size: num(arrow_size, def.arrow_size),
        text_height: if text_height.is_finite() && text_height > 0.0 { text_height } else { def.text_height },
        landing_gap: num(landing_gap, def.landing_gap),
        dogleg: num(dogleg, def.dogleg),
        text_style: tables.styles.get(&text_style).filter(|n| !n.is_empty()).cloned().unwrap_or(def.text_style),
        annotative,
    })
}

/// MLINESTYLE (§20.4.73): name, description, flags, fill colour, cap angles and elements, each
/// with its offset, colour and linetype (a handle from AutoCAD 2018 on, before that an index into
/// the linetype table's order, 0x7FFF ByLayer and 0x7FFE ByBlock).
fn mline_style(r: &mut ObjectReader, tables: &Tables) -> DwgResult<Option<draftcraft_doc::MLineStyle>> {
    use draftcraft_doc::{MAX_MLINE_ELEMENTS, MLineElement, MLineStyle};
    r.common_refs()?;
    let mut st = MLineStyle { elements: Vec::new(), ..MLineStyle::default() };
    st.name = r.text()?;
    st.description = r.text()?;
    let flags = r.bs()?;
    let fill = super::tables::color(r.color()?.0).unwrap_or(draftcraft_color::Color::ByLayer);
    let angle = |a: f64| if a.is_finite() { a } else { std::f64::consts::FRAC_PI_2 };
    st.start_caps.angle = angle(r.bd()?);
    st.end_caps.angle = angle(r.bd()?);
    let n = usize::from(r.rc()?);
    let by_handle = r.version() >= draftcraft_dwg::DwgVersion::R2018;
    let mut linetypes = Vec::new();
    for _ in 0..n {
        let offset = r.bd()?;
        let color = super::tables::color(r.color()?.0).unwrap_or(draftcraft_color::Color::ByLayer);
        let linetype = if by_handle {
            None
        } else {
            Some(match r.bs()? {
                0x7FFF => "ByLayer".to_string(),
                0x7FFE => "ByBlock".to_string(),
                i => tables.linetype_order.get(usize::from(i)).and_then(|h| tables.linetypes.get(h)).cloned().unwrap_or_else(|| "ByLayer".into()),
            })
        };
        linetypes.push(linetype);
        if st.elements.len() < MAX_MLINE_ELEMENTS {
            st.elements.push(MLineElement { offset: if offset.is_finite() { offset } else { 0.0 }, color, linetype: String::new() });
        }
    }
    for (e, lt) in st.elements.iter_mut().zip(linetypes) {
        e.linetype = match lt {
            Some(name) => name,
            None => {
                let h = r.reference()?;
                tables.linetypes.get(&h).cloned().unwrap_or_else(|| "ByLayer".into())
            }
        };
    }
    Ok(crate::dxf_mline::style_from(st, i64::from(flags), fill))
}

/// Read the objects into `d`. Returns the handles of the objects read.
pub(super) fn read(file: &DwgFile, objects: &[ObjectRef], tables: &Tables, named: &Named, d: &mut Drawing, unread: &mut Unread) -> HashSet<u64> {
    let mut read = HashSet::new();
    // Multileader styles, named by the ACAD_MLEADERSTYLE dictionary, by the DXF reader's rule: a
    // name the drawing has is replaced.
    for o in objects.iter().filter(|o| o.type_name == "MLEADERSTYLE") {
        let Some(name) = named.mleader_styles.get(&o.handle).filter(|n| !n.is_empty()) else {
            unread.count("MLEADERSTYLE of no dictionary entry");
            continue;
        };
        match file.read_object(o).and_then(|mut r| mleader_style(&mut r, name, tables)) {
            Ok(st) => {
                match d.mleader_styles.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name)) {
                    Some(x) => *x = st,
                    None => d.mleader_styles.push(st),
                }
                read.insert(o.handle);
            }
            Err(_) => unread.count("MLEADERSTYLE (not decoded)"),
        }
    }
    // Multiline styles, then each multiline's elements from its style.
    let mut styles = Vec::new();
    for o in objects.iter().filter(|o| o.type_name == "MLINESTYLE") {
        match file.read_object(o).and_then(|mut r| mline_style(&mut r, tables)) {
            Ok(Some(st)) => {
                styles.push(st);
                read.insert(o.handle);
            }
            Ok(None) => unread.count("MLINESTYLE without a name"),
            Err(_) => unread.count("MLINESTYLE (not decoded)"),
        }
    }
    crate::dxf_mline::apply_styles(d, styles);
    for o in objects.iter().filter(|o| o.type_name == "DIMASSOC") {
        match file.read_object(o).and_then(|mut r| dimassoc(&mut r)) {
            Ok(tags) => {
                crate::dxf_read::apply_dimassoc(d, &tags);
                read.insert(o.handle);
            }
            Err(_) => unread.count("DIMASSOC (not decoded)"),
        }
    }
    read
}
