//! Encoding single objects in the R2010+ object format (spec §20.1,
//! §20.2 and the per-type prescriptions of §20.4), the mirror of
//! `dwg::object` and the per-type readers.
//!
//! Field order and every choice the format leaves open (which compressed
//! form a value takes, how a handle reference is coded, where padding
//! goes) follow what AutoCAD 2027 writes: the tests encode every object of
//! the AutoCAD-saved MI-201 corpus file that the drawing model holds whole
//! and compare the bytes.

use crate::bits::BitWriter;
use crate::crc::crc16;
use crate::read::error::{Error, Result};
use crate::read::geom::{Vec2, Vec3};
use crate::read::model::{
    BlockHeader, Color, Dimension, DimensionKind, Drawing, Handle, Hatch, HatchEdge, HatchLibrary, HatchPath, Leader, Linetype, LinetypeRef, MText,
    NonGraphic, Object, ObjectKind, PaperViewport, PolylineKind, Space, Spline, Text, TextStyle, VertexKind, XDataApp, XValue, XrValue,
};
use crate::read::nongraphic::{DimVars, Vport};
use crate::read::version::DwgVersion as V;
use crate::read::version::DwgVersion;

/// The three streams an R2007+ object is written as.
pub(crate) struct ObjW {
    pub ver: DwgVersion,
    /// The object's own handle, for relative references.
    pub own: u64,
    pub dat: BitWriter,
    strs: BitWriter,
    has_strings: bool,
    pub hdl: BitWriter,
}

/// Which handle-reference codes a field takes (spec §2.13): soft
/// ownership 2, hard ownership 3, soft pointer 4, hard pointer 5.
#[derive(Clone, Copy)]
pub(crate) enum Ref {
    SoftOwner = 2,
    HardOwner = 3,
    SoftPtr = 4,
    HardPtr = 5,
}

impl ObjW {
    pub fn new(ver: DwgVersion, own: Handle) -> Self {
        ObjW { ver, own: own.0, dat: BitWriter::new(), strs: BitWriter::new(), has_strings: false, hdl: BitWriter::new() }
    }

    /// TV: from R2007 in the string stream, as UTF-16.
    pub fn tv(&mut self, s: &str) {
        if self.ver.has_string_stream() {
            self.strs.text16(s);
            self.has_strings = true;
        } else {
            let bytes: Vec<u8> = s.chars().map(|c| if (c as u32) < 0x100 { c as u8 } else { b'?' }).collect();
            self.dat.text8(&bytes);
        }
    }

    /// An absolute reference.
    pub fn h(&mut self, kind: Ref, target: Handle) {
        self.hdl.handle(kind as u8, target.0);
    }

    /// A soft pointer in its shortest form: absolute (code 4), the next
    /// or previous handle (6, 8: no bytes) or an offset from this object
    /// (0xA, 0xC) when the offset is smaller than the handle, as AutoCAD
    /// writes owners.
    pub fn h_rel(&mut self, target: Handle) {
        let t = target.0;
        let own = self.own;
        if t == 0 {
            self.hdl.handle(4, 0);
        } else if t == own.wrapping_add(1) {
            self.hdl.handle(6, 0);
        } else if t.wrapping_add(1) == own {
            self.hdl.handle(8, 0);
        } else if t > own && t - own < t {
            self.hdl.handle(0xA, t - own);
        } else if t < own && own - t < t {
            // AutoCAD codes an owner relative to the object only when the
            // offset is the smaller number (LAYER A owned by 8: C.2; APPID
            // 13 owned by 9: 4.9, not C.A).
            self.hdl.handle(0xC, own - t);
        } else {
            self.hdl.handle(4, t);
        }
    }

    /// CMC (R2004+): BS index, BL method word, RC name flags.
    pub fn cmc(&mut self, c: Color) {
        if self.ver < V::R2004 {
            self.dat.bs(c.to_aci() as u16);
            return;
        }
        // The index field stays 0: the colour is in the method word, as
        // AutoCAD writes every CmColor from R2004 on.
        self.dat.bs(0);
        self.dat.bl(c.to_method_word());
        self.dat.rc(0);
    }

    /// The finished object as it sits in the objects section: MS size, MC
    /// handle-stream size in bits, the data, the CRC.
    pub fn finish(mut self) -> Vec<u8> {
        if self.ver.has_string_stream() {
            if self.has_strings {
                let n = self.strs.position();
                self.dat.append(&self.strs);
                if n >= 0x8000 {
                    self.dat.rs((n >> 15) as u16);
                    self.dat.rs((n & 0x7FFF) as u16 | 0x8000);
                } else {
                    self.dat.rs(n as u16);
                }
                self.dat.b(true);
            } else {
                self.dat.b(false);
            }
        }
        let endbit = self.dat.position();
        self.dat.append(&self.hdl);
        // AutoCAD fills the last byte with 1 bits.
        while !self.dat.position().is_multiple_of(8) {
            self.dat.b(true);
        }
        let total_bits = self.dat.position();
        let data = self.dat.into_bytes();
        frame(self.ver, &data, (total_bits - endbit) as u64)
    }
}

/// Wrap object data in its size fields and CRC.
pub(crate) fn frame(ver: DwgVersion, data: &[u8], handle_bits: u64) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.ms(data.len() as u32);
    if ver >= V::R2010 {
        w.mc_unsigned(handle_bits);
    }
    let mut out = w.into_bytes();
    out.extend_from_slice(data);
    let crc = crc16(0xC0C1, &out);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

/// What the encoder needs to know about the drawing beyond the object.
pub(crate) struct Ctx<'a> {
    pub d: &'a Drawing,
    /// The handle of the APPID record of each application name.
    pub appids: &'a std::collections::BTreeMap<String, Handle>,
    /// The "Normal" plot style and "Global" material, for layers.
    pub plot_style: Handle,
    pub material: Handle,
    /// Text style and linetype handles the ByLayer/ByBlock entries need.
    pub dimstyle_vars: &'a dyn Fn(&Object) -> DimVars,
    pub vport: &'a dyn Fn(&Object) -> Vport,
    /// A line of text's width at height 1 (`WriteOptions::text_width`).
    pub text_width: &'a dyn Fn(&str) -> f64,
    /// Line families for a pattern hatch without them
    /// (`WriteOptions::hatch_lines`).
    pub hatch_lines: Option<&'a HatchLibrary<'a>>,
}

fn v3(p: Vec3) -> [f64; 3] {
    p.to_array()
}
fn v2(p: Vec2) -> [f64; 2] {
    [p.x, p.y]
}

/// Extended data (chapter 28), the mirror of `eed::read`.
fn eed(w: &mut ObjW, o: &Object, ctx: &Ctx) -> Result<()> {
    for x in &o.xdata {
        let app = match &x.app {
            XDataApp::Handle(h) => *h,
            XDataApp::Name(n) => match ctx.appids.get(&n.to_ascii_uppercase()) {
                Some(h) => *h,
                None => continue,
            },
        };
        let mut b: Vec<u8> = Vec::new();
        for it in &x.items {
            match it {
                XValue::Str(s) => {
                    b.push(0);
                    if w.ver.has_string_stream() {
                        let units: Vec<u16> = s.encode_utf16().collect();
                        b.extend_from_slice(&(units.len() as u16).to_le_bytes());
                        for u in units {
                            b.extend_from_slice(&u.to_le_bytes());
                        }
                    } else {
                        let bytes: Vec<u8> = s.chars().map(|c| c as u32 as u8).collect();
                        b.push(bytes.len() as u8);
                        b.extend_from_slice(&[0, 30]);
                        b.extend_from_slice(&bytes);
                    }
                }
                XValue::Open => b.extend_from_slice(&[2, 0]),
                XValue::Close => b.extend_from_slice(&[2, 1]),
                XValue::Layer(h) => {
                    b.push(3);
                    b.extend_from_slice(&h.0.to_be_bytes());
                }
                XValue::Bytes(bytes) => {
                    b.push(4);
                    b.push(bytes.len().min(255) as u8);
                    b.extend_from_slice(&bytes[..bytes.len().min(255)]);
                }
                XValue::Handle(h) => {
                    b.push(5);
                    b.extend_from_slice(&h.0.to_be_bytes());
                }
                XValue::Point(p) | XValue::Position(p) | XValue::Displacement(p) | XValue::Direction(p) => {
                    b.push(match it {
                        XValue::Point(_) => 10,
                        XValue::Position(_) => 11,
                        XValue::Displacement(_) => 12,
                        _ => 13,
                    });
                    for c in p.to_array() {
                        b.extend_from_slice(&c.to_le_bytes());
                    }
                }
                XValue::Real(v) | XValue::Distance(v) | XValue::ScaleFactor(v) => {
                    b.push(match it {
                        XValue::Real(_) => 40,
                        XValue::Distance(_) => 41,
                        _ => 42,
                    });
                    b.extend_from_slice(&v.to_le_bytes());
                }
                XValue::Int16(v) => {
                    b.push(70);
                    b.extend_from_slice(&v.to_le_bytes());
                }
                XValue::Int32(v) => {
                    b.push(71);
                    b.extend_from_slice(&v.to_le_bytes());
                }
                XValue::Undecoded { version, data } => {
                    if *version != w.ver && version.has_string_stream() != w.ver.has_string_stream() {
                        return Err(Error::unsupported("extended data from another DWG version could not be decoded"));
                    }
                    b.extend_from_slice(data);
                }
            }
        }
        if b.is_empty() || b.len() > 0x7FFF {
            continue;
        }
        w.dat.bs(b.len() as u16);
        w.dat.handle(5, app.0);
        w.dat.raw_bytes(&b);
    }
    w.dat.bs(0);
    Ok(())
}

