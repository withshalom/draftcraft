//! Decoding of individual objects: the common object/entity header, the
//! stream split, type dispatch and the handle stream.
//!
//! Written from knowledge of the spec, chapter 20 ("common non-entity
//! object format", "common entity format", "common entity handle data").
//! VERIFY against the spec PDF and real files. Checked so far: the fields
//! up to and including the extended data (§20.1, §20.4.1), which
//! `eed.rs` decodes (chapter 28).

use super::eed;
use super::entities;
use super::streams::{Streams, string_stream};
use super::tables;
use super::version::DwgVersion as V;
use super::version::DwgVersion;
use crate::bits::BitReader;
use crate::read::error::{Error, Result};
use crate::read::model::{ClassDef, EntityCommon, Handle, LinetypeRef, Object, ObjectKind, RawDwgObject, Space, VertexKind, XData};

pub(crate) struct DecodeCtx<'a> {
    pub ver: DwgVersion,
    pub codepage: u16,
    pub classes: &'a [ClassDef],
    /// How text becomes strings.
    pub text: crate::records::text::TextDecoding,
}

/// Name and entity/non-entity classification of the fixed object types.
pub(crate) fn fixed_type(code: u16) -> Option<(&'static str, bool)> {
    Some(match code {
        0x01 => ("TEXT", true),
        0x02 => ("ATTRIB", true),
        0x03 => ("ATTDEF", true),
        0x04 => ("BLOCK", true),
        0x05 => ("ENDBLK", true),
        0x06 => ("SEQEND", true),
        0x07 => ("INSERT", true),
        0x08 => ("MINSERT", true),
        0x0A => ("VERTEX_2D", true),
        0x0B => ("VERTEX_3D", true),
        0x0C => ("VERTEX_MESH", true),
        0x0D => ("VERTEX_PFACE", true),
        0x0E => ("VERTEX_PFACE_FACE", true),
        0x0F => ("POLYLINE_2D", true),
        0x10 => ("POLYLINE_3D", true),
        0x11 => ("ARC", true),
        0x12 => ("CIRCLE", true),
        0x13 => ("LINE", true),
        0x14 => ("DIMENSION_ORDINATE", true),
        0x15 => ("DIMENSION_LINEAR", true),
        0x16 => ("DIMENSION_ALIGNED", true),
        0x17 => ("DIMENSION_ANG_3PT", true),
        0x18 => ("DIMENSION_ANG_2LN", true),
        0x19 => ("DIMENSION_RADIUS", true),
        0x1A => ("DIMENSION_DIAMETER", true),
        0x1B => ("POINT", true),
        0x1C => ("3DFACE", true),
        0x1D => ("POLYLINE_PFACE", true),
        0x1E => ("POLYLINE_MESH", true),
        0x1F => ("SOLID", true),
        0x20 => ("TRACE", true),
        0x21 => ("SHAPE", true),
        0x22 => ("VIEWPORT", true),
        0x23 => ("ELLIPSE", true),
        0x24 => ("SPLINE", true),
        0x25 => ("REGION", true),
        0x26 => ("3DSOLID", true),
        0x27 => ("BODY", true),
        0x28 => ("RAY", true),
        0x29 => ("XLINE", true),
        0x2A => ("DICTIONARY", false),
        0x2B => ("OLEFRAME", true),
        0x2C => ("MTEXT", true),
        0x2D => ("LEADER", true),
        0x2E => ("TOLERANCE", true),
        0x2F => ("MLINE", true),
        0x30 => ("BLOCK_CONTROL", false),
        0x31 => ("BLOCK_HEADER", false),
        0x32 => ("LAYER_CONTROL", false),
        0x33 => ("LAYER", false),
        0x34 => ("STYLE_CONTROL", false),
        0x35 => ("STYLE", false),
        0x38 => ("LTYPE_CONTROL", false),
        0x39 => ("LTYPE", false),
        0x3C => ("VIEW_CONTROL", false),
        0x3D => ("VIEW", false),
        0x3E => ("UCS_CONTROL", false),
        0x3F => ("UCS", false),
        0x40 => ("VPORT_CONTROL", false),
        0x41 => ("VPORT", false),
        0x42 => ("APPID_CONTROL", false),
        0x43 => ("APPID", false),
        0x44 => ("DIMSTYLE_CONTROL", false),
        0x45 => ("DIMSTYLE", false),
        0x46 => ("VX_CONTROL", false),
        0x47 => ("VX_TABLE_RECORD", false),
        0x48 => ("GROUP", false),
        0x49 => ("MLINESTYLE", false),
        0x4A => ("OLE2FRAME", true),
        0x4B => ("DUMMY", false),
        0x4C => ("LONG_TRANSACTION", true),
        0x4D => ("LWPOLYLINE", true),
        0x4E => ("HATCH", true),
        0x4F => ("XRECORD", false),
        0x50 => ("ACDBPLACEHOLDER", false),
        0x51 => ("VBA_PROJECT", false),
        0x52 => ("LAYOUT", false),
        0x1F2 => ("ACAD_PROXY_ENTITY", true),
        0x1F3 => ("ACAD_PROXY_OBJECT", false),
        _ => return None,
    })
}

/// Name and entity flag for any type code, using the class table for
/// codes 500 and up.
pub(crate) fn classify(code: u16, classes: &[ClassDef]) -> (String, bool) {
    if let Some((name, ent)) = fixed_type(code) {
        return (name.to_string(), ent);
    }
    if code >= 500
        && let Some(c) = classes.iter().find(|c| c.number == code)
    {
        let name = if c.dxf_name.is_empty() { format!("CLASS_{code}") } else { c.dxf_name.clone() };
        return (name, c.is_entity());
    }
    (format!("TYPE_{code:#X}"), false)
}