/// The type code an object is written with: its own, or for a class
/// object the class number.
fn type_code(o: &Object) -> u16 {
    o.type_code
}

/// The common start of every object: type, handle, extended data, and
/// for an entity its graphics flag and common entity data.
fn common(w: &mut ObjW, o: &Object, ctx: &Ctx) -> Result<()> {
    if w.ver >= V::R2010 {
        w.dat.ot(type_code(o));
    } else {
        w.dat.bs(type_code(o));
    }
    if (V::R2000..=V::R2007).contains(&w.ver) {
        // R2000–R2007 store the data size in bits here; not supported.
        return Err(Error::unsupported("writing R2000–R2007 objects"));
    }
    w.dat.handle(0, o.handle.0);
    eed(w, o, ctx)?;
    let xdic_missing = o.xdictionary.is_null();
    match &o.entity {
        Some(e) => {
            w.dat.b(false); // no proxy graphics
            w.dat.bb(match e.space {
                Space::Owned => 0,
                Space::Paper => 1,
                Space::Model => 2,
            });
            w.dat.bl(o.reactors.len() as u32);
            w.dat.b(xdic_missing);
            if w.ver >= V::R2013 {
                w.dat.b(false);
            }
            enc(w, e.color, e.transparency);
            w.dat.bd(e.linetype_scale);
            w.dat.bb(match e.linetype {
                LinetypeRef::ByLayer => 0,
                LinetypeRef::ByBlock => 1,
                LinetypeRef::Continuous => 2,
                LinetypeRef::Handle(_) => 3,
            });
            w.dat.bb(if e.plotstyle.is_null() { 0 } else { 3 });
            w.dat.bb(if e.material.is_null() { 0 } else { 3 });
            w.dat.rc(0); // shadow flags
            if w.ver >= V::R2010 {
                w.dat.b(false);
                w.dat.b(false);
                w.dat.b(false);
            }
            w.dat.bs(e.invisible as u16);
            w.dat.rc(e.lineweight);
        }
        None => {
            w.dat.bl(o.reactors.len() as u32);
            w.dat.b(xdic_missing);
            if w.ver >= V::R2013 {
                w.dat.b(false);
            }
        }
    }
    Ok(())
}

/// ENC (§2.11, R2004+): BS index with flag bits, then the method word
/// (true colour) and the transparency when present.
fn enc(w: &mut ObjW, c: Color, transparency: Option<u32>) {
    let mut flags: u16 = match c {
        Color::ByLayer => 256,
        Color::ByBlock => 0,
        Color::Index(i) => i as u16,
        Color::Rgb(..) => 0x8000,
        Color::None => 257,
    };
    if transparency.is_some() {
        flags |= 0x2000;
    }
    w.dat.bs(flags);
    if let Color::Rgb(..) = c {
        w.dat.bl(c.to_method_word());
    }
    if let Some(t) = transparency {
        w.dat.bl(t);
    }
}

/// The common handles: owner, reactors, extension dictionary, and for an
/// entity its layer, linetype, material and plot style.
fn common_handles(w: &mut ObjW, o: &Object) {
    let owned = o.entity.as_ref().map(|e| e.space == Space::Owned).unwrap_or(true);
    if owned {
        w.h_rel(o.owner);
    }
    for r in &o.reactors {
        w.h(Ref::SoftPtr, *r);
    }
    if !o.xdictionary.is_null() {
        w.h(Ref::HardOwner, o.xdictionary);
    }
    if let Some(e) = &o.entity {
        w.h(Ref::HardPtr, e.layer);
        if let LinetypeRef::Handle(h) = e.linetype {
            w.h(Ref::HardPtr, h);
        }
        if !e.material.is_null() {
            w.h(Ref::HardPtr, e.material);
        }
        if !e.plotstyle.is_null() {
            w.h(Ref::HardPtr, e.plotstyle);
        }
    }
}

/// A table record's name and xref index (R2007+: no 64-flag or
/// xref-dependent bit; see `tables::record_common_flags`).
fn record_name(w: &mut ObjW, name: &str) {
    w.tv(name);
    if !w.ver.has_string_stream() {
        w.dat.b(false);
    }
    w.dat.bs(0);
    if !w.ver.has_string_stream() {
        w.dat.b(false);
    }
}

/// Encode one object the drawing model holds whole. `Ok(None)` for kinds
/// it does not hold whole (those are written from their DWG record).
pub(crate) fn encode(o: &Object, ctx: &Ctx, ver: DwgVersion) -> Result<Option<Vec<u8>>> {
    let mut w = ObjW::new(ver, o.handle);
    match &o.kind {
        ObjectKind::Unsupported | ObjectKind::Failed(_) | ObjectKind::NewerKind(_) | ObjectKind::Damaged(_) => return Ok(None),
        ObjectKind::NamedRecord(_) if !matches!(o.type_code, 0x41 | 0x43 | 0x45 | 0x39 | 0x35) => return Ok(None),
        _ => {}
    }
    common(&mut w, o, ctx)?;
    specific(&mut w, o, ctx)?;
    Ok(Some(w.finish()))
}

/// Type data, then the handles (common, then the type's own): one function per group of types.
fn specific(w: &mut ObjW, o: &Object, ctx: &Ctx) -> Result<()> {
    match &o.kind {
        ObjectKind::Arc(..)
        | ObjectKind::Attribute(..)
        | ObjectKind::BlockBegin { .. }
        | ObjectKind::BlockEnd
        | ObjectKind::SeqEnd
        | ObjectKind::Circle(..)
        | ObjectKind::Ellipse(..)
        | ObjectKind::Face3d(..)
        | ObjectKind::Insert(..)
        | ObjectKind::Line(..)
        | ObjectKind::Point(..)
        | ObjectKind::Solid(..)
        | ObjectKind::Text(..) => specific_curves(w, o, ctx),
        ObjectKind::LwPolyline(..) | ObjectKind::Polyline(..) | ObjectKind::Vertex(..) => specific_polylines(w, o, ctx),
        ObjectKind::Dimension(..)
        | ObjectKind::Hatch(..)
        | ObjectKind::MText(..)
        | ObjectKind::Spline(..)
        | ObjectKind::Image(..)
        | ObjectKind::Leader(..)
        | ObjectKind::MLine(..)
        | ObjectKind::PaperViewport(..)
        | ObjectKind::Tolerance(..)
        | ObjectKind::Xline(..)
        | ObjectKind::Ray(..)
        | ObjectKind::Wipeout(..) => specific_annotation(w, o, ctx),
        _ => specific_records(w, o, ctx),
    }
}

fn specific_curves(w: &mut ObjW, o: &Object, ctx: &Ctx) -> Result<()> {
    let ver = w.ver;
    let d = ctx.d;
    match &o.kind {
        ObjectKind::Line(l) => {
            let z_zero = l.start.z == 0.0 && l.end.z == 0.0;
            w.dat.b(z_zero);
            w.dat.rd(l.start.x);
            w.dat.dd(l.end.x, l.start.x);
            w.dat.rd(l.start.y);
            w.dat.dd(l.end.y, l.start.y);
            if !z_zero {
                w.dat.rd(l.start.z);
                w.dat.dd(l.end.z, l.start.z);
            }
            w.dat.bt(l.thickness, ver);
            w.dat.be(v3(l.extrusion), ver);
            common_handles(w, o);
        }
        ObjectKind::Circle(c) => {
            w.dat.bd3(v3(c.center));
            w.dat.bd(c.radius);
            w.dat.bt(c.thickness, ver);
            w.dat.be(v3(c.extrusion), ver);
            common_handles(w, o);
        }
        ObjectKind::Arc(a) => {
            w.dat.bd3(v3(a.center));
            w.dat.bd(a.radius);
            w.dat.bt(a.thickness, ver);
            w.dat.be(v3(a.extrusion), ver);
            w.dat.bd(a.start_angle);
            w.dat.bd(a.end_angle);
            common_handles(w, o);
        }
        ObjectKind::Point(p) => {
            w.dat.bd3(v3(p.position));
            w.dat.bt(p.thickness, ver);
            w.dat.be(v3(p.extrusion), ver);
            w.dat.bd(p.x_axis_angle);
            common_handles(w, o);
        }
        ObjectKind::Ellipse(e) => {
            w.dat.bd3(v3(e.center));
            w.dat.bd3(v3(e.major_axis));
            w.dat.bd3(v3(e.extrusion));
            w.dat.bd(e.axis_ratio);
            w.dat.bd(e.start_param);
            w.dat.bd(e.end_param);
            common_handles(w, o);
        }
        ObjectKind::Solid(s) => {
            w.dat.bt(s.thickness, ver);
            w.dat.bd(s.elevation);
            for c in s.corners {
                w.dat.rd2(v2(c));
            }
            w.dat.be(v3(s.extrusion), ver);
            common_handles(w, o);
        }
        ObjectKind::Face3d(f) => {
            let no_flags = f.invisible_edges == 0;
            let z_zero = f.corners[0].z == 0.0;
            w.dat.b(no_flags);
            w.dat.b(z_zero);
            w.dat.rd(f.corners[0].x);
            w.dat.rd(f.corners[0].y);
            if !z_zero {
                w.dat.rd(f.corners[0].z);
            }
            for i in 1..4 {
                let (p, c) = (f.corners[i - 1], f.corners[i]);
                w.dat.dd(c.x, p.x);
                w.dat.dd(c.y, p.y);
                w.dat.dd(c.z, p.z);
            }
            if !no_flags {
                w.dat.bs(f.invisible_edges);
            }
            common_handles(w, o);
        }
        ObjectKind::Text(t) => {
            text_data(w, t);
            common_handles(w, o);
            w.h(Ref::HardPtr, t.style);
        }
        ObjectKind::Attribute(a) => {
            text_data(w, &a.text);
            if ver >= V::R2010 {
                w.dat.rc(0);
            }
            if ver >= V::R2018 {
                w.dat.rc(1); // single-line
            }
            w.tv(&a.tag);
            w.dat.bs(a.field_length);
            w.dat.rc(a.flags);
            if ver >= V::R2007 {
                w.dat.b(a.lock_position);
            }
            if a.is_definition {
                if ver >= V::R2010 {
                    w.dat.rc(0);
                }
                w.tv(&a.prompt);
            }
            common_handles(w, o);
            w.h(Ref::HardPtr, a.text.style);
        }
        ObjectKind::BlockBegin { name } => {
            w.tv(name);
            common_handles(w, o);
        }
        ObjectKind::BlockEnd | ObjectKind::SeqEnd => common_handles(w, o),
        ObjectKind::Insert(ins) => {
            w.dat.bd3(v3(ins.insertion));
            let s = ins.scale;
            if s == Vec3::new(1.0, 1.0, 1.0) {
                w.dat.bb(3);
            } else if s.x == 1.0 {
                w.dat.bb(1);
                w.dat.dd(s.y, 1.0);
                w.dat.dd(s.z, 1.0);
            } else if s.y == s.x && s.z == s.x {
                w.dat.bb(2);
                w.dat.rd(s.x);
            } else {
                w.dat.bb(0);
                w.dat.rd(s.x);
                w.dat.dd(s.y, s.x);
                w.dat.dd(s.z, s.x);
            }
            w.dat.bd(ins.rotation);
            w.dat.bd3(v3(ins.extrusion));
            w.dat.b(ins.has_attribs);
            let attribs: Vec<Handle> =
                if ins.attribs.is_empty() { d.insert_attributes(o).iter().map(|a| a.handle).collect() } else { ins.attribs.clone() };
            if ins.has_attribs {
                w.dat.bl(attribs.len() as u32);
            }
            if let Some((cols, rows, cs, rs)) = ins.array {
                w.dat.bs(cols);
                w.dat.bs(rows);
                w.dat.bd(cs);
                w.dat.bd(rs);
            }
            common_handles(w, o);
            w.h(Ref::HardPtr, ins.block_header);
            if ins.has_attribs {
                for a in attribs {
                    w.h(Ref::HardOwner, a);
                }
                w.h(Ref::HardOwner, ins.seqend);
            }
        }
        other => {
            return Err(Error::unsupported(format!("{} {}: writing {:?} is not supported", o.type_name, o.handle, std::mem::discriminant(other))));
        }
    }
    Ok(())
}

fn specific_polylines(w: &mut ObjW, o: &Object, ctx: &Ctx) -> Result<()> {
    let ver = w.ver;
    let d = ctx.d;
    match &o.kind {
        ObjectKind::Vertex(v) => {
            match v.kind {
                VertexKind::Vertex2d => {
                    w.dat.rc(v.flags);
                    w.dat.bd3(v3(v.point));
                    if v.start_width == v.end_width && v.start_width != 0.0 {
                        w.dat.bd(-v.start_width);
                    } else {
                        w.dat.bd(v.start_width);
                        w.dat.bd(v.end_width);
                    }
                    w.dat.bd(v.bulge);
                    if ver >= V::R2010 {
                        w.dat.bl(v.id);
                    }
                    w.dat.bd(v.tangent_dir);
                }
                VertexKind::PfaceFace => {
                    for f in v.face {
                        w.dat.bs(f as u16);
                    }
                }
                _ => {
                    w.dat.rc(v.flags);
                    w.dat.bd3(v3(v.point));
                }
            }
            common_handles(w, o);
        }
        ObjectKind::Polyline(p) => {
            let verts: Vec<Handle> = if p.vertices.is_empty() {
                d.owned_by(o.handle).filter(|c| matches!(c.kind, ObjectKind::Vertex(_))).map(|c| c.handle).collect()
            } else {
                p.vertices.clone()
            };
            match p.kind {
                PolylineKind::Polyline2d => {
                    w.dat.bs(p.flags);
                    w.dat.bs(p.curve_type);
                    w.dat.bd(p.start_width);
                    w.dat.bd(p.end_width);
                    w.dat.bt(p.thickness, ver);
                    w.dat.bd(p.elevation);
                    w.dat.be(v3(p.extrusion), ver);
                }
                PolylineKind::Polyline3d => {
                    w.dat.rc(p.curve_type as u8);
                    w.dat.rc((p.flags & 1) as u8);
                }
                PolylineKind::PolyfaceMesh => {
                    let all = d.polyline_vertices(o);
                    let nv = all.iter().filter(|v| v.kind == VertexKind::PfaceVertex).count();
                    w.dat.bs(nv as u16);
                    w.dat.bs((all.len() - nv) as u16);
                }
                PolylineKind::PolygonMesh => {
                    w.dat.bs(p.flags & !16);
                    w.dat.bs(p.curve_type);
                    for _ in 0..4 {
                        w.dat.bs(0);
                    }
                }
            }
            w.dat.bl(verts.len() as u32);
            common_handles(w, o);
            for v in verts {
                w.h(Ref::HardOwner, v);
            }
            w.h(Ref::HardOwner, p.seqend);
        }
        ObjectKind::LwPolyline(p) => {
            let mut flags = p.flags & !(1 | 2 | 4 | 8 | 16 | 32 | 1024);
            if p.extrusion != Vec3::Z {
                flags |= 1;
            }
            if p.thickness != 0.0 {
                flags |= 2;
            }
            if p.const_width != 0.0 {
                flags |= 4;
            }
            if p.elevation != 0.0 {
                flags |= 8;
            }
            let bulges = p.bulges.iter().any(|b| *b != 0.0);
            if bulges {
                flags |= 16;
            }
            if !p.widths.is_empty() {
                flags |= 32;
            }
            if !p.vertex_ids.is_empty() {
                flags |= 1024;
            }
            w.dat.bs(flags);
            if flags & 4 != 0 {
                w.dat.bd(p.const_width);
            }
            if flags & 8 != 0 {
                w.dat.bd(p.elevation);
            }
            if flags & 2 != 0 {
                w.dat.bd(p.thickness);
            }
            if flags & 1 != 0 {
                w.dat.bd3(v3(p.extrusion));
            }
            w.dat.bl(p.points.len() as u32);
            if bulges {
                w.dat.bl(p.points.len() as u32);
            }
            if flags & 1024 != 0 {
                w.dat.bl(p.vertex_ids.len() as u32);
            }
            if flags & 32 != 0 {
                w.dat.bl(p.widths.len() as u32);
            }
            let mut prev: Option<Vec2> = None;
            for pt in &p.points {
                match prev {
                    None => w.dat.rd2(v2(*pt)),
                    Some(q) => {
                        w.dat.dd(pt.x, q.x);
                        w.dat.dd(pt.y, q.y);
                    }
                }
                prev = Some(*pt);
            }
            if bulges {
                for i in 0..p.points.len() {
                    w.dat.bd(p.bulge(i));
                }
            }
            for id in &p.vertex_ids {
                w.dat.bl(*id);
            }
            for (a, b) in &p.widths {
                w.dat.bd(*a);
                w.dat.bd(*b);
            }
            common_handles(w, o);
        }
        other => {
            return Err(Error::unsupported(format!("{} {}: writing {:?} is not supported", o.type_name, o.handle, std::mem::discriminant(other))));
        }
    }
    Ok(())
}