/// Lightweight look at an object: its own handle and type code. Used to
/// validate the object map before full decoding.
pub(crate) fn peek(ver: DwgVersion, data: &[u8], offset: usize) -> Result<(u64, u16)> {
    let mut r = BitReader::new(data);
    r.set_position(offset.checked_mul(8).ok_or_else(|| Error::format("offset overflow"))?)?;
    let size = r.ms()? as usize;
    if ver >= V::R2010 {
        let _hsize = r.mc_unsigned()?;
    }
    let start = r.position() / 8;
    let end = start.checked_add(size).ok_or_else(|| Error::format("size overflow"))?;
    if end > data.len() || size == 0 {
        return Err(Error::truncated(format!("object at {offset} of {size} bytes runs past data")));
    }
    let mut dat = BitReader::new(&data[start..end]);
    let code = if ver >= V::R2010 { dat.ot()? } else { dat.bs()? };
    if (V::R2000..=V::R2007).contains(&ver) {
        let _bitsize = dat.rl()?;
    }
    let h = dat.handle()?;
    Ok((h.value, code))
}

/// Common entity fields read from the main stream, kept for the handle
/// stream that follows.
#[derive(Default)]
struct EntityFlags {
    entmode: u8,
    num_reactors: u32,
    xdic_missing: bool,
    bylayer_ltype: bool,
    no_links: bool,
    color_handle: bool,
    ltype_flags: u8,
    plotstyle_flags: u8,
    material_flags: u8,
    full_vs: bool,
    face_vs: bool,
    edge_vs: bool,
}

/// Extra counts some types need when reading their handles.
enum Pending {
    None,
    Owned(u32),
    Insert {
        owned: u32,
    },
    Block(tables::BlockHeaderCounts),
    Control {
        entries: u32,
        is_block_control: bool,
    },
    Hatch {
        /// Boundary object handles of each path.
        boundary_handles: Vec<u32>,
    },
    Viewport {
        frozen: u32,
    },
}

/// An object whose common fields have been read: the streams are
/// positioned at the type-specific data, and the handle stream at the
/// owner reference.
pub(crate) struct Opened<'a> {
    pub handle: u64,
    pub type_code: u16,
    pub type_name: String,
    pub is_entity: bool,
    pub size: usize,
    pub common: Option<EntityCommon>,
    flags: EntityFlags,
    pub num_reactors: u32,
    pub xdic_missing: bool,
    pub xdata: Vec<XData>,
    /// The entity that follows this one in its block's chain (up to AutoCAD 2000), when the
    /// entity stores it; `None` when the next object of the file follows.
    pub next_entity: Option<u64>,
    pub s: Streams<'a>,
}

/// Read the common fields of the object whose modular-short size begins
/// at byte `offset` of `data`, up to its type-specific data.
pub(crate) fn open<'a>(ctx: &DecodeCtx, data: &'a [u8], offset: usize) -> Result<Opened<'a>> {
    let ver = ctx.ver;
    let mut r = BitReader::new(data);
    r.set_position(offset.checked_mul(8).ok_or_else(|| Error::format("offset overflow"))?)?;
    let size = r.ms()? as usize;
    let hsize = if ver >= V::R2010 { r.mc_unsigned()? as usize } else { 0 };
    let start = r.position() / 8;
    let end = start.checked_add(size).ok_or_else(|| Error::format("size overflow"))?;
    if end > data.len() || size == 0 {
        return Err(Error::truncated(format!("object at {offset} of {size} bytes runs past data")));
    }
    let obj = &data[start..end];
    let total_bits = size * 8;
    let mut dat = BitReader::new(obj);

    let type_code = if ver >= V::R2010 { dat.ot()? } else { dat.bs()? };
    let mut bitsize = if ver >= V::R2010 {
        total_bits.checked_sub(hsize).ok_or_else(|| Error::format(format!("handle stream {hsize} bits > object {total_bits} bits")))?
    } else if ver >= V::R2000 {
        dat.rl()? as usize
    } else {
        0
    };
    let handle = dat.handle()?.value;
    let xdata = eed::read(&mut dat, ver, handle, Some(ctx.text))?;

    let (type_name, is_entity) = classify(type_code, ctx.classes);

    let mut flags = EntityFlags::default();
    let mut common: Option<EntityCommon> = None;
    let num_reactors;
    let xdic_missing;
    if is_entity {
        if dat.b()? {
            let gsize = if ver >= V::R2010 { dat.bll()? } else { dat.rl()? as u64 };
            let bits = (gsize as usize).checked_mul(8).ok_or_else(|| Error::format("graphics size overflow"))?;
            dat.skip_bits(bits)?;
        }
        if ver <= V::R14 {
            bitsize = dat.rl()? as usize;
        }
        flags.entmode = dat.bb()?;
        flags.num_reactors = dat.bl()?;
        if ver >= V::R2004 {
            flags.xdic_missing = dat.b()?;
        }
        if ver >= V::R2013 {
            let _has_ds_data = dat.b()?;
        }
        if ver <= V::R14 {
            flags.bylayer_ltype = dat.b()?;
        }
        flags.no_links = if ver <= V::R2000 { dat.b()? } else { true };
        let mut ent = EntityCommon {
            space: match flags.entmode {
                1 => Space::Paper,
                2 => Space::Model,
                _ => Space::Owned,
            },
            linetype_scale: 1.0,
            ..Default::default()
        };
        // Color: BS index up to R2000, ENC from R2004.
        {
            let mut tmp = Streams { ver, codepage: ctx.codepage, dat, strs: None, hdl: None, text: ctx.text };
            let (color, transparency, color_handle) = tmp.enc()?;
            dat = tmp.dat;
            ent.color = color;
            ent.transparency = transparency;
            flags.color_handle = color_handle;
        }
        ent.linetype_scale = dat.bd()?;
        if ver >= V::R2000 {
            flags.ltype_flags = dat.bb()?;
            flags.plotstyle_flags = dat.bb()?;
        }
        if ver >= V::R2007 {
            flags.material_flags = dat.bb()?;
            let _shadow = dat.rc()?;
        }
        if ver >= V::R2010 {
            flags.full_vs = dat.b()?;
            flags.face_vs = dat.b()?;
            flags.edge_vs = dat.b()?;
        }
        ent.invisible = dat.bs()? & 1 != 0;
        if ver >= V::R2000 {
            ent.lineweight = dat.rc()?;
        }
        ent.linetype = if ver <= V::R14 {
            if flags.bylayer_ltype { LinetypeRef::ByLayer } else { LinetypeRef::Continuous }
        } else {
            match flags.ltype_flags {
                0 => LinetypeRef::ByLayer,
                1 => LinetypeRef::ByBlock,
                2 => LinetypeRef::Continuous,
                _ => LinetypeRef::Continuous,
            }
        };
        num_reactors = flags.num_reactors;
        xdic_missing = flags.xdic_missing;
        common = Some(ent);
    } else {
        if ver <= V::R14 {
            bitsize = dat.rl()? as usize;
        }
        num_reactors = dat.bl()?;
        xdic_missing = if ver >= V::R2004 { dat.b()? } else { false };
        if ver >= V::R2013 {
            let _has_ds_data = dat.b()?;
        }
    }

    if bitsize > total_bits {
        return Err(Error::format(format!("data end bit {bitsize} beyond object size {total_bits} ({type_name} {handle:X})")));
    }

    // Split into streams.
    let hdl = BitReader::with_bounds(obj, bitsize, total_bits);
    let mut strs = None;
    let mut data_end = bitsize;
    if ver.has_string_stream() {
        match string_stream(obj, bitsize)? {
            Some((sr, st)) => {
                strs = Some(sr);
                data_end = st;
            }
            None => data_end = bitsize.saturating_sub(1),
        }
    }
    // If the common data already ran into the strings, reads now fail and
    // the object is reported as failed rather than decoded from garbage.
    dat.set_end(data_end);
    Ok(Opened {
        handle,
        type_code,
        type_name,
        is_entity,
        size,
        common,
        flags,
        num_reactors,
        xdic_missing,
        xdata,
        next_entity: None,
        s: Streams { ver, codepage: ctx.codepage, dat, strs, hdl: Some(hdl), text: ctx.text },
    })
}