fn specific_annotation(w: &mut ObjW, o: &Object, ctx: &Ctx) -> Result<()> {
    let d = ctx.d;
    let ver = w.ver;
    match &o.kind {
        ObjectKind::Spline(sp) => {
            spline_data(w, sp);
            common_handles(w, o);
        }
        ObjectKind::MText(m) => {
            mtext_data(w, m, ctx.text_width);
            common_handles(w, o);
            w.h(Ref::HardPtr, m.style);
            if ver >= V::R2018 {
                w.h(Ref::HardPtr, Handle::NULL); // registered application
            }
        }
        ObjectKind::Dimension(m) => {
            dimension_data(w, o, m)?;
            common_handles(w, o);
            w.h(Ref::HardPtr, m.dimstyle);
            w.h(Ref::HardPtr, m.block);
        }
        ObjectKind::Hatch(h) => {
            hatch_data(w, h, d, ctx.hatch_lines);
            common_handles(w, o);
            for i in 0..h.paths.len() {
                for s in h.sources.get(i).map(Vec::as_slice).unwrap_or_default() {
                    w.h(Ref::SoftPtr, *s);
                }
            }
        }
        ObjectKind::Xline(x) | ObjectKind::Ray(x) => {
            w.dat.bd3(v3(x.base));
            w.dat.bd3(v3(x.direction));
            common_handles(w, o);
        }
        ObjectKind::Leader(l) => {
            leader_data(w, l);
            common_handles(w, o);
            w.h(Ref::HardPtr, l.annotation);
            w.h(Ref::HardPtr, l.dimstyle);
        }
        ObjectKind::MLine(m) => {
            // §20.4.50, the mirror of `entities::mline`.
            w.dat.bd(m.scale);
            w.dat.rc(m.justification);
            w.dat.bd3(v3(m.base));
            w.dat.bd3(v3(if m.extrusion == Vec3::ZERO { Vec3::Z } else { m.extrusion }));
            w.dat.bs(m.flags);
            w.dat.rc(m.lines_in_style);
            w.dat.bs(m.vertices.len().min(usize::from(u16::MAX)) as u16);
            for v in m.vertices.iter().take(usize::from(u16::MAX)) {
                w.dat.bd3(v3(v.point));
                w.dat.bd3(v3(v.direction));
                w.dat.bd3(v3(v.miter));
                // Exactly one parameter list per element of the style.
                for i in 0..usize::from(m.lines_in_style) {
                    let (seg, fill) = v.params.get(i).map(|(a, b)| (a.as_slice(), b.as_slice())).unwrap_or((&[], &[]));
                    w.dat.bs(seg.len().min(usize::from(u16::MAX)) as u16);
                    for x in seg.iter().take(usize::from(u16::MAX)) {
                        w.dat.bd(*x);
                    }
                    w.dat.bs(fill.len().min(usize::from(u16::MAX)) as u16);
                    for x in fill.iter().take(usize::from(u16::MAX)) {
                        w.dat.bd(*x);
                    }
                }
            }
            common_handles(w, o);
            w.h(Ref::HardPtr, m.style);
        }
        ObjectKind::Image(r) | ObjectKind::Wipeout(r) => {
            // §20.4.80, the mirror of `entities::raster_image`.
            w.dat.bl(r.class_version);
            w.dat.bd3(v3(r.insertion));
            w.dat.bd3(v3(r.u));
            w.dat.bd3(v3(r.v));
            w.dat.rd2(v2(r.size));
            w.dat.bs(r.display);
            w.dat.b(r.clipping);
            w.dat.rc(r.brightness);
            w.dat.rc(r.contrast);
            w.dat.rc(r.fade);
            if ver >= V::R2010 {
                w.dat.b(r.clip_inside);
            }
            // A rectangle is two corners; anything else is a polygon. No
            // boundary at all is the whole image, pixel centres at whole
            // numbers (DXF reference, IMAGE group 14).
            let whole = [Vec2::new(-0.5, -0.5), Vec2::new(r.size.x - 0.5, r.size.y - 0.5)];
            let clip: &[Vec2] = if r.clip.is_empty() { &whole } else { &r.clip };
            let rect = clip.len() == 2 && (r.clip_type == 1 || r.clip.is_empty());
            w.dat.bs(if rect { 1 } else { 2 });
            if !rect {
                w.dat.bl(clip.len() as u32);
            }
            for p in clip {
                w.dat.rd2(v2(*p));
            }
            common_handles(w, o);
            w.h(Ref::HardPtr, r.definition);
            w.h(Ref::HardOwner, r.reactor);
        }
        ObjectKind::Tolerance(t) => {
            // §20.4.49, the mirror of `entities::tolerance`.
            if ver <= V::R14 {
                let (short, height, gap) = t.r14.unwrap_or((0, 0.0, 0.0));
                w.dat.bs(short);
                w.dat.bd(height);
                w.dat.bd(gap);
            }
            w.dat.bd3(v3(t.insertion));
            w.dat.bd3(v3(t.x_direction));
            w.dat.bd3(v3(if t.extrusion == Vec3::ZERO { Vec3::Z } else { t.extrusion }));
            w.tv(&t.text);
            common_handles(w, o);
            w.h(Ref::HardPtr, t.dimstyle);
        }
        ObjectKind::PaperViewport(v) => {
            viewport_data(w, v);
            common_handles(w, o);
            if ver <= V::R14 {
                w.h(Ref::HardPtr, Handle::NULL);
            } else {
                for h in &v.frozen_layers {
                    w.h(if ver >= V::R2004 { Ref::SoftPtr } else { Ref::HardPtr }, *h);
                }
                w.h(Ref::SoftPtr, v.clip_boundary);
                if ver == V::R2000 {
                    w.h(Ref::HardPtr, Handle::NULL);
                }
                w.h(Ref::HardPtr, v.named_ucs);
                w.h(Ref::HardPtr, v.base_ucs);
                if ver >= V::R2007 {
                    w.h(Ref::SoftPtr, v.background);
                    w.h(Ref::HardPtr, v.visual_style);
                    w.h(Ref::SoftPtr, v.shade_plot);
                    w.h(Ref::HardOwner, v.sun);
                }
            }
        }
        other => {
            return Err(Error::unsupported(format!("{} {}: writing {:?} is not supported", o.type_name, o.handle, std::mem::discriminant(other))));
        }
    }
    Ok(())
}

fn specific_records(w: &mut ObjW, o: &Object, ctx: &Ctx) -> Result<()> {
    let ver = w.ver;
    let d = ctx.d;
    match &o.kind {
        ObjectKind::Control(c) => {
            w.dat.bl(c.entries.len() as u32);
            // DIMSTYLE_CONTROL (R2000+) adds a count and as many hard
            // pointers; AutoCAD 2027 points them at the styles again.
            if o.type_code == 0x44 && ver >= V::R2000 {
                w.dat.rc(c.entries.len().min(255) as u8);
            }
            common_handles(w, o);
            for e in &c.entries {
                w.h(Ref::SoftOwner, *e);
            }
            if o.type_code == 0x44 && ver >= V::R2000 {
                for e in c.entries.iter().take(255) {
                    w.h(Ref::HardPtr, *e);
                }
            }
            match o.type_code {
                0x30 => {
                    w.h(Ref::HardOwner, c.model_space);
                    w.h(Ref::HardOwner, c.paper_space);
                }
                0x38 => {
                    let hv = crate::read::hvars::Hv::new(d);
                    w.h(Ref::HardOwner, hv.handle("LTYPE_BYBLOCK"));
                    w.h(Ref::HardOwner, hv.handle("LTYPE_BYLAYER"));
                }
                _ => {}
            }
        }
        ObjectKind::BlockHeader(b) => {
            block_header(w, o, b, d);
        }
        ObjectKind::Layer(l) => {
            record_name(w, &l.name);
            // Off is the flag word's 0x02 bit (clear): from R2004 the
            // colour's index field, whose sign once marked it, is 0.
            w.dat.bs(l.dwg_flags());
            w.cmc(l.color);
            common_handles(w, o);
            w.h(Ref::HardPtr, Handle::NULL); // xref block
            w.h(Ref::HardPtr, ctx.plot_style);
            w.h(Ref::HardPtr, ctx.material);
            w.h(Ref::HardPtr, l.linetype);
            if ver >= V::R2013 {
                w.h(Ref::HardPtr, Handle::NULL);
            }
        }
        ObjectKind::Linetype(l) => {
            record_name(w, &l.name);
            linetype_body(w, l);
            common_handles(w, o);
            w.h(Ref::HardPtr, Handle::NULL);
            for e in &l.elements {
                w.h(Ref::HardPtr, e.style);
            }
        }
        ObjectKind::NamedRecord(r) if o.type_code == 0x39 => {
            record_name(w, &r.name);
            linetype_body(w, &Linetype::default());
            common_handles(w, o);
            w.h(Ref::HardPtr, Handle::NULL);
        }
        ObjectKind::TextStyle(t) => {
            record_name(w, &t.name);
            text_style_body(w, t);
            common_handles(w, o);
            w.h(Ref::HardPtr, Handle::NULL);
        }
        ObjectKind::NamedRecord(r) if o.type_code == 0x35 => {
            record_name(w, &r.name);
            text_style_body(w, &TextStyle { name: r.name.clone(), last_height: 0.2, font_file: "txt".into(), ..Default::default() });
            common_handles(w, o);
            w.h(Ref::HardPtr, Handle::NULL);
        }
        ObjectKind::NamedRecord(r) if o.type_code == 0x43 => {
            record_name(w, &r.name);
            w.dat.rc(0);
            common_handles(w, o);
            w.h(Ref::HardPtr, Handle::NULL);
        }
        ObjectKind::NamedRecord(_) | ObjectKind::DimStyle(_) | ObjectKind::DimStyleRecord(_) if o.type_code == 0x45 => {
            let name = d.record_name(o.handle).unwrap_or("").to_string();
            let v = (ctx.dimstyle_vars)(o);
            record_name(w, &name);
            dimstyle_body(w, &v);
            common_handles(w, o);
            w.h(Ref::HardPtr, Handle::NULL);
            for h in [v.txsty, v.ldrblk, v.blk, v.blk1, v.blk2] {
                w.h(Ref::HardPtr, h);
            }
            if ver >= V::R2007 {
                for h in [v.ltype, v.ltex1, v.ltex2] {
                    w.h(Ref::HardPtr, h);
                }
            }
        }
        ObjectKind::NamedRecord(_) | ObjectKind::Viewport(_) if o.type_code == 0x41 => {
            let v = (ctx.vport)(o);
            record_name(w, &v.name);
            vport_body(w, &v);
            common_handles(w, o);
            w.h(Ref::HardPtr, Handle::NULL);
            if ver >= V::R2007 {
                w.h(Ref::SoftPtr, v.background);
                w.h(Ref::HardPtr, v.visual_style);
                w.h(Ref::HardOwner, v.sun);
            }
            w.h(Ref::HardPtr, v.named_ucs);
            w.h(Ref::HardPtr, v.base_ucs);
        }
        ObjectKind::NonGraphic(NonGraphic::Groups(g)) => {
            common_handles(w, o);
            crate::read::styles::encode(w, g, d)?;
        }
        ObjectKind::NonGraphic(n) => nongraphic(w, o, n)?,
        other => {
            return Err(Error::unsupported(format!("{} {}: writing {:?} is not supported", o.type_name, o.handle, std::mem::discriminant(other))));
        }
    }
    Ok(())
}

fn text_data(w: &mut ObjW, t: &Text) {
    let ver = w.ver;
    let mut flags = 0u8;
    if t.elevation == 0.0 {
        flags |= 0x01;
    }
    if t.alignment == t.insertion {
        flags |= 0x02;
    }
    if t.oblique == 0.0 {
        flags |= 0x04;
    }
    if t.rotation == 0.0 {
        flags |= 0x08;
    }
    if t.width_factor == 1.0 {
        flags |= 0x10;
    }
    if t.generation == 0 {
        flags |= 0x20;
    }
    if t.h_align == 0 {
        flags |= 0x40;
    }
    if t.v_align == 0 {
        flags |= 0x80;
    }
    w.dat.rc(flags);
    if flags & 0x01 == 0 {
        w.dat.rd(t.elevation);
    }
    w.dat.rd2(v2(t.insertion));
    if flags & 0x02 == 0 {
        w.dat.dd(t.alignment.x, t.insertion.x);
        w.dat.dd(t.alignment.y, t.insertion.y);
    }
    // AutoCAD writes a text's extrusion in full, even (0, 0, 1).
    w.dat.b(false);
    w.dat.bd3(v3(t.extrusion));
    w.dat.bt(t.thickness, ver);
    if flags & 0x04 == 0 {
        w.dat.rd(t.oblique);
    }
    if flags & 0x08 == 0 {
        w.dat.rd(t.rotation);
    }
    w.dat.rd(t.height);
    if flags & 0x10 == 0 {
        w.dat.rd(t.width_factor);
    }
    w.tv(&t.value);
    if flags & 0x20 == 0 {
        w.dat.bs(t.generation);
    }
    if flags & 0x40 == 0 {
        w.dat.bs(t.h_align);
    }
    if flags & 0x80 == 0 {
        w.dat.bs(t.v_align);
    }
}

fn mtext_data(w: &mut ObjW, m: &MText, text_width: &dyn Fn(&str) -> f64) {
    let ver = w.ver;
    w.dat.bd3(v3(m.insertion));
    w.dat.bd3(v3(m.extrusion));
    w.dat.bd3(v3(m.x_axis));
    w.dat.bd(m.rect_width);
    if ver >= V::R2007 {
        w.dat.bd(m.rect_height);
    }
    w.dat.bd(m.text_height);
    w.dat.bs(m.attachment);
    w.dat.bs(m.drawing_dir);
    let (ext_h, ext_w) = mtext_extents(m, text_width);
    w.dat.bd(ext_h);
    w.dat.bd(ext_w);
    w.tv(&m.contents);
    w.dat.bs(if m.linespacing_exact { 2 } else { 1 }); // 1 at least, 2 exactly
    w.dat.bd(m.linespacing_factor);
    w.dat.b(false);
    if ver >= V::R2004 {
        w.dat.bl(0); // no background
    }
    if ver >= V::R2018 {
        w.dat.b(true); // not annotative
        w.dat.bs(4);
        w.dat.b(true);
        w.dat.bl(m.attachment as u32);
        w.dat.bd3(v3(m.x_axis));
        w.dat.bd3(v3(m.insertion));
        w.dat.bd(m.rect_width);
        w.dat.bd(m.rect_height);
        w.dat.bd(ext_w);
        w.dat.bd(ext_h);
        w.dat.bs(0); // no columns
    }
}

/// The text's extents (height, width) as the writing program's font sets
/// it: one line per paragraph. AutoCAD stores its own font's extents here
/// and measures again when it regenerates the text.
fn mtext_extents(m: &MText, text_width: &dyn Fn(&str) -> f64) -> (f64, f64) {
    let plain = crate::read::text::strip_mtext(&m.contents);
    let lines: Vec<&str> = plain.split('\n').collect();
    let n = lines.len().max(1) as f64;
    let widest = lines.iter().map(|l| text_width(l)).fold(0.0, f64::max);
    let h = m.text_height * (1.0 + (n - 1.0) * 1.6667 * m.linespacing_factor);
    (h, widest * m.text_height)
}