/// The common handles: owner, reactors, extension dictionary and, for an
/// entity, its layer, linetype, material and plot style (into `common`).
pub(crate) fn common_handles(o: &mut Opened) -> Result<(u64, Vec<Handle>, u64)> {
    let ver = o.s.ver;
    let handle = o.handle;
    let s = &mut o.s;
    let flags = &o.flags;
    let mut owner = 0u64;
    let mut reactors = Vec::new();
    let mut xdictionary = 0u64;
    let mut next_entity = None;
    if o.is_entity {
        let Some(ent) = o.common.as_mut() else {
            return Err(Error::format("entity without its common data"));
        };
        if flags.entmode == 0 {
            owner = s.h_abs(handle)?;
        }
        let n = s.check_handle_count(o.num_reactors, "reactor")?;
        for _ in 0..n {
            reactors.push(Handle(s.h_abs(handle)?));
        }
        if ver < V::R2004 || !o.xdic_missing {
            xdictionary = s.h_abs(handle)?;
        }
        if ver <= V::R14 {
            ent.layer = Handle(s.h_abs(handle)?);
            if !flags.bylayer_ltype {
                ent.linetype = LinetypeRef::Handle(Handle(s.h_abs(handle)?));
            }
        }
        if ver <= V::R2000 && !flags.no_links {
            let _prev = s.h()?;
            next_entity = Some(s.h_abs(handle)?);
        }
        if ver >= V::R2004 && flags.color_handle {
            ent.color_book = Handle(s.h_abs(handle)?);
        }
        if ver >= V::R2000 {
            ent.layer = Handle(s.h_abs(handle)?);
            if flags.ltype_flags == 3 {
                ent.linetype = LinetypeRef::Handle(Handle(s.h_abs(handle)?));
            }
        }
        if ver >= V::R2007 && flags.material_flags == 3 {
            ent.material = Handle(s.h_abs(handle)?);
        }
        if ver >= V::R2000 && flags.plotstyle_flags == 3 {
            ent.plotstyle = Handle(s.h_abs(handle)?);
        }
        if ver >= V::R2010 {
            for present in [flags.full_vs, flags.face_vs, flags.edge_vs] {
                if present {
                    let _vs = s.h()?;
                }
            }
        }
    } else {
        owner = s.h_abs(handle)?;
        let n = s.check_handle_count(o.num_reactors, "reactor")?;
        for _ in 0..n {
            reactors.push(Handle(s.h_abs(handle)?));
        }
        if ver < V::R2004 || !o.xdic_missing {
            xdictionary = s.h_abs(handle)?;
        }
    }
    o.next_entity = next_entity;
    Ok((owner, reactors, xdictionary))
}

/// Decode the object whose modular-short size begins at byte `offset` of
/// `data` (the whole file for R13–R15, the AcDb:AcDbObjects section for
/// R2004+).
pub(crate) fn decode(ctx: &DecodeCtx, data: &[u8], offset: usize) -> Result<Object> {
    let ver = ctx.ver;
    let mut o = open(ctx, data, offset)?;
    let handle = o.handle;
    let type_code = o.type_code;

    // Type-specific data.
    let specific = decode_specific(type_code, &o.type_name, &mut o.s);
    // Records whose every data field is read must end exactly where their
    // strings begin (R2007+, where that boundary is known). Anything else
    // means the layout read here differs from the file's.
    let mut fit_note = None;
    if ver.has_string_stream() && specific.is_ok() && matches!(type_code, 0x31 | 0x33 | 0x43) && o.s.dat.remaining() != 0 {
        fit_note = Some(format!("data: {} bit(s) left before the strings; this version's layout may differ", o.s.dat.remaining()));
    }

    // Common handles.
    let (mut owner, mut reactors, mut xdictionary) = (0u64, Vec::new(), 0u64);
    let mut handle_note: Option<String> = None;
    match common_handles(&mut o) {
        Ok((a, b, c)) => {
            owner = a;
            reactors = b;
            xdictionary = c;
        }
        Err(e) => handle_note = Some(format!("common handles: {e}")),
    }

    // Types decoded from version 0.6 on (construction lines, leaders,
    // viewports; tolerances from 0.13, images and wipeouts from 0.14,
    // multilines from 0.15) are
    // kept as their bytes whenever the layout read here does not fit the
    // record exactly, as they were before.
    let newer = matches!(type_code, 0x22 | 0x28 | 0x29 | 0x2D | 0x2E | 0x2F) || matches!(o.type_name.as_str(), "IMAGE" | "WIPEOUT");
    let misfit = newer && (specific.is_err() || (ver.has_string_stream() && o.s.dat.remaining() != 0));
    let kind = match specific {
        _ if misfit => ObjectKind::Unsupported,
        Ok((mut kind, pending)) => {
            if handle_note.is_none()
                && let Err(e) = read_specific_handles(&mut kind, pending, &mut o.s, handle)
            {
                if newer {
                    kind = ObjectKind::Unsupported;
                } else {
                    handle_note = Some(format!("handles: {e}"));
                }
            }
            kind
        }
        Err(e) => ObjectKind::Failed(e.to_string()),
    };
    if handle_note.is_none() {
        handle_note = fit_note;
    }

    Ok(Object {
        handle: Handle(handle),
        type_code,
        type_name: o.type_name,
        owner: Handle(owner),
        reactors,
        xdictionary: Handle(xdictionary),
        entity: o.common,
        kind,
        offset,
        size: o.size,
        note: handle_note,
        raw: None,
        xdata: o.xdata,
        unknown_tags: Vec::new(),
    })
}