fn dimension_data(w: &mut ObjW, o: &Object, m: &Dimension) -> Result<()> {
    let ver = w.ver;
    if ver >= V::R2010 {
        w.dat.rc(0);
    }
    w.dat.bd3(v3(m.extrusion));
    w.dat.rd2(v2(m.text_midpoint));
    w.dat.bd(m.elevation);
    w.dat.rc(m.flags);
    w.tv(&m.user_text);
    w.dat.bd(m.text_rotation);
    w.dat.bd(m.horizontal_direction);
    w.dat.bd3(v3(m.insert_scale));
    w.dat.bd(m.insert_rotation);
    w.dat.bs(m.attachment);
    w.dat.bs(1);
    w.dat.bd(1.0);
    w.dat.bd(m.measurement);
    if ver >= V::R2007 {
        w.dat.b(false);
        w.dat.b(false);
        w.dat.b(false);
    }
    w.dat.rd2(v2(m.clone_insertion));
    let code = o.type_code;
    match m.kind {
        DimensionKind::Ordinate => {
            w.dat.bd3(v3(m.pt10));
            w.dat.bd3(v3(m.pt13));
            w.dat.bd3(v3(m.pt14));
            w.dat.rc(0);
        }
        DimensionKind::Linear => {
            w.dat.bd3(v3(m.pt13));
            w.dat.bd3(v3(m.pt14));
            w.dat.bd3(v3(m.pt10));
            w.dat.bd(m.ext_line_rotation);
            w.dat.bd(m.dim_rotation);
        }
        DimensionKind::Aligned => {
            w.dat.bd3(v3(m.pt13));
            w.dat.bd3(v3(m.pt14));
            w.dat.bd3(v3(m.pt10));
            w.dat.bd(m.ext_line_rotation);
        }
        DimensionKind::Angular3Pt => {
            w.dat.bd3(v3(m.pt10));
            w.dat.bd3(v3(m.pt13));
            w.dat.bd3(v3(m.pt14));
            w.dat.bd3(v3(m.pt15));
        }
        DimensionKind::Angular2Line => {
            w.dat.rd2(v2(m.pt16));
            w.dat.bd3(v3(m.pt13));
            w.dat.bd3(v3(m.pt14));
            w.dat.bd3(v3(m.pt15));
            w.dat.bd3(v3(m.pt10));
        }
        DimensionKind::Radius | DimensionKind::Diameter => {
            w.dat.bd3(v3(m.pt10));
            w.dat.bd3(v3(m.pt15));
            w.dat.bd(m.leader_length);
        }
    }
    let _ = code;
    Ok(())
}

fn spline_data(w: &mut ObjW, sp: &Spline) {
    let ver = w.ver;
    let fit = !sp.fit_points.is_empty() && sp.control_points.is_empty();
    let scenario = if fit { 2 } else { 1 };
    w.dat.bl(scenario);
    if ver >= V::R2013 {
        let mut flags1 = 0u32;
        if sp.closed {
            flags1 |= 4;
        }
        if fit {
            flags1 |= 1;
        }
        w.dat.bl(flags1);
        w.dat.bl(0);
    }
    w.dat.bl(sp.degree);
    if fit {
        w.dat.bd(sp.fit_tolerance);
        w.dat.bd3(v3(sp.start_tangent));
        w.dat.bd3(v3(sp.end_tangent));
        w.dat.bl(sp.fit_points.len() as u32);
    } else {
        w.dat.b(sp.rational);
        w.dat.b(sp.closed);
        w.dat.b(sp.periodic);
        w.dat.bd(1e-10);
        w.dat.bd(1e-10);
        w.dat.bl(sp.knots.len() as u32);
        w.dat.bl(sp.control_points.len() as u32);
        w.dat.b(!sp.weights.is_empty());
    }
    if !fit {
        for k in &sp.knots {
            w.dat.bd(*k);
        }
        for (i, p) in sp.control_points.iter().enumerate() {
            w.dat.bd3(v3(*p));
            if !sp.weights.is_empty() {
                w.dat.bd(sp.weights.get(i).copied().unwrap_or(1.0));
            }
        }
    } else {
        for p in &sp.fit_points {
            w.dat.bd3(v3(*p));
        }
    }
}

fn hatch_data(w: &mut ObjW, h: &Hatch, d: &Drawing, library: Option<&HatchLibrary<'_>>) {
    let ver = w.ver;
    if ver >= V::R2004 {
        w.dat.bl(h.is_gradient as u32);
        w.dat.bl(0);
        w.dat.bd(0.0);
        w.dat.bd(0.0);
        w.dat.bl(0);
        w.dat.bd(0.0);
        w.dat.bl(0);
        w.tv("");
    }
    w.dat.bd(h.elevation);
    w.dat.bd3(v3(h.extrusion));
    w.tv(&h.pattern_name);
    w.dat.b(h.solid_fill);
    w.dat.b(h.associative);
    w.dat.bl(h.paths.len() as u32);
    for (i, p) in h.paths.iter().enumerate() {
        match p {
            HatchPath::Edges { flags, edges } => {
                // Bit 4 (made by picking) would call for a pixel size,
                // which the model does not keep.
                w.dat.bl(*flags & !(2 | 4));
                w.dat.bl(edges.len() as u32);
                for e in edges {
                    match e {
                        HatchEdge::Line { a, b } => {
                            w.dat.rc(1);
                            w.dat.rd2(v2(*a));
                            w.dat.rd2(v2(*b));
                        }
                        HatchEdge::Arc { center, radius, start, end, ccw } => {
                            w.dat.rc(2);
                            w.dat.rd2(v2(*center));
                            w.dat.bd(*radius);
                            w.dat.bd(*start);
                            w.dat.bd(*end);
                            w.dat.b(*ccw);
                        }
                        HatchEdge::Ellipse { center, major, ratio, start, end, ccw } => {
                            w.dat.rc(3);
                            w.dat.rd2(v2(*center));
                            w.dat.rd2(v2(*major));
                            w.dat.bd(*ratio);
                            w.dat.bd(*start);
                            w.dat.bd(*end);
                            w.dat.b(*ccw);
                        }
                        HatchEdge::Spline { degree, knots, control_points, weights } => {
                            w.dat.rc(4);
                            w.dat.bl(*degree);
                            w.dat.b(!weights.is_empty());
                            w.dat.b(false);
                            w.dat.bl(knots.len() as u32);
                            w.dat.bl(control_points.len() as u32);
                            for k in knots {
                                w.dat.bd(*k);
                            }
                            for (i, c) in control_points.iter().enumerate() {
                                w.dat.rd2(v2(*c));
                                if !weights.is_empty() {
                                    w.dat.bd(weights.get(i).copied().unwrap_or(1.0));
                                }
                            }
                            if ver >= V::R2010 {
                                w.dat.bl(0);
                            }
                        }
                    }
                }
            }
            HatchPath::Polyline { flags, closed, vertices } => {
                w.dat.bl((*flags | 2) & !4);
                let bulges = vertices.iter().any(|(_, b)| *b != 0.0);
                w.dat.b(bulges);
                w.dat.b(*closed);
                w.dat.bl(vertices.len() as u32);
                for (p, b) in vertices {
                    w.dat.rd2(v2(*p));
                    if bulges {
                        w.dat.bd(*b);
                    }
                }
            }
        }
        // Boundary objects; their handles follow the common handles.
        w.dat.bl(h.sources.get(i).map_or(0, Vec::len) as u32);
    }
    w.dat.bs(h.style);
    w.dat.bs(h.pattern_type);
    if !h.solid_fill {
        w.dat.bd(h.pattern_angle);
        w.dat.bd(h.pattern_scale);
        w.dat.b(h.pattern_double);
        // The line families: as stored, or from the program's library for
        // a hatch that has none, so other programs draw the same pattern.
        let lines = match library {
            Some(library) if h.pattern_lines.is_empty() => library(d, h),
            _ => h.pattern_lines.clone(),
        };
        w.dat.bs(lines.len().min(u16::MAX as usize) as u16);
        for l in lines.iter().take(u16::MAX as usize) {
            w.dat.bd(l.angle);
            w.dat.bd2(v2(l.base));
            w.dat.bd2(v2(l.offset));
            w.dat.bs(l.dashes.len().min(u16::MAX as usize) as u16);
            for d in l.dashes.iter().take(u16::MAX as usize) {
                w.dat.bd(*d);
            }
        }
    }
    w.dat.bl(h.seed_points.len() as u32);
    for p in &h.seed_points {
        w.dat.rd2(v2(*p));
    }
}

/// LEADER (§20.4.47), the mirror of `entities::leader`.
fn leader_data(w: &mut ObjW, l: &Leader) {
    let ver = w.ver;
    w.dat.b(l.unknown.0);
    w.dat.bs(l.annotation_type);
    w.dat.bs(l.spline as u16);
    w.dat.bl(l.vertices.len() as u32);
    for p in &l.vertices {
        w.dat.bd3(v3(*p));
    }
    w.dat.bd3(v3(l.origin));
    w.dat.bd3(v3(if l.extrusion == Vec3::ZERO { Vec3::Z } else { l.extrusion }));
    w.dat.bd3(v3(l.x_direction));
    w.dat.bd3(v3(l.block_offset));
    if ver >= V::R14 {
        w.dat.bd3(v3(l.annotation_offset));
    }
    if ver <= V::R14 {
        w.dat.bd(0.0);
    }
    w.dat.bd(l.text_height);
    w.dat.bd(l.text_width);
    w.dat.b(l.hookline_on_x);
    w.dat.b(l.arrowhead);
    if ver <= V::R14 {
        w.dat.bs(0);
        w.dat.bd(0.0);
        w.dat.b(false);
        w.dat.b(false);
        w.dat.bs(0);
        w.dat.bs(0);
        w.dat.b(false);
        w.dat.b(false);
    } else {
        w.dat.bs(l.unknown.1);
        w.dat.b(l.unknown.2);
        w.dat.b(l.unknown.3);
    }
}

/// VIEWPORT (§20.4.38), the mirror of `entities::viewport`.
fn viewport_data(w: &mut ObjW, v: &PaperViewport) {
    let ver = w.ver;
    w.dat.bd3(v3(v.center));
    w.dat.bd(v.width);
    w.dat.bd(v.height);
    if ver >= V::R2000 {
        w.dat.bd3(v3(v.view_target));
        w.dat.bd3(v3(v.view_direction));
        w.dat.bd(v.twist);
        w.dat.bd(v.view_height);
        w.dat.bd(v.lens);
        w.dat.bd(v.front_clip);
        w.dat.bd(v.back_clip);
        w.dat.bd(v.snap_angle);
        w.dat.rd2(v2(v.view_center));
        w.dat.rd2(v2(v.snap_base));
        w.dat.rd2(v2(v.snap_spacing));
        w.dat.rd2(v2(v.grid_spacing));
        w.dat.bs(v.circle_zoom);
        if ver >= V::R2007 {
            w.dat.bs(v.grid_major);
        }
        w.dat.bl(v.frozen_layers.len() as u32);
        w.dat.bl(v.status);
        w.tv(&v.style_sheet);
        w.dat.rc(v.render_mode);
        w.dat.b(v.ucs_at_origin);
        w.dat.b(v.ucs_per_viewport);
        w.dat.bd3(v3(v.ucs_origin));
        w.dat.bd3(v3(v.ucs_x));
        w.dat.bd3(v3(v.ucs_y));
        w.dat.bd(v.ucs_elevation);
        w.dat.bs(v.ucs_ortho_type);
        if ver >= V::R2004 {
            w.dat.bs(v.shade_plot_mode);
        }
        if ver >= V::R2007 {
            w.dat.b(v.default_lights);
            w.dat.rc(v.lighting_type);
            w.dat.bd(v.brightness);
            w.dat.bd(v.contrast);
            w.cmc(v.ambient);
        }
    }
}

fn block_header(w: &mut ObjW, o: &Object, b: &BlockHeader, d: &Drawing) {
    let ver = w.ver;
    record_name(w, &b.name);
    w.dat.b(b.anonymous);
    w.dat.b(b.has_attribs);
    w.dat.b(b.is_xref);
    w.dat.b(b.is_overlaid);
    w.dat.b(false); // loaded
    let lists = !b.is_xref && !b.is_overlaid;
    let entities: Vec<Handle> =
        if !b.entities.is_empty() { b.entities.clone() } else { d.block_entities(o.handle).iter().map(|e| e.handle).collect() };
    if ver >= V::R2004 && lists {
        w.dat.bl(entities.len() as u32);
    }
    w.dat.bd3(v3(b.base_point));
    w.tv(&b.xref_path);
    let inserts: Vec<Handle> =
        d.objects.values().filter(|i| matches!(&i.kind, ObjectKind::Insert(ins) if ins.block_header == o.handle)).map(|i| i.handle).collect();
    for _ in &inserts {
        w.dat.rc(1);
    }
    w.dat.rc(0);
    w.tv(&b.description);
    w.dat.bl(0); // no preview
    if ver >= V::R2007 {
        w.dat.bs(b.insert_units);
        w.dat.b(!b.not_explodable);
        w.dat.rc(0); // scaling: any
    }
    common_handles(w, o);
    w.h(Ref::HardPtr, Handle::NULL);
    w.h(Ref::HardOwner, b.block_entity);
    if lists {
        for e in &entities {
            w.h(Ref::HardOwner, *e);
        }
    }
    w.h(Ref::HardOwner, b.endblk);
    for i in inserts {
        w.h(Ref::SoftPtr, i);
    }
    w.h(Ref::HardPtr, b.layout);
}

fn linetype_body(w: &mut ObjW, l: &Linetype) {
    w.tv(&l.description);
    w.dat.bd(l.pattern_length);
    w.dat.rc(b'A');
    w.dat.rc(l.elements.len() as u8);
    let has_text = l.elements.iter().any(|e| e.complex & 2 != 0);
    let mut area = vec![0u8; 512];
    let mut at = 0usize;
    for e in &l.elements {
        w.dat.bd(e.length);
        let shape = if e.complex & 2 != 0 {
            let pos = at;
            for u in e.text.encode_utf16() {
                if at + 2 < area.len() {
                    area[at..at + 2].copy_from_slice(&u.to_le_bytes());
                    at += 2;
                }
            }
            at += 2;
            pos as u16
        } else {
            e.shape
        };
        w.dat.bs(shape);
        w.dat.rd(e.offset.x);
        w.dat.rd(e.offset.y);
        w.dat.bd(e.scale);
        w.dat.bd(e.rotation);
        w.dat.bs(e.complex);
    }
    if has_text {
        w.dat.raw_bytes(&area);
    }
}

fn text_style_body(w: &mut ObjW, t: &TextStyle) {
    w.dat.b(t.flags & 4 != 0);
    w.dat.b(t.flags & 1 != 0);
    w.dat.bd(t.height);
    w.dat.bd(t.width_factor);
    w.dat.bd(t.oblique);
    w.dat.rc(t.generation as u8);
    w.dat.bd(t.last_height);
    w.tv(&t.font_file);
    w.tv(&t.bigfont_file);
}

fn dimstyle_body(w: &mut ObjW, v: &DimVars) {
    let ver = w.ver;
    w.tv(&v.post);
    w.tv(&v.apost);
    for x in [v.scale, v.asz, v.exo, v.dli, v.exe, v.rnd, v.dle, v.tp, v.tm] {
        w.dat.bd(x);
    }
    if ver >= V::R2007 {
        w.dat.bd(v.fxl);
        w.dat.bd(v.jogang);
        w.dat.bs(v.tfill);
        w.cmc(v.tfillclr);
    }
    for b in [v.tol, v.lim, v.tih, v.toh, v.se1, v.se2] {
        w.dat.b(b);
    }
    w.dat.bs(v.tad);
    w.dat.bs(v.zin);
    w.dat.bs(v.azin);
    if ver >= V::R2007 {
        w.dat.bs(v.arcsym);
    }
    for x in [v.txt, v.cen, v.tsz, v.altf, v.lfac, v.tvp, v.tfac, v.gap, v.altrnd] {
        w.dat.bd(x);
    }
    w.dat.b(v.alt);
    w.dat.bs(v.altd);
    for b in [v.tofl, v.sah, v.tix, v.soxd] {
        w.dat.b(b);
    }
    w.cmc(v.clrd);
    w.cmc(v.clre);
    w.cmc(v.clrt);
    for x in [v.adec, v.dec, v.tdec, v.altu, v.alttd, v.aunit, v.frac, v.lunit, v.dsep, v.tmove, v.just] {
        w.dat.bs(x);
    }
    w.dat.b(v.sd1);
    w.dat.b(v.sd2);
    for x in [v.tolj, v.tzin, v.altz, v.alttz] {
        w.dat.bs(x);
    }
    w.dat.b(v.upt);
    w.dat.bs(v.fit);
    if ver >= V::R2007 {
        w.dat.b(v.fxlon);
    }
    if ver >= V::R2010 {
        w.dat.b(v.txtdirection);
        w.dat.bd(v.altmzf);
        w.tv(&v.altmzs);
        w.dat.bd(v.mzf);
        w.tv(&v.mzs);
    }
    w.dat.bs(v.lwd as u16);
    w.dat.bs(v.lwe as u16);
    w.dat.b(v.flag_bit);
}