/// A buffer holding `raw` as it sits in an objects section (size,
/// handle-stream size, data and a blank CRC), for [`open`].
pub(crate) fn raw_buffer(raw: &RawDwgObject) -> Vec<u8> {
    let mut w = crate::bits::BitWriter::new();
    w.ms(raw.data.len() as u32);
    if raw.version >= V::R2010 {
        w.mc_unsigned(raw.handle_stream_bits);
    }
    let mut out = w.into_bytes();
    out.extend_from_slice(&raw.data);
    out.extend_from_slice(&[0, 0]);
    out
}

/// The undecoded bytes of the object whose size prefix starts at `offset`,
/// for keeping objects the core cannot decode.
pub(crate) fn raw(ver: DwgVersion, data: &[u8], offset: usize) -> Result<RawDwgObject> {
    let mut r = BitReader::new(data);
    r.set_position(offset.checked_mul(8).ok_or_else(|| Error::format("offset overflow"))?)?;
    let size = r.ms()? as usize;
    let hsize = if ver >= V::R2010 { r.mc_unsigned()? } else { 0 };
    let start = r.position() / 8;
    let end = start.checked_add(size).ok_or_else(|| Error::format("size overflow"))?;
    let bytes = data.get(start..end).ok_or_else(|| Error::truncated("object runs past data"))?;
    Ok(RawDwgObject { version: ver, handle_stream_bits: hsize, data: bytes.to_vec() })
}

fn decode_specific(code: u16, name: &str, s: &mut Streams) -> Result<(ObjectKind, Pending)> {
    use ObjectKind as K;
    let r = match code {
        0x01 => (K::Text(entities::text(s)?), Pending::None),
        0x02 => (K::Attribute(entities::attribute(s, false)?), Pending::None),
        0x03 => (K::Attribute(entities::attribute(s, true)?), Pending::None),
        0x04 => (K::BlockBegin { name: s.tv()? }, Pending::None),
        0x05 => (K::BlockEnd, Pending::None),
        0x06 => (K::SeqEnd, Pending::None),
        0x07 | 0x08 => {
            let (ins, owned) = entities::insert(s, code == 0x08)?;
            (K::Insert(ins), Pending::Insert { owned })
        }
        0x0A => (K::Vertex(entities::vertex_2d(s)?), Pending::None),
        0x0B => (K::Vertex(entities::vertex_3d(s, VertexKind::Vertex3d)?), Pending::None),
        0x0C => (K::Vertex(entities::vertex_3d(s, VertexKind::Mesh)?), Pending::None),
        0x0D => (K::Vertex(entities::vertex_3d(s, VertexKind::PfaceVertex)?), Pending::None),
        0x0E => (K::Vertex(entities::vertex_pface_face(s)?), Pending::None),
        0x0F => {
            let (p, owned) = entities::polyline_2d(s)?;
            (K::Polyline(p), Pending::Owned(owned))
        }
        0x10 => {
            let (p, owned) = entities::polyline_3d(s)?;
            (K::Polyline(p), Pending::Owned(owned))
        }
        0x11 => (K::Arc(entities::arc(s)?), Pending::None),
        0x12 => (K::Circle(entities::circle(s)?), Pending::None),
        0x13 => (K::Line(entities::line(s)?), Pending::None),
        0x14..=0x1A => (K::Dimension(entities::dimension(s, code)?), Pending::None),
        0x1B => (K::Point(entities::point(s)?), Pending::None),
        0x1C => (K::Face3d(entities::face3d(s)?), Pending::None),
        0x1D => {
            let (p, owned) = entities::polyline_pface(s)?;
            (K::Polyline(p), Pending::Owned(owned))
        }
        0x1E => {
            let (p, owned) = entities::polyline_mesh(s)?;
            (K::Polyline(p), Pending::Owned(owned))
        }
        0x1F | 0x20 => (K::Solid(entities::solid(s, code == 0x20)?), Pending::None),
        0x23 => (K::Ellipse(entities::ellipse(s)?), Pending::None),
        0x24 => (K::Spline(entities::spline(s)?), Pending::None),
        0x2C => (K::MText(entities::mtext(s)?), Pending::None),
        0x28 => (K::Ray(entities::construction_line(s)?), Pending::None),
        0x29 => (K::Xline(entities::construction_line(s)?), Pending::None),
        0x2D => (K::Leader(entities::leader(s)?), Pending::None),
        0x2E => (K::Tolerance(entities::tolerance(s)?), Pending::None),
        0x2F => (K::MLine(entities::mline(s)?), Pending::None),
        0x22 => {
            let (v, frozen) = entities::viewport(s)?;
            (K::PaperViewport(Box::new(v)), Pending::Viewport { frozen })
        }
        0x4D => (K::LwPolyline(entities::lwpolyline(s)?), Pending::None),
        0x4E => {
            let (h, n) = entities::hatch(s)?;
            (K::Hatch(h), Pending::Hatch { boundary_handles: n })
        }
        0x30 | 0x32 | 0x34 | 0x38 | 0x3C | 0x3E | 0x40 | 0x42 | 0x44 | 0x46 => {
            let (c, n) = tables::control(s)?;
            (K::Control(c), Pending::Control { entries: n, is_block_control: code == 0x30 })
        }
        0x31 => {
            let (b, counts) = tables::block_header(s)?;
            (K::BlockHeader(b), Pending::Block(counts))
        }
        0x33 => (K::Layer(tables::layer(s)?), Pending::None),
        0x35 => (tables::text_style(s)?, Pending::None),
        0x39 => (tables::linetype(s)?, Pending::None),
        0x43 => (K::NamedRecord(tables::appid(s)?), Pending::None),
        0x3D | 0x3F | 0x41 | 0x45 | 0x47 => (K::NamedRecord(tables::named_record(s)?), Pending::None),
        c if c >= 500 => match name {
            "LWPOLYLINE" => (K::LwPolyline(entities::lwpolyline(s)?), Pending::None),
            "IMAGE" => (K::Image(Box::new(entities::raster_image(s)?)), Pending::None),
            "WIPEOUT" => (K::Wipeout(Box::new(entities::raster_image(s)?)), Pending::None),
            "HATCH" => {
                let (h, n) = entities::hatch(s)?;
                (K::Hatch(h), Pending::Hatch { boundary_handles: n })
            }
            _ => (K::Unsupported, Pending::None),
        },
        _ => (K::Unsupported, Pending::None),
    };
    Ok(r)
}