fn vport_body(w: &mut ObjW, v: &Vport) {
    let ver = w.ver;
    w.dat.bd(v.view_height);
    w.dat.bd(v.aspect * v.view_height);
    w.dat.rd2(v2(v.center));
    w.dat.bd3(v3(v.target));
    w.dat.bd3(v3(v.direction));
    w.dat.bd(v.twist);
    w.dat.bd(v.lens);
    w.dat.bd(v.front_clip);
    w.dat.bd(v.back_clip);
    let m = v.view_mode;
    w.dat.b(m & 1 != 0);
    w.dat.b(m & 2 != 0);
    w.dat.b(m & 4 != 0);
    w.dat.b(m & 16 == 0);
    w.dat.rc(v.render_mode);
    if ver >= V::R2007 {
        w.dat.b(v.default_lights);
        w.dat.rc(v.lighting_type);
        w.dat.bd(v.brightness);
        w.dat.bd(v.contrast);
        w.cmc(v.ambient);
    }
    w.dat.rd2(v2(v.lower_left));
    w.dat.rd2(v2(v.upper_right));
    w.dat.b(m & 8 != 0);
    w.dat.bs(v.circle_zoom);
    w.dat.b(v.fast_zoom);
    w.dat.bits(v.ucsicon as u64 & 3, 2);
    w.dat.b(v.grid_on);
    w.dat.rd2(v2(v.grid_spacing));
    w.dat.b(v.snap_on);
    w.dat.b(v.snap_style);
    w.dat.bs(v.snap_isopair);
    w.dat.bd(v.snap_rotation);
    w.dat.rd2(v2(v.snap_base));
    w.dat.rd2(v2(v.snap_spacing));
    w.dat.b(v.unknown_bit);
    w.dat.b(v.ucs_per_viewport);
    w.dat.bd3(v3(v.ucs_origin));
    w.dat.bd3(v3(v.ucs_x));
    w.dat.bd3(v3(v.ucs_y));
    w.dat.bd(v.ucs_elevation);
    w.dat.bs(v.ucs_ortho_type);
    if ver >= V::R2007 {
        w.dat.bs(v.grid_flags);
        w.dat.bs(v.grid_major);
    }
}

/// The XRECORD data bytes (spec §20.4.104): each item's group code, then
/// its value in the size its code takes, strings as UTF-16 from R2007.
fn xrecord_bytes(items: &[(u16, XrValue)], ver: DwgVersion) -> Vec<u8> {
    let mut b = Vec::new();
    for (code, v) in items {
        b.extend_from_slice(&code.to_le_bytes());
        match v {
            XrValue::Str(s) => {
                if ver.has_string_stream() {
                    let units: Vec<u16> = s.encode_utf16().collect();
                    b.extend_from_slice(&(units.len() as u16).to_le_bytes());
                    for u in units {
                        b.extend_from_slice(&u.to_le_bytes());
                    }
                } else {
                    // A 16-bit length, the code page, the bytes.
                    let bytes: Vec<u8> = s.chars().map(|c| c as u32 as u8).collect();
                    let n = bytes.len().min(usize::from(u16::MAX));
                    b.extend_from_slice(&(n as u16).to_le_bytes());
                    b.push(30);
                    b.extend_from_slice(&bytes[..n]);
                }
            }
            XrValue::Point(p) => {
                for c in p.to_array() {
                    b.extend_from_slice(&c.to_le_bytes());
                }
            }
            XrValue::Real(r) => b.extend_from_slice(&r.to_le_bytes()),
            XrValue::Int8(i) => b.push(*i as u8),
            XrValue::Int16(i) => b.extend_from_slice(&i.to_le_bytes()),
            XrValue::Int32(i) => b.extend_from_slice(&i.to_le_bytes()),
            XrValue::Int64(i) => b.extend_from_slice(&i.to_le_bytes()),
            XrValue::Bool(v) => b.push(*v as u8),
            XrValue::Bytes(x) => {
                b.push(x.len().min(255) as u8);
                b.extend_from_slice(&x[..x.len().min(255)]);
            }
            XrValue::Handle(h) => b.extend_from_slice(&h.0.to_le_bytes()),
        }
    }
    b
}

/// DWG keeps the MLINESTYLE flag bits in a different order from DXF 70.
fn mline_flags_to_dwg(dxf: u16) -> u16 {
    crate::read::nongraphic::mline_flags_to_dwg(dxf)
}

/// The non-graphical objects (the mirror of `dwg::nongraphic::decode`):
/// type data, the common handles, then the type's own handles, coded as
/// AutoCAD 2027 codes them.
fn nongraphic(w: &mut ObjW, o: &Object, n: &NonGraphic) -> Result<()> {
    let ver = w.ver;
    match n {
        // Written by styles::encode, which `specific` calls instead.
        NonGraphic::Groups(_) => return Err(Error::format("style object reached the wrong encoder")),
        NonGraphic::Dictionary(d) => {
            w.dat.bl(d.entries.len() as u32);
            w.dat.bs(d.cloning);
            w.dat.rc(d.hard_owner);
            for (k, _) in &d.entries {
                w.tv(k);
            }
            common_handles(w, o);
            // Entries are soft-owner references even in a dictionary that
            // owns them hard (DXF 280 = 1, entries under 360): AutoCAD
            // 2027 codes both kinds 2.
            for (_, h) in &d.entries {
                w.h(Ref::SoftOwner, *h);
            }
            if let Some(def) = d.default {
                w.h(Ref::HardPtr, def);
            }
        }
        NonGraphic::XRecord(x) => {
            let bytes = xrecord_bytes(&x.items, ver);
            w.dat.bl(bytes.len() as u32);
            w.dat.raw_bytes(&bytes);
            w.dat.bs(x.cloning);
            common_handles(w, o);
        }
        NonGraphic::Placeholder => common_handles(w, o),
        NonGraphic::DictionaryVar { schema, value } => {
            w.dat.rc(*schema);
            w.tv(value);
            common_handles(w, o);
        }
        NonGraphic::Scale(s) => {
            w.dat.bs(s.flag);
            w.tv(&s.name);
            w.dat.bd(s.paper_units);
            w.dat.bd(s.drawing_units);
            w.dat.b(s.unit_scale);
            common_handles(w, o);
        }
        NonGraphic::Layout(l) => {
            let p = &l.plot;
            w.tv(&p.page_setup_name);
            w.tv(&p.printer);
            w.dat.bs(p.flags);
            for m in p.margins {
                w.dat.bd(m);
            }
            w.dat.bd(p.paper_width);
            w.dat.bd(p.paper_height);
            w.tv(&p.paper_size);
            w.dat.bd2(v2(p.origin));
            w.dat.bs(p.paper_units);
            w.dat.bs(p.rotation);
            w.dat.bs(p.plot_type);
            w.dat.bd2(v2(p.window_min));
            w.dat.bd2(v2(p.window_max));
            if ver <= V::R2000 {
                w.tv("");
            }
            w.dat.bd(p.real_world_units);
            w.dat.bd(p.drawing_units);
            w.tv(&p.style_sheet);
            w.dat.bs(p.scale_type);
            w.dat.bd(p.scale_factor);
            w.dat.bd2(v2(p.image_origin));
            if ver >= V::R2004 {
                w.dat.bs(p.shade_plot_mode);
                w.dat.bs(p.shade_plot_res_level);
                w.dat.bs(p.shade_plot_custom_dpi);
            }
            w.tv(&l.name);
            w.dat.bl(l.tab_order);
            w.dat.bs(l.flags);
            w.dat.bd3(v3(l.ucs_origin));
            w.dat.rd2(v2(l.limmin));
            w.dat.rd2(v2(l.limmax));
            w.dat.bd3(v3(l.insbase));
            w.dat.bd3(v3(l.ucs_x));
            w.dat.bd3(v3(l.ucs_y));
            w.dat.bd(l.elevation);
            w.dat.bs(l.ortho_type);
            w.dat.bd3(v3(l.extmin));
            w.dat.bd3(v3(l.extmax));
            if ver >= V::R2004 {
                w.dat.bl(l.viewports.len() as u32);
            }
            common_handles(w, o);
            if ver >= V::R2004 {
                w.h(Ref::HardPtr, p.plot_view);
            }
            if ver >= V::R2007 {
                w.h(Ref::SoftPtr, p.visual_style);
            }
            w.h(Ref::SoftPtr, l.block);
            w.h(Ref::SoftPtr, l.last_viewport);
            w.h(Ref::HardPtr, l.base_ucs);
            w.h(Ref::HardPtr, l.named_ucs);
            if ver >= V::R2004 {
                for h in &l.viewports {
                    w.h(Ref::SoftPtr, *h);
                }
            }
        }
        NonGraphic::MLineStyle(m) => {
            w.tv(&m.name);
            w.tv(&m.description);
            w.dat.bs(mline_flags_to_dwg(m.flags));
            w.cmc(m.fill_color);
            w.dat.bd(m.start_angle);
            w.dat.bd(m.end_angle);
            w.dat.rc(m.elements.len().min(255) as u8);
            for e in m.elements.iter().take(255) {
                w.dat.bd(e.offset);
                w.cmc(e.color);
                if ver < V::R2018 {
                    w.dat.bs(e.linetype_index as u16);
                }
            }
            common_handles(w, o);
            if ver >= V::R2018 {
                for e in m.elements.iter().take(255) {
                    w.h(Ref::HardPtr, e.linetype);
                }
            }
        }
    }
    Ok(())
}