fn read_specific_handles(kind: &mut ObjectKind, pending: Pending, s: &mut Streams, own: u64) -> Result<()> {
    let ver = s.ver;
    match kind {
        ObjectKind::Text(t) => t.style = Handle(s.h_abs(own)?),
        ObjectKind::Attribute(a) => a.text.style = Handle(s.h_abs(own)?),
        ObjectKind::MText(m) => m.style = Handle(s.h_abs(own)?),
        ObjectKind::Insert(ins) => {
            ins.block_header = Handle(s.h_abs(own)?);
            if ins.has_attribs {
                if ver <= V::R2000 {
                    ins.first_attrib = Handle(s.h_abs(own)?);
                    ins.last_attrib = Handle(s.h_abs(own)?);
                } else if let Pending::Insert { owned } = pending {
                    let n = s.check_handle_count(owned, "attribute")?;
                    for _ in 0..n {
                        ins.attribs.push(Handle(s.h_abs(own)?));
                    }
                }
                ins.seqend = Handle(s.h_abs(own)?);
            }
        }
        ObjectKind::Polyline(p) => {
            if ver <= V::R2000 {
                p.first_vertex = Handle(s.h_abs(own)?);
                p.last_vertex = Handle(s.h_abs(own)?);
            } else if let Pending::Owned(owned) = pending {
                let n = s.check_handle_count(owned, "vertex")?;
                for _ in 0..n {
                    p.vertices.push(Handle(s.h_abs(own)?));
                }
            }
            p.seqend = Handle(s.h_abs(own)?);
        }
        ObjectKind::Dimension(d) => {
            d.dimstyle = Handle(s.h_abs(own)?);
            d.block = Handle(s.h_abs(own)?);
        }
        ObjectKind::Leader(l) => {
            l.annotation = Handle(s.h_abs(own)?);
            l.dimstyle = Handle(s.h_abs(own)?);
        }
        ObjectKind::Tolerance(t) => t.dimstyle = Handle(s.h_abs(own)?),
        ObjectKind::MLine(m) => m.style = Handle(s.h_abs(own)?),
        ObjectKind::Image(r) | ObjectKind::Wipeout(r) => {
            r.definition = Handle(s.h_abs(own)?);
            r.reactor = Handle(s.h_abs(own)?);
        }
        ObjectKind::PaperViewport(v) => {
            if ver <= V::R14 {
                let _header = s.h()?;
            } else {
                if let Pending::Viewport { frozen } = pending {
                    let n = s.check_handle_count(frozen, "viewport frozen layer")?;
                    for _ in 0..n {
                        v.frozen_layers.push(Handle(s.h_abs(own)?));
                    }
                }
                v.clip_boundary = Handle(s.h_abs(own)?);
                if ver == V::R2000 {
                    let _header = s.h()?;
                }
                v.named_ucs = Handle(s.h_abs(own)?);
                v.base_ucs = Handle(s.h_abs(own)?);
                if ver >= V::R2007 {
                    v.background = Handle(s.h_abs(own)?);
                    v.visual_style = Handle(s.h_abs(own)?);
                    v.shade_plot = Handle(s.h_abs(own)?);
                    v.sun = Handle(s.h_abs(own)?);
                }
            }
        }
        ObjectKind::Hatch(h) => {
            // Boundary object handles, path by path.
            if let Pending::Hatch { boundary_handles } = pending {
                h.sources.clear();
                for count in boundary_handles {
                    let n = s.check_handle_count(count, "hatch boundary")?;
                    let mut path = Vec::with_capacity(n);
                    for _ in 0..n {
                        path.push(Handle(s.h_abs(own)?));
                    }
                    h.sources.push(path);
                }
                if h.sources.iter().all(Vec::is_empty) {
                    h.sources.clear();
                }
            }
        }
        ObjectKind::Control(c) => {
            if let Pending::Control { entries, is_block_control } = pending {
                let n = s.check_handle_count(entries, "table entry")?;
                for _ in 0..n {
                    c.entries.push(Handle(s.h_abs(own)?));
                }
                if is_block_control {
                    c.model_space = Handle(s.h_abs(own)?);
                    c.paper_space = Handle(s.h_abs(own)?);
                }
            }
        }
        ObjectKind::BlockHeader(b) => {
            if let Pending::Block(counts) = pending {
                let _null = s.h()?;
                b.block_entity = Handle(s.h_abs(own)?);
                if counts.lists_entities {
                    if ver <= V::R2000 {
                        b.first_entity = Handle(s.h_abs(own)?);
                        b.last_entity = Handle(s.h_abs(own)?);
                    } else {
                        let n = s.check_handle_count(counts.owned, "block entity")?;
                        for _ in 0..n {
                            b.entities.push(Handle(s.h_abs(own)?));
                        }
                    }
                }
                b.endblk = Handle(s.h_abs(own)?);
                if ver >= V::R2000 {
                    let n = s.check_handle_count(counts.inserts, "block insert")?;
                    for _ in 0..n {
                        let _insert = s.h()?;
                    }
                    b.layout = Handle(s.h_abs(own)?);
                }
            }
        }
        ObjectKind::Layer(l) => {
            let _xref_block = s.h()?;
            if ver >= V::R2000 {
                let _plotstyle = s.h()?;
            }
            if ver >= V::R2007 {
                let _material = s.h()?;
            }
            l.linetype = Handle(s.h_abs(own)?);
        }
        ObjectKind::Linetype(l) => {
            let _xref_block = s.h()?;
            // One shape file or text style per element (§20.4.58).
            for e in l.elements.iter_mut() {
                e.style = Handle(s.h_abs(own)?);
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::geom::Vec3;
    use crate::read::model::{Groups, XrValue};

    /// xorshift64* — a tiny deterministic generator for robustness tests.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn bytes(&mut self, n: usize) -> Vec<u8> {
            (0..n).map(|_| self.next() as u8).collect()
        }
    }

    /// The STYLE example printed in the ODA specification (§20.4.56.1), an
    /// R14 object, decodes to the values it shows: STANDARD, font txt.
    #[test]
    fn the_specifications_style_example_decodes() {
        let bytes = [
            0x25, 0x00, 0x4D, 0x40, 0x44, 0x20, 0x20, 0x10, 0x00, 0x09, 0x08, 0x53, 0x54, 0x41, 0x4E, 0x44, 0x41, 0x52, 0x44, 0xC2, 0x60, 0x02, 0x6A,
            0x66, 0x66, 0x66, 0x66, 0x67, 0x24, 0xFD, 0x03, 0x74, 0x78, 0x74, 0x90, 0x40, 0xCC, 0x14, 0x28, 0xEC, 0x6E,
        ];
        let ctx = DecodeCtx { ver: V::R14, codepage: 30, classes: &[], text: Default::default() };
        let o = decode(&ctx, &bytes, 0).unwrap();
        assert_eq!(o.type_name, "STYLE");
        assert_eq!(o.handle, Handle(0x10));
        match &o.kind {
            ObjectKind::TextStyle(t) => {
                assert_eq!(t.name, "STANDARD");
                assert_eq!(t.font_file, "txt");
                assert_eq!(t.bigfont_file, "");
                assert_eq!(t.height, 0.0);
                assert_eq!(t.width_factor, 1.0);
                assert_eq!(t.oblique, 0.0);
                assert_eq!(t.last_height, 0.2);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(o.note, None, "handles read cleanly");
    }

    /// The TOLERANCE example printed in the ODA specification (§20.4.49.1),
    /// an R14 object (handle 010C).
    #[test]
    fn the_specifications_tolerance_example_decodes() {
        let bytes = [
            0x65, 0x00, 0x4B, 0x80, 0x80, 0x43, 0x27, 0x18, 0x10, 0x00, 0x05, 0x5B, 0x40, 0x56, 0xBD, 0x1B, 0x81, 0xE8, 0x56, 0x39, 0xF8, 0x15, 0xAF,
            0x46, 0xE0, 0x7A, 0x15, 0x6E, 0x7E, 0x51, 0x19, 0xA0, 0x47, 0x00, 0xC7, 0x13, 0xA0, 0x18, 0x09, 0x38, 0x21, 0x8A, 0x5E, 0xA1, 0x48, 0x13,
            0x54, 0xA5, 0xCF, 0x6B, 0x88, 0xCC, 0xEC, 0x8E, 0x87, 0x6D, 0x4F, 0xA4, 0xA4, 0xAE, 0xCF, 0x6B, 0x88, 0xCC, 0xEC, 0x8E, 0x87, 0x6D, 0xCF,
            0xAC, 0x2E, 0x6C, 0x8C, 0xCF, 0x6B, 0x88, 0xCC, 0xEC, 0x8E, 0x87, 0x6D, 0xAF, 0xA4, 0xA4, 0xAE, 0xC4, 0xA4, 0xAE, 0xC4, 0xA4, 0xAE, 0xC4,
            0xA4, 0xAE, 0xC6, 0x0A, 0x21, 0xF8, 0x20, 0x4C, 0x0A, 0x23, 0xB4, 0x45, 0x2F,
        ];
        let ctx = DecodeCtx { ver: V::R14, codepage: 30, classes: &[], text: Default::default() };
        let o = decode(&ctx, &bytes, 0).unwrap();
        assert_eq!(o.type_name, "TOLERANCE");
        assert_eq!(o.handle, Handle(0x10C));
        match &o.kind {
            ObjectKind::Tolerance(t) => {
                assert_eq!(t.text, "{\\Fgdt;j}%%v{\\Fgdt;n}asdf{\\Fgdt;m}%%v%%v%%v%%v");
                assert_eq!(t.r14, Some((0, 0.18, 0.09)), "DIMTXT and DIMGAP's defaults");
                assert!((t.insertion.x - 11.777_355_618_798_37).abs() < 1e-12);
                assert!((t.insertion.y - 3.369_785_403_870_281_5).abs() < 1e-12);
                assert_eq!(t.x_direction, Vec3::X);
                assert_eq!(t.extrusion, Vec3::Z);
                assert_eq!(t.dimstyle, Handle(0x1D));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(o.note, None, "handles read cleanly");
    }

    /// The MLINE example printed in the ODA specification (§20.4.50.1), an
    /// R14 object (handle 010D).
    #[test]
    fn the_specifications_mline_example_decodes() {
        let bytes = hex(
            "e4004bc0804366c83000055b200461ad1ef213a88a0181933c67d42bc07f528081206461ad1ef213a88a0181933c67d42bc07f13e9cf70de473cc7e4f86a7b9c002f39fc4e86a7b9c002f3df93e9cf70de473cc7f2055204000000000000785fd038dd69b078de3880440a1c33bde105204062c65315c95771f9fb6cf09b58b3ab7f05473cf47b0b79b7e08f921ddcd12f79fc81548118d6ba82c020de77f42750a165460292a013161543da240028101cb194c57255dc7e7f5b3c26d62ceadfc765b3c26d62cea9f820b194c57255dc7f2055204000000000000785fd182887c05087288e4c9188",
        );
        let ctx = DecodeCtx { ver: V::R14, codepage: 30, classes: &[], text: Default::default() };
        let o = decode(&ctx, &bytes, 0).unwrap();
        assert_eq!((o.type_name.as_str(), o.handle), ("MLINE", Handle(0x10D)));
        assert_eq!(o.note, None, "handles read cleanly");
        let ObjectKind::MLine(m) = &o.kind else { panic!("{:?}", o.kind) };
        // A three-vertex multiline in a two-element style (STANDARD, one
        // unit apart), top justified, open: unit directions, miters at
        // right angles to the first and last segments, each element's
        // start along the miter (0, then 1 unit or more at the bend).
        assert_eq!((m.scale, m.justification, m.flags, m.lines_in_style), (1.0, 0, 1, 2));
        assert_eq!(m.style, Handle(0x1C));
        assert_eq!(m.base, m.vertices[0].point);
        assert_eq!(m.vertices.len(), 3);
        for v in &m.vertices {
            assert!((v.direction.length() - 1.0).abs() < 1e-12);
            assert!((v.miter.length() - 1.0).abs() < 1e-12);
            assert_eq!(v.params.len(), 2);
            assert!(v.params.iter().all(|(seg, fill)| seg.len() == 2 && fill.is_empty()));
        }
        assert!(m.vertices[0].direction.dot(m.vertices[0].miter).abs() < 1e-12);
        assert_eq!(m.vertices[0].params[1].0, vec![-1.0, 0.0]);
        assert!((m.vertices[1].params[1].0[0] + 1.188_965_882_476_041_6).abs() < 1e-12);
        assert!((m.vertices[2].point.x - 10.510_839_768_060_261).abs() < 1e-12);
    }

    /// The raster examples printed in the ODA specification, R14 objects
    /// whose classes are numbered as in the file they came from: IMAGE
    /// (§20.4.80.1, class 1F9), IMAGEDEF (§20.4.81.1, 1F7), IMAGEDEF_REACTOR
    /// (§20.4.82.1, 1F8) and RASTERVARIABLES (§20.4.91.1, 1F5).
    fn raster_example(name: &str) -> (Vec<u8>, Vec<ClassDef>) {
        let (hex_bytes, number, entity) = match name {
            "IMAGE" => (
                "09013e40405b6c600000046000000008000004200000003000000028000006b56e620baae102000372c563f8d8aa000000000000000006b56e620baae102000372c563f8d8ca000000000000000006b56e620baae112000372c563f8d8ca000000000000000006b56e620baae112000372c563f8d8aa000000000000000006b56e620baae102000372c563f8d8aa000000000000000006d038000280db46b56e620baae1020000dcb158fe362a810000000000001007f4000000000053109e0000000000001007f0000000000003020000000000000302020e32320040400000000000382fc00000000000382fc0000000003817d000000000003817d0105ecc1443f0416843545acc5b3f0d2a",
                0x1F9,
                true,
            ),
            "IMAGEDEF" => (
                "4e003dc0405ae3b02000040a0000000000006040000000000000604046d0ce9715d2539395171199585d1a195c95195e1d1d5c994b989b5c205e25d4eb0752bac7fe25d4eb0752bac7f0082d482d8611e823",
                0x1F7,
                false,
            ),
            "IMAGEDEF_REACTOR" => ("0c003e00405b2500000009026030a113", 0x1F8, false),
            _ => ("11003d404056a66000000406405019010430c0dcd2", 0x1F5, false),
        };
        let classes = vec![ClassDef { number, dxf_name: name.into(), item_class_id: if entity { 0x1F2 } else { 0x1F3 }, ..Default::default() }];
        (hex(hex_bytes), classes)
    }

    #[test]
    fn the_specifications_image_example_decodes() {
        let (bytes, classes) = raster_example("IMAGE");
        let ctx = DecodeCtx { ver: V::R14, codepage: 30, classes: &classes, text: Default::default() };
        let o = decode(&ctx, &bytes, 0).unwrap();
        assert_eq!((o.type_name.as_str(), o.handle), ("IMAGE", Handle(0x6D)));
        assert_eq!(o.note, None, "handles read cleanly");
        let ObjectKind::Image(r) = &o.kind else { panic!("{:?}", o.kind) };
        // A 128 × 128 pixel image, 1/128 unit a pixel, unclipped: its clip
        // rectangle is the whole image, pixel centres at whole numbers.
        assert_eq!(r.size, crate::read::geom::Vec2::new(128.0, 128.0));
        assert_eq!(r.u, Vec3::new(0.0078125, 0.0, 0.0));
        assert!((r.v.y - 0.0078125).abs() < 1e-15 && r.v.x.abs() < 1e-15);
        assert!((r.insertion.x - 8.180_582_100_138_889).abs() < 1e-12);
        assert!((r.insertion.y - 5.276_854_222_199_745).abs() < 1e-12);
        assert_eq!((r.clip_type, r.clipping), (1, false));
        assert_eq!(r.clip, vec![crate::read::geom::Vec2::new(-0.5, -0.5), crate::read::geom::Vec2::new(127.5, 127.5)]);
        assert_eq!((r.display, r.brightness, r.contrast, r.fade), (7, 50, 50, 0));
        assert_eq!(r.definition, Handle(0x6B), "its IMAGEDEF, the next example");
        assert_eq!(r.reactor, Handle(0x6C), "its IMAGEDEF_REACTOR");
    }

    /// Decodes a spec example of an object kept as DXF groups.
    fn groups_example(name: &str) -> Groups {
        let (bytes, classes) = raster_example(name);
        let ver = V::R14;
        let mut o = decode(&DecodeCtx { ver, codepage: 30, classes: &classes, text: Default::default() }, &bytes, 0).unwrap();
        assert_eq!(o.type_name, name);
        o.raw = Some(raw(ver, &bytes, 0).unwrap());
        let lookup = crate::read::styles::Lookup::empty();
        match crate::read::nongraphic::decode_groups(&o, &classes, 30, &lookup) {
            Some(Ok(g)) => g,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_specifications_image_object_examples_decode() {
        use crate::read::geom::Vec3 as P;
        let def = groups_example("IMAGEDEF");
        assert_eq!(
            def.items,
            vec![
                (100, XrValue::Str("AcDbRasterImageDef".into())),
                (90, XrValue::Int32(0)),
                (1, XrValue::Str("C:\\WINNT\\FeatherTexture.bmp".into())),
                (10, XrValue::Point(P::new(128.0, 128.0, 0.0))),
                (11, XrValue::Point(P::new(0.352858149123434, 0.352858149123434, 0.0))),
                (280, XrValue::Int8(1)),
                (281, XrValue::Int8(2)),
            ],
            "128 × 128 pixels, loaded, centimetres"
        );
        let reactor = groups_example("IMAGEDEF_REACTOR");
        assert_eq!(reactor.items, vec![(100, XrValue::Str("AcDbRasterImageDefReactor".into())), (90, XrValue::Int32(2))]);
        let vars = groups_example("RASTERVARIABLES");
        assert_eq!(
            vars.items,
            vec![
                (100, XrValue::Str("AcDbRasterVariables".into())),
                (90, XrValue::Int32(0)),
                (70, XrValue::Int16(1)),
                (71, XrValue::Int16(1)),
                (72, XrValue::Int16(0)),
            ],
            "frame shown, high quality, no units"
        );
    }

    /// An R2010+ object as it sits in the objects section: its size (MS),
    /// its handle-stream size in bits (MC), then the object.
    fn frame(handle_bits: u16, body: &[u8]) -> Vec<u8> {
        let n = body.len() as u16;
        assert!(n < 0x8000);
        let mut out = vec![n as u8, (n >> 8) as u8];
        if handle_bits < 0x80 {
            out.push(handle_bits as u8);
        } else {
            out.push((handle_bits & 0x7F) as u8 | 0x80);
            out.push((handle_bits >> 7) as u8);
        }
        out.extend_from_slice(body);
        out.extend_from_slice(&[0, 0]); // CRC (not checked here)
        out
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// Table records from an AC1032 file saved by AutoCAD LT 2026 (generic
    /// records: layer "0", application "ACAD", the paper-space block). From
    /// R2007 a table record stores no 64-flag and no xref-dependent bit
    /// (see `tables::record_common_flags`); read with them, none of these
    /// decodes. Each must also end exactly where its strings begin.
    #[test]
    fn r2018_table_records_from_a_real_file() {
        let ctx = DecodeCtx { ver: V::R2018, codepage: 30, classes: &[], text: Default::default() };
        let appid = frame(27, &hex("10c044aa8010441004300410044004a00a084a87"));
        let o = decode(&ctx, &appid, 0).unwrap();
        assert_eq!(o.handle, Handle(0x12));
        assert_eq!(o.owner, Handle(0x9));
        assert_eq!(o.note, None);
        match &o.kind {
            ObjectKind::NamedRecord(r) => assert_eq!(r.name, "ACAD"),
            other => panic!("{other:?}"),
        }

        let layer = frame(101, &hex("0cc044288f0038070000c300404c0006802820463cca0a21ea330a22ca1f"));
        let o = decode(&ctx, &layer, 0).unwrap();
        assert_eq!(o.handle, Handle(0x10));
        assert_eq!(o.note, None);
        match &o.kind {
            ObjectKind::Layer(l) => {
                assert_eq!(l.name, "0");
                assert_eq!(l.color, crate::read::model::Color::Index(7));
                assert!(!l.off && !l.frozen && !l.locked && l.plot);
                assert_eq!(l.flags, 0x3F0);
                assert_eq!(l.linetype, Handle(0x16));
            }
            other => panic!("{other:?}"),
        }

        let block = frame(77, &hex("0c40562a815401500430a80140018401c0019401c8017c014c01c00184018c019402b38028202a062b462b6a2b3f"));
        let o = decode(&ctx, &block, 0).unwrap();
        assert_eq!(o.handle, Handle(0x58));
        assert_eq!(o.note, None);
        match &o.kind {
            ObjectKind::BlockHeader(b) => {
                assert_eq!(b.name, "*Paper_Space");
                assert!(b.entities.is_empty());
                assert_eq!(b.base_point, crate::read::geom::Vec3::ZERO);
            }
            other => panic!("{other:?}"),
        }
    }

    /// Random bytes must produce errors or objects, never a panic. This
    /// exercises every decoder path reachable from random type codes.
    #[test]
    fn random_objects_do_not_panic() {
        let classes = vec![
            ClassDef { number: 500, dxf_name: "LWPOLYLINE".into(), item_class_id: 0x1F2, ..Default::default() },
            ClassDef { number: 501, dxf_name: "IMAGE".into(), item_class_id: 0x1F2, ..Default::default() },
            ClassDef { number: 502, dxf_name: "WIPEOUT".into(), item_class_id: 0x1F2, ..Default::default() },
        ];
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for ver in [V::R14, V::R2000, V::R2004, V::R2007, V::R2010, V::R2013, V::R2018] {
            let ctx = DecodeCtx { ver, codepage: 30, classes: &classes, text: Default::default() };
            for i in 0..4000 {
                let len = 4 + (rng.next() % 600) as usize;
                let mut buf = rng.bytes(len);
                // Plant a plausible size and a known type code half the time
                // so decoding gets past the header into type-specific code.
                if i % 2 == 0 && len > 8 {
                    let size = (len - 4) as u16 & 0x7FFF;
                    buf[0] = size as u8;
                    buf[1] = (size >> 8) as u8;
                }
                let _ = decode(&ctx, &buf, 0);
                let _ = peek(ver, &buf, 0);
            }
        }
    }
}
