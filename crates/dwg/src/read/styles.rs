//! The style objects the model keeps as their DXF groups
//! ([`Groups`]): visual styles, materials, table styles, multileader,
//! section and detail view styles, and dimension associations, to and from
//! their DWG records (R2013 and later).
//!
//! The spec documents MLEADERSTYLE and TABLESTYLE; for the others the
//! layouts come from AutoCAD's own files, each record of the MI-201 corpus
//! read field by field against the DXF AutoCAD wrote of the same drawing
//! (black-box, as `PROVENANCE.md` records). Most follow their DXF groups in
//! order; where AutoCAD stores a field elsewhere in the record, or keeps a
//! DXF group out of the DWG (the property counts and group numbers DXF
//! adds), the layout says so. A record that does not fit its layout is
//! reported, and the object stays as its original bytes or is left out.

use super::streams::Streams;
use super::version::DwgVersion as V;
use crate::bits::BitReader;
use crate::read::error::{Error, Result};
use crate::read::geom::Vec3;
use crate::read::model::{Drawing, Groups, Handle, ObjectKind, XrValue};
use crate::write::objects::{ObjW, Ref};

/// How a value is stored in the DWG record.
#[derive(Clone, Copy, Debug, PartialEq)]
enum D {
    B,
    Bs,
    Bl,
    Bd,
    Rc,
    Tv,
    /// A handle with the given reference code.
    H(u8),
    /// CmColor: DXF 62 (the colour index), 420 (true colour) when set.
    Cmc,
    /// CmColor whose DXF group holds the method word itself.
    Word,
    /// 2RD: a 2D point, one DXF point group.
    Rd2,
}

/// One DXF group of a record, in DXF order.
#[derive(Clone, Debug)]
enum G {
    /// Subclass marker (DXF only).
    M(&'static str),
    /// A value stored in both forms.
    V(u16, D),
    /// A group only DXF has, with the value AutoCAD writes there.
    X(u16, i64),
    /// A handle DXF leaves out when it is null.
    OptH(u16),
    /// A whole number or flag at the end of a record that DXF in the
    /// reference's older layout leaves out, and the value it has then.
    Tail(u16, D, i64),
    /// A count and that many values: DXF count group, item group.
    Rep(u16, D, u16, D),
}

/// A step of the DWG record, in DWG order.
#[derive(Clone, Copy, Debug)]
enum W {
    /// The value of DXF group number `n` of the layout.
    I(usize),
    /// A value only DWG has.
    K(D, i64),
}

struct Layout {
    dxf: Vec<G>,
    dwg: Vec<W>,
}

/// The layout of a type, or `None` for types handled by code below.
fn layout(kind: &str) -> Option<Layout> {
    use D::*;
    use G::*;
    Some(match kind {
        // The objects raster images use (§20.4.81, §20.4.82, §20.4.91,
        // checked against the specification text). They keep the DXF
        // groups' order, except that IMAGEDEF's record puts the path after
        // the image size and the pixel size last.
        "IMAGEDEF" => Layout {
            dxf: vec![M("AcDbRasterImageDef"), V(90, Bl), V(1, Tv), V(10, Rd2), V(11, Rd2), V(280, B), V(281, Rc)],
            dwg: vec![W::I(1), W::I(3), W::I(2), W::I(5), W::I(6), W::I(4)],
        },
        "IMAGEDEF_REACTOR" => Layout { dxf: vec![M("AcDbRasterImageDefReactor"), V(90, Bl)], dwg: vec![W::I(1)] },
        "RASTERVARIABLES" => {
            Layout { dxf: vec![M("AcDbRasterVariables"), V(90, Bl), V(70, Bs), V(71, Bs), V(72, Bs)], dwg: vec![W::I(1), W::I(2), W::I(3), W::I(4)] }
        }
        "VISUALSTYLE" => {
            let mut dxf = vec![M("AcDbVisualStyle"), V(2, Tv), V(70, Bs), X(177, 3), V(291, B), X(70, 58)];
            for code in VISUALSTYLE_PROPERTIES {
                let d = match code {
                    90 => Bl,
                    40 => Bd,
                    62 => Cmc,
                    290 => B,
                    _ => Tv,
                };
                dxf.push(V(code, d));
                dxf.push(V(176, Bs));
            }
            // DXF 177 is 3 where the DWG record holds 2.
            let mut dwg = vec![W::I(1), W::I(2), W::K(Bs, 2), W::I(4)];
            dwg.extend((6..dxf.len()).map(W::I));
            Layout { dxf, dwg }
        }
        "MLEADERSTYLE" => {
            let dxf = vec![
                M("AcDbMLeaderStyle"),
                V(179, Bs),
                V(170, Bs),
                V(171, Bs),
                V(172, Bs),
                V(90, Bl),
                V(40, Bd),
                V(41, Bd),
                V(173, Bs),
                V(91, Word),
                OptH(340),
                V(92, Bl),
                V(290, B),
                V(42, Bd),
                V(291, B),
                V(43, Bd),
                V(3, Tv),
                OptH(341),
                V(44, Bd),
                V(300, Tv),
                OptH(342),
                V(174, Bs),
                V(178, Bs),
                V(175, Bs),
                V(176, Bs),
                V(93, Word),
                V(45, Bd),
                V(292, B),
                V(297, B),
                V(46, Bd),
                OptH(343),
                V(94, Word),
                V(47, Bd),
                V(49, Bd),
                V(140, Bd),
                V(293, B),
                V(141, Bd),
                V(294, B),
                V(177, Bs),
                V(142, Bd),
                V(295, B),
                V(296, B),
                V(143, Bd),
                V(271, Bs),
                V(272, Bs),
                V(273, Bs),
                Tail(298, B, 0),
            ];
            // The spec's order: 273 before 272.
            let mut dwg: Vec<W> = (1..dxf.len()).map(W::I).collect();
            let (a, b) = (dwg.len() - 3, dwg.len() - 2);
            dwg.swap(a, b);
            Layout { dxf, dwg }
        }
        "ACDBSECTIONVIEWSTYLE" => {
            let mut dxf = model_doc_view();
            let base = dxf.len();
            dxf.extend([
                M("AcDbSectionViewStyle"),
                V(70, Bs),           // 1
                X(71, 0),            // 2
                V(90, Bl),           // 3
                X(71, 1),            // 4
                V(340, H(5)),        // 5
                V(62, Cmc),          // 6
                V(40, Bd),           // 7
                V(340, H(5)),        // 8
                V(340, H(5)),        // 9
                V(62, Cmc),          // 10
                V(40, Bd),           // 11
                V(300, Tv),          // 12
                V(40, Bd),           // 13
                V(90, Bl),           // 14
                V(40, Bd),           // 15
                V(90, Bl),           // 16
                X(71, 2),            // 17
                V(340, H(5)),        // 18
                V(90, Bl),           // 19
                V(62, Cmc),          // 20
                V(340, H(5)),        // 21
                V(90, Bl),           // 22
                V(62, Cmc),          // 23
                V(40, Bd),           // 24
                V(40, Bd),           // 25
                V(40, Bd),           // 26
                X(71, 3),            // 27
                V(340, H(5)),        // 28
                V(62, Cmc),          // 29
                V(40, Bd),           // 30
                V(90, Bl),           // 31
                V(40, Bd),           // 32
                V(90, Bl),           // 33
                V(300, Tv),          // 34
                X(71, 4),            // 35
                V(62, Cmc),          // 36
                V(62, Cmc),          // 37
                V(300, Tv),          // 38
                V(40, Bd),           // 39
                V(90, Bl),           // 40
                V(290, B),           // 41
                V(290, B),           // 42
                Rep(90, Bl, 40, Bd), // 43
            ]);
            // AutoCAD stores the fields DXF lists at 14–16 and 25 near the
            // end, before the angle list.
            let order = [
                1, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 18, 19, 20, 21, 22, 23, 24, 26, 28, 29, 30, 31, 32, 33, 34, 36, 37, 38, 39, 40, 41, 42, 14, 15,
                16, 25, 43,
            ];
            let mut dwg: Vec<W> = (1..base).map(W::I).collect();
            dwg.extend(order.iter().map(|i| W::I(base + i)));
            Layout { dxf, dwg }
        }
        "ACDBDETAILVIEWSTYLE" => {
            let mut dxf = model_doc_view();
            let base = dxf.len();
            dxf.extend([
                M("AcDbDetailViewStyle"),
                V(70, Bs),    // 1
                X(71, 0),     // 2
                V(90, Bl),    // 3
                X(71, 1),     // 4
                V(340, H(5)), // 5
                V(62, Cmc),   // 6
                V(40, Bd),    // 7
                V(340, H(5)), // 8
                V(62, Cmc),   // 9
                V(40, Bd),    // 10
                V(300, Tv),   // 11
                V(40, Bd),    // 12
                V(280, Rc),   // 13
                X(71, 2),     // 14
                V(340, H(5)), // 15
                V(90, Bl),    // 16
                V(62, Cmc),   // 17
                X(71, 3),     // 18
                V(340, H(5)), // 19
                V(62, Cmc),   // 20
                V(40, Bd),    // 21
                V(90, Bl),    // 22
                V(40, Bd),    // 23
                V(90, Bl),    // 24
                V(300, Tv),   // 25
                X(71, 4),     // 26
                V(340, H(5)), // 27
                V(90, Bl),    // 28
                V(62, Cmc),   // 29
                V(340, H(5)), // 30
                V(90, Bl),    // 31
                V(62, Cmc),   // 32
                V(280, Rc),   // 33
            ]);
            let order = [1, 3, 5, 6, 7, 8, 12, 13, 9, 10, 11, 15, 16, 17, 19, 20, 21, 22, 23, 24, 25, 27, 28, 29, 30, 31, 32, 33];
            let mut dwg: Vec<W> = (1..base).map(W::I).collect();
            dwg.extend(order.iter().map(|i| W::I(base + i)));
            Layout { dxf, dwg }
        }
        _ => return None,
    })
}

/// The part section and detail view styles share.
fn model_doc_view() -> Vec<G> {
    use D::*;
    use G::*;
    vec![M("AcDbModelDocViewStyle"), V(70, Bs), V(3, Tv), V(290, B), V(300, Tv), V(90, Bl)]
}

/// The value kind of each of a visual style's 58 properties, by DXF code
/// (the same in all 24 of AutoCAD's default styles); each is followed by
/// its flag (176).
const VISUALSTYLE_PROPERTIES: [u16; 58] = [
    90, 90, 90, 90, 40, 40, 62, 90, 90, 62, 62, 90, 90, 40, 90, 62, 40, 90, 90, 90, 62, 90, 90, 90, 290, 90, 40, 90, 290, 290, 290, 290, 290, 290,
    290, 290, 290, 90, 40, 40, 90, 62, 90, 90, 62, 290, 90, 90, 90, 290, 90, 62, 40, 90, 1, 290, 40, 40,
];

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// A value read from or for one field.
#[derive(Clone, Debug, PartialEq)]
enum Val {
    Int(i64),
    Real(f64),
    Str(String),
    Handle(Handle),
    Point(Vec3),
    /// A CmColor method word (and, for `D::Cmc`, the colour index DXF
    /// gives with it).
    Color(u32),
    List(Vec<Val>),
    Null,
}

fn xr(code: u16, v: &Val) -> XrValue {
    use super::nongraphic::{XrType, xr_type};
    let i = match v {
        Val::Int(i) => *i,
        Val::Real(f) => *f as i64,
        _ => 0,
    };
    let f = match v {
        Val::Real(f) => *f,
        Val::Int(i) => *i as f64,
        _ => 0.0,
    };
    match xr_type(code) {
        Some(XrType::Str) => XrValue::Str(match v {
            Val::Str(s) => s.clone(),
            _ => String::new(),
        }),
        Some(XrType::Point) => XrValue::Point(match v {
            Val::Point(p) => *p,
            _ => Vec3::ZERO,
        }),
        Some(XrType::Real) => XrValue::Real(f),
        Some(XrType::Int8) => XrValue::Int8(i as i8),
        Some(XrType::Int16) => XrValue::Int16(i as i16),
        Some(XrType::Int32) => XrValue::Int32(i as i32),
        Some(XrType::Int64) => XrValue::Int64(i),
        Some(XrType::Bool) => XrValue::Bool(i != 0),
        Some(XrType::Handle) => XrValue::Handle(match v {
            Val::Handle(h) => *h,
            _ => Handle::NULL,
        }),
        Some(XrType::Bytes) | None => XrValue::Int32(i as i32),
    }
}

fn from_xr(v: &XrValue) -> Val {
    match v {
        XrValue::Str(s) => Val::Str(s.clone()),
        XrValue::Point(p) => Val::Point(*p),
        XrValue::Real(f) => Val::Real(*f),
        XrValue::Int8(i) => Val::Int(*i as i64),
        XrValue::Int16(i) => Val::Int(*i as i64),
        XrValue::Int32(i) => Val::Int(*i as i64),
        XrValue::Int64(i) => Val::Int(*i),
        XrValue::Bool(b) => Val::Int(*b as i64),
        XrValue::Handle(h) => Val::Handle(*h),
        XrValue::Bytes(_) => Val::Null,
    }
}

/// DXF 62 (and 420) for a method word, as AutoCAD writes them: the nearest
/// index beside a true colour.
fn color_groups(code: u16, word: u32, out: &mut Vec<(u16, XrValue)>) {
    let index: i16 = match word >> 24 {
        0xC0 => 256,
        0xC1 => 0,
        0xC2 => crate::read::model::color::nearest_aci((word >> 16) as u8, (word >> 8) as u8, word as u8) as i16,
        0xC3 => (word & 0xFF) as i16,
        0xC8 => 257,
        _ => 7,
    };
    out.push((code, XrValue::Int16(index)));
    if word >> 24 == 0xC2 {
        out.push((420, XrValue::Int32((word & 0xFF_FFFF) as i32)));
    }
}

fn word_of(index: i64, rgb: Option<i64>) -> u32 {
    match rgb {
        Some(rgb) => 0xC200_0000 | (rgb as u32 & 0xFF_FFFF),
        None => match index {
            256 => 0xC000_0000,
            0 => 0xC100_0000,
            257 => 0xC800_0000,
            i => 0xC300_0000 | (i as u32 & 0xFF),
        },
    }
}

// ---------------------------------------------------------------------------
// Reading DWG
// ---------------------------------------------------------------------------

fn read(s: &mut Streams, own: u64, d: D) -> Result<Val> {
    Ok(match d {
        D::B => Val::Int(s.dat.b()? as i64),
        D::Bs => Val::Int(s.dat.bs()? as i16 as i64),
        D::Bl => Val::Int(s.dat.bl()? as i32 as i64),
        D::Bd => Val::Real(s.dat.bd()?),
        D::Rc => Val::Int(s.dat.rc()? as i64),
        D::Tv => Val::Str(s.tv()?),
        D::Rd2 => {
            let [x, y] = s.dat.rd2()?;
            Val::Point(Vec3::new(x, y, 0.0))
        }
        D::H(_) => Val::Handle(Handle(s.h_abs(own)?)),
        D::Cmc | D::Word => {
            let _index = s.dat.bs()?;
            let word = s.dat.bl()?;
            let names = s.dat.rc()?;
            if names != 0 {
                return Err(Error::format("named colour in a style object"));
            }
            Val::Color(word)
        }
    })
}

fn decode_layout(s: &mut Streams, own: u64, l: &Layout) -> Result<Vec<(u16, XrValue)>> {
    let mut vals: Vec<Val> = vec![Val::Null; l.dxf.len()];
    for w in &l.dwg {
        match *w {
            W::K(d, want) => {
                let got = read(s, own, d)?;
                if got != Val::Int(want) {
                    return Err(Error::format(format!("expected {want}, found {got:?}")));
                }
            }
            W::I(i) => {
                vals[i] = match &l.dxf[i] {
                    G::V(_, d) | G::Tail(_, d, _) => read(s, own, *d)?,
                    G::OptH(_) => read(s, own, D::H(5))?,
                    G::Rep(_, cd, _, id) => {
                        let n = match read(s, own, *cd)? {
                            Val::Int(n) if (0..=10_000).contains(&n) => n as usize,
                            other => return Err(Error::format(format!("count {other:?}"))),
                        };
                        let mut list = Vec::with_capacity(n);
                        for _ in 0..n {
                            list.push(read(s, own, *id)?);
                        }
                        Val::List(list)
                    }
                    _ => Val::Null,
                };
            }
        }
    }
    let mut out = Vec::new();
    for (g, v) in l.dxf.iter().zip(&vals) {
        match g {
            G::M(name) => out.push((100, XrValue::Str(name.to_string()))),
            G::X(code, value) => out.push((*code, xr(*code, &Val::Int(*value)))),
            G::V(code, D::Cmc) => match v {
                Val::Color(w) => color_groups(*code, *w, &mut out),
                _ => return Err(Error::format("colour expected")),
            },
            G::V(code, D::Word) => match v {
                Val::Color(w) => out.push((*code, XrValue::Int32(*w as i32))),
                _ => return Err(Error::format("colour expected")),
            },
            G::V(code, _) | G::Tail(code, _, _) => out.push((*code, xr(*code, v))),
            G::OptH(code) => {
                if let Val::Handle(h) = v
                    && !h.is_null()
                {
                    out.push((*code, XrValue::Handle(*h)));
                }
            }
            G::Rep(cc, _, ic, _) => {
                let list = match v {
                    Val::List(l) => l.as_slice(),
                    _ => &[],
                };
                out.push((*cc, xr(*cc, &Val::Int(list.len() as i64))));
                for x in list {
                    out.push((*ic, xr(*ic, x)));
                }
            }
        }
    }
    Ok(out)
}

/// The groups of a style object from its DWG record, the common part and
/// common handles already read. `None` for types this module does not
/// know; an error for a record that does not fit its layout.
pub(crate) fn decode(kind: &str, s: &mut Streams, own: u64, d: &Lookup) -> Option<Result<Groups>> {
    if !Groups::KINDS.contains(&kind) || s.ver < since(kind) {
        return None;
    }
    let items = match kind {
        "DIMASSOC" => decode_dimassoc(s, own),
        "MATERIAL" => decode_material(s),
        "TABLESTYLE" => decode_tablestyle(s, own, d),
        "CELLSTYLEMAP" => return None,
        k => decode_layout(s, own, &layout(k)?),
    };
    Some(items.map(|items| Groups { kind: kind.to_string(), items }))
}

// ---------------------------------------------------------------------------
// Writing DWG
// ---------------------------------------------------------------------------

/// The groups of a record, walked in DXF order.
struct Cur<'a> {
    items: &'a [(u16, XrValue)],
    at: usize,
    kind: &'a str,
}

impl<'a> Cur<'a> {
    fn new(g: &'a Groups) -> Self {
        Cur { items: &g.items, at: 0, kind: &g.kind }
    }
    fn peek(&self) -> Option<u16> {
        self.items.get(self.at).map(|x| x.0)
    }
    /// Groups not read yet.
    fn left(&self) -> usize {
        self.items.len().saturating_sub(self.at)
    }
    fn take(&mut self, code: u16) -> Result<&'a XrValue> {
        match self.items.get(self.at) {
            Some((c, v)) if *c == code => {
                self.at += 1;
                Ok(v)
            }
            other => Err(Error::format(format!("{}: group {code} expected at item {}, found {:?}", self.kind, self.at, other.map(|x| x.0)))),
        }
    }
    fn int(&mut self, code: u16) -> Result<i64> {
        match from_xr(self.take(code)?) {
            Val::Int(i) => Ok(i),
            Val::Real(f) => Ok(f as i64),
            v => Err(Error::format(format!("{}: group {code}: number expected, {v:?}", self.kind))),
        }
    }
    fn point(&mut self, code: u16) -> Result<Vec3> {
        match self.take(code)? {
            XrValue::Point(p) => Ok(*p),
            other => Err(Error::format(format!("{}: group {code} is {other:?}, a point expected", self.kind))),
        }
    }

    fn real(&mut self, code: u16) -> Result<f64> {
        match from_xr(self.take(code)?) {
            Val::Real(f) => Ok(f),
            Val::Int(i) => Ok(i as f64),
            v => Err(Error::format(format!("{}: group {code}: number expected, {v:?}", self.kind))),
        }
    }
    fn str(&mut self, code: u16) -> Result<String> {
        match self.take(code)? {
            XrValue::Str(s) => Ok(s.clone()),
            v => Err(Error::format(format!("{}: group {code}: text expected, {v:?}", self.kind))),
        }
    }
    fn handle(&mut self, code: u16) -> Result<Handle> {
        match self.take(code)? {
            XrValue::Handle(h) => Ok(*h),
            v => Err(Error::format(format!("{}: group {code}: handle expected, {v:?}", self.kind))),
        }
    }
    fn color(&mut self, code: u16) -> Result<u32> {
        let index = self.int(code)?;
        let rgb = if self.peek() == Some(420) { Some(self.int(420)?) } else { None };
        if self.peek() == Some(430) {
            return Err(Error::format(format!("{}: named colour", self.kind)));
        }
        Ok(word_of(index, rgb))
    }
    fn done(&self) -> Result<()> {
        match self.items.get(self.at) {
            None => Ok(()),
            Some((c, _)) => Err(Error::format(format!("{}: unexpected group {c} at item {}", self.kind, self.at))),
        }
    }
}

fn write(w: &mut ObjW, d: D, v: &Val) -> Result<()> {
    let int = || match v {
        Val::Int(i) => Ok(*i),
        _ => Err(Error::format(format!("number expected, {v:?}"))),
    };
    match d {
        D::B => w.dat.b(int()? != 0),
        D::Bs => w.dat.bs(int()? as i16 as u16),
        D::Bl => w.dat.bl(int()? as i32 as u32),
        D::Bd => w.dat.bd(match v {
            Val::Real(f) => *f,
            Val::Int(i) => *i as f64,
            _ => return Err(Error::format("number expected")),
        }),
        D::Rc => w.dat.rc(int()? as u8),
        D::Rd2 => match v {
            Val::Point(p) => w.dat.rd2([p.x, p.y]),
            _ => return Err(Error::format("point expected")),
        },
        D::Tv => w.tv(match v {
            Val::Str(s) => s,
            _ => "",
        }),
        D::H(code) => {
            let h = match v {
                Val::Handle(h) => *h,
                _ => Handle::NULL,
            };
            w.hdl.handle(code, h.0);
        }
        D::Cmc | D::Word => {
            let word = match v {
                Val::Color(c) => *c,
                Val::Int(i) => *i as u32,
                _ => return Err(Error::format("colour expected")),
            };
            w.dat.bs(0);
            w.dat.bl(word);
            w.dat.rc(0);
        }
    }
    Ok(())
}

fn encode_layout(w: &mut ObjW, g: &Groups, l: &Layout) -> Result<()> {
    let mut c = Cur::new(g);
    let mut vals: Vec<Val> = vec![Val::Null; l.dxf.len()];
    for (i, f) in l.dxf.iter().enumerate() {
        vals[i] = match f {
            G::M(name) => {
                if c.str(100)? != *name {
                    return Err(Error::format(format!("{}: subclass {name} expected", g.kind)));
                }
                Val::Null
            }
            G::X(code, _) => {
                c.take(*code)?;
                Val::Null
            }
            G::V(code, D::Cmc) => Val::Color(c.color(*code)?),
            G::V(code, D::Word) => Val::Color(c.int(*code)? as u32),
            G::V(code, D::Tv) => Val::Str(c.str(*code)?),
            G::V(code, D::H(_)) => Val::Handle(c.handle(*code)?),
            G::V(code, D::Bd) => Val::Real(c.real(*code)?),
            G::V(code, D::Rd2) => Val::Point(c.point(*code)?),
            G::V(code, _) => Val::Int(c.int(*code)?),
            G::Tail(code, _, missing) => Val::Int(if c.peek() == Some(*code) { c.int(*code)? } else { *missing }),
            G::OptH(code) => {
                if c.peek() == Some(*code) {
                    Val::Handle(c.handle(*code)?)
                } else {
                    Val::Handle(Handle::NULL)
                }
            }
            G::Rep(cc, _, ic, id) => {
                // Each item is one group: a count past the groups left is
                // damage, and must not size the allocation.
                let n = (c.int(*cc)?.max(0) as usize).min(c.left());
                let mut list = Vec::with_capacity(n);
                for _ in 0..n {
                    list.push(match id {
                        D::Bd => Val::Real(c.real(*ic)?),
                        D::Tv => Val::Str(c.str(*ic)?),
                        _ => Val::Int(c.int(*ic)?),
                    });
                }
                Val::List(list)
            }
        };
    }
    c.done()?;
    for step in &l.dwg {
        match *step {
            W::K(d, v) => write(w, d, &Val::Int(v))?,
            W::I(i) => match (&l.dxf[i], &vals[i]) {
                (G::V(_, d) | G::Tail(_, d, _), v) => write(w, *d, v)?,
                (G::OptH(_), v) => write(w, D::H(5), v)?,
                (G::Rep(_, cd, _, id), Val::List(list)) => {
                    write(w, *cd, &Val::Int(list.len() as i64))?;
                    for x in list {
                        write(w, *id, x)?;
                    }
                }
                _ => {}
            },
        }
    }
    Ok(())
}

/// Write a style object's type data and handles (after the common part
/// and common handles).
pub(crate) fn encode(w: &mut ObjW, g: &Groups, d: &Drawing) -> Result<()> {
    if w.ver < since(&g.kind) {
        return Err(Error::unsupported(format!("{} before {:?}", g.kind, since(&g.kind))));
    }
    match g.kind.as_str() {
        "DIMASSOC" => encode_dimassoc(w, g),
        "MATERIAL" => encode_material(w, g),
        "TABLESTYLE" => encode_tablestyle(w, g, &Lookup::new(d)),
        k => match layout(k) {
            Some(l) => encode_layout(w, g, &l),
            None => Err(Error::unsupported(format!("{k} is kept for DXF only"))),
        },
    }
}

/// The first version whose records of `kind` this module reads and writes:
/// the raster objects' layouts are the specification's, for every
/// version; the others come from AutoCAD 2027's R2013 and later files.
fn since(kind: &str) -> V {
    match kind {
        "IMAGEDEF" | "IMAGEDEF_REACTOR" | "RASTERVARIABLES" => V::R13,
        _ => V::R2013,
    }
}

/// Names and handles the codecs look up in the drawing: text styles by
/// name for the table style (DXF names its text style, DWG points at it).
pub(crate) struct Lookup {
    text_styles: Vec<(String, Handle)>,
}

impl Lookup {
    #[cfg(test)]
    pub(crate) fn empty() -> Lookup {
        Lookup { text_styles: Vec::new() }
    }

    /// The text styles by name and handle, as a reader that has its own tables knows them.
    pub(crate) fn from_text_styles(text_styles: Vec<(String, Handle)>) -> Lookup {
        Lookup { text_styles }
    }

    pub fn new(d: &Drawing) -> Lookup {
        let text_styles = d
            .objects
            .values()
            .filter_map(|o| match &o.kind {
                ObjectKind::TextStyle(t) => Some((t.name.clone(), o.handle)),
                ObjectKind::NamedRecord(r) if o.type_code == 0x35 => Some((r.name.clone(), o.handle)),
                _ => None,
            })
            .collect();
        Lookup { text_styles }
    }
    fn style_handle(&self, name: &str) -> Handle {
        self.text_styles.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, h)| *h).unwrap_or(Handle::NULL)
    }
    fn style_name(&self, h: Handle) -> String {
        self.text_styles.iter().find(|(_, x)| *x == h).map(|(n, _)| n.clone()).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// DIMASSOC
// ---------------------------------------------------------------------------

/// DIMASSOC: the dimension (330), the association flags (90: one bit per
/// point reference present, up to four), the transformation-space flag
/// (70) and rotated-dimension type (71), then each point reference: its
/// class (1), osnap type (72), main objects (331), subentity type (73),
/// graphics marker (91), geometry parameter (40), point (10) and whether
/// a last point reference follows (75). References with intersection
/// objects or xref paths are not handled.
fn decode_dimassoc(s: &mut Streams, own: u64) -> Result<Vec<(u16, XrValue)>> {
    let mut out = vec![(100, XrValue::Str("AcDbDimAssoc".into()))];
    let flags = s.dat.bl()?;
    let trans = s.dat.b()?;
    let rotated = s.dat.rc()?;
    let dim = s.h_abs(own)?;
    out.push((330, XrValue::Handle(Handle(dim))));
    out.push((90, XrValue::Int32(flags as i32)));
    out.push((70, XrValue::Int16(trans as i16)));
    out.push((71, XrValue::Int16(rotated as i16)));
    for bit in 0..4 {
        if flags & (1 << bit) == 0 {
            continue;
        }
        let class = s.tv()?;
        let osnap = s.dat.rc()?;
        let n = s.dat.bl()? as usize;
        if n > 64 {
            return Err(Error::format("DIMASSOC: object count"));
        }
        let mut mains = Vec::with_capacity(n);
        for _ in 0..n {
            mains.push(s.h_abs(own)?);
        }
        let subent = s.dat.bs()?;
        let marker = s.dat.bl()?;
        let extra = s.dat.bl()?;
        if extra != 0 {
            return Err(Error::format("DIMASSOC: intersection or xref data"));
        }
        let param = s.dat.bd()?;
        let p = s.dat.bd3()?;
        let last = s.dat.b()?;
        out.push((1, XrValue::Str(class)));
        out.push((72, XrValue::Int16(osnap as i16)));
        for m in mains {
            out.push((331, XrValue::Handle(Handle(m))));
        }
        out.push((73, XrValue::Int16(subent as i16)));
        out.push((91, XrValue::Int32(marker as i32)));
        out.push((40, XrValue::Real(param)));
        out.push((10, XrValue::Point(Vec3::new(p[0], p[1], p[2]))));
        out.push((75, XrValue::Int16(last as i16)));
    }
    Ok(out)
}

fn encode_dimassoc(w: &mut ObjW, g: &Groups) -> Result<()> {
    let mut c = Cur::new(g);
    if c.str(100)? != "AcDbDimAssoc" {
        return Err(Error::format("DIMASSOC: subclass"));
    }
    let dim = c.handle(330)?;
    let flags = c.int(90)?;
    w.dat.bl(flags as u32);
    w.dat.b(c.int(70)? != 0);
    w.dat.rc(c.int(71)? as u8);
    w.h(Ref::SoftPtr, dim);
    for bit in 0..4 {
        if flags & (1 << bit) == 0 {
            continue;
        }
        w.tv(&c.str(1)?);
        w.dat.rc(c.int(72)? as u8);
        let mut mains = Vec::new();
        while c.peek() == Some(331) {
            mains.push(c.handle(331)?);
        }
        w.dat.bl(mains.len() as u32);
        for m in mains {
            w.h(Ref::SoftPtr, m);
        }
        w.dat.bs(c.int(73)? as u16);
        w.dat.bl(c.int(91)? as u32);
        // Writers that follow the DXF reference give a 301 here, empty when
        // the reference has no intersection or xref data (AutoCAD leaves it
        // out then).
        if c.peek() == Some(301) && !c.str(301)?.is_empty() {
            return Err(Error::unsupported("DIMASSOC: intersection or xref data"));
        }
        w.dat.bl(0);
        w.dat.bd(c.real(40)?);
        let p = match from_xr(c.take(10)?) {
            Val::Point(p) => p,
            _ => return Err(Error::format("DIMASSOC: point expected")),
        };
        w.dat.bd3([p.x, p.y, p.z]);
        w.dat.b(c.int(75)? != 0);
    }
    c.done()
}

// ---------------------------------------------------------------------------
// MATERIAL
// ---------------------------------------------------------------------------

/// A material's record between its name and its six map transforms, and
/// after the last, as AutoCAD writes its default materials (ByLayer,
/// ByBlock, Global). The DXF of those materials gives only the name, the
/// map sources (all 0) and the six matrices; the rest of each record is
/// AutoCAD's default values, which these runs hold. A material with any
/// other property set is not handled.
const MATERIAL_RUNS: [&str; 7] = [
    "0000000001000000000101000000100000000100000001",
    "00000000000000000101000000100000000100000001",
    "0000000000000000000000000000000000000000000000000000000000111000000011111101000000100000000100000001",
    "000000000101000000100000000100000001",
    "0000000001000000100000000100000001",
    "000000000101000000100000000100000001",
    "0000000010101010010111111110",
];
/// The matrix codes, and the DXF-only source group (0) before each but the
/// last.
const MATERIAL_MAPS: [(u16, Option<u16>); 6] = [(43, Some(72)), (47, Some(77)), (49, Some(171)), (142, Some(175)), (144, Some(179)), (147, None)];

fn read_run(r: &mut BitReader, run: &str) -> Result<()> {
    for ch in run.bytes() {
        if r.b()? != (ch == b'1') {
            return Err(Error::format("MATERIAL: not one of AutoCAD's default materials"));
        }
    }
    Ok(())
}

fn decode_material(s: &mut Streams) -> Result<Vec<(u16, XrValue)>> {
    let name = s.tv()?;
    let description = s.tv()?;
    let mut out = vec![(100, XrValue::Str("AcDbMaterial".into())), (1, XrValue::Str(name))];
    // DXF gives the description only when there is one.
    if !description.is_empty() {
        out.push((2, XrValue::Str(description)));
    }
    for (i, (code, source)) in MATERIAL_MAPS.iter().enumerate() {
        read_run(&mut s.dat, MATERIAL_RUNS[i])?;
        if let Some(src) = source {
            out.push((*src, XrValue::Int16(0)));
        }
        for _ in 0..16 {
            out.push((*code, XrValue::Real(s.dat.bd()?)));
        }
    }
    read_run(&mut s.dat, MATERIAL_RUNS[6])?;
    Ok(out)
}

fn encode_material(w: &mut ObjW, g: &Groups) -> Result<()> {
    let mut c = Cur::new(g);
    if c.str(100)? != "AcDbMaterial" {
        return Err(Error::format("MATERIAL: subclass"));
    }
    w.tv(&c.str(1)?);
    w.tv(&if c.peek() == Some(2) { c.str(2)? } else { String::new() });
    for (i, (code, source)) in MATERIAL_MAPS.iter().enumerate() {
        for ch in MATERIAL_RUNS[i].bytes() {
            w.dat.b(ch == b'1');
        }
        if let Some(src) = source
            && c.int(*src)? != 0
        {
            return Err(Error::format("MATERIAL: map source other than the default"));
        }
        for _ in 0..16 {
            w.dat.bd(c.real(*code)?);
        }
    }
    for ch in MATERIAL_RUNS[6].bytes() {
        w.dat.b(ch == b'1');
    }
    c.done()
}

// ---------------------------------------------------------------------------
// TABLESTYLE
// ---------------------------------------------------------------------------

/// One row style of a table style as DXF gives it (the pre-2008 layout
/// AutoCAD still writes to DXF): text style, height, alignment, text and
/// fill colours, data type and unit, format.
#[derive(Debug, Clone, PartialEq)]
struct RowStyle {
    text_style: Handle,
    height: f64,
    alignment: i64,
    text_color: i64,
    fill_color: i64,
    fill_on: bool,
    data_type: i64,
    unit_type: i64,
    format: String,
}

/// The borders DXF gives each row (lineweight, visibility, colour, six
/// times), as every row of AutoCAD's default table style has them; the
/// DWG record then stores no border.
const DEFAULT_BORDER: (i64, i64, i64) = (-2, 1, 0);

/// The table style in DWG (spec §20.4.101.2, R24): a header, the "Table"
/// cell style, then the title, header and data cell styles, each a cell
/// style (§20.4.101.4) with no margin overrides and no borders. AutoCAD's
/// DXF gives the pre-2008 layout (rows in the order data, title, header);
/// both carry the same values for a style with default borders and
/// margins, which is what this handles.
fn decode_tablestyle(s: &mut Streams, own: u64, d: &Lookup) -> Result<Vec<(u16, XrValue)>> {
    let version = s.dat.rc()?;
    let description = s.tv()?;
    let flow = s.dat.bl()?;
    let next_id = s.dat.bl()?;
    if next_id != 101 {
        return Err(Error::format("TABLESTYLE: custom cell styles"));
    }
    let _owner = s.h_abs(own)?;
    let table = read_cell_style(s, own)?;
    let (id, class, name) = (s.dat.bl()?, s.dat.bl()?, s.tv()?);
    if (id, class, name.as_str()) != (4, 2, "Table") || table.1 != 0 {
        return Err(Error::format("TABLESTYLE: unexpected table cell style"));
    }
    let n = s.dat.bl()?;
    if n != 3 {
        return Err(Error::format("TABLESTYLE: cell style count"));
    }
    let mut rows = Vec::new();
    for want in 1..=3u32 {
        let pre = s.dat.bl()?;
        let (row, merge) = read_cell_style(s, own)?;
        let (id, _class, _name) = (s.dat.bl()?, s.dat.bl()?, s.tv()?);
        if pre != want || id != want || merge != if want == 1 { 0x8000 } else { 0 } {
            return Err(Error::format("TABLESTYLE: unexpected cell style"));
        }
        rows.push(row);
    }
    let mut out = vec![
        (100, XrValue::Str("AcDbTableStyle".into())),
        (280, XrValue::Int8(version as i8)),
        (3, XrValue::Str(description)),
        (70, XrValue::Int16(flow as i16)),
        (71, XrValue::Int16(0)),
        (40, XrValue::Real(0.06)),
        (41, XrValue::Real(0.06)),
        (280, XrValue::Int8(0)),
        (281, XrValue::Int8(0)),
    ];
    // DXF order: data, title, header.
    for row in [&rows[2], &rows[0], &rows[1]] {
        out.push((7, XrValue::Str(d.style_name(row.text_style))));
        out.push((140, XrValue::Real(row.height)));
        out.push((170, XrValue::Int16(row.alignment as i16)));
        out.push((62, XrValue::Int16(row.text_color as i16)));
        out.push((63, XrValue::Int16(row.fill_color as i16)));
        out.push((283, XrValue::Int8(row.fill_on as i8)));
        out.push((90, XrValue::Int32(row.data_type as i32)));
        out.push((91, XrValue::Int32(row.unit_type as i32)));
        out.push((1, XrValue::Str(row.format.clone())));
        for k in 0..6u16 {
            out.push((274 + k, XrValue::Int16(DEFAULT_BORDER.0 as i16)));
            out.push((284 + k, XrValue::Int8(DEFAULT_BORDER.1 as i8)));
            out.push((64 + k, XrValue::Int16(DEFAULT_BORDER.2 as i16)));
        }
    }
    Ok(out)
}

/// A cell style with data, no margin overrides and no borders; returns the
/// row values and the merge flags.
fn read_cell_style(s: &mut Streams, own: u64) -> Result<(RowStyle, u32)> {
    let kind = s.dat.bl()?;
    let has_data = s.dat.bs()?;
    if kind != 5 || has_data != 1 {
        return Err(Error::format("TABLESTYLE: cell style without data"));
    }
    let overrides = s.dat.bl()?;
    let merge = s.dat.bl()?;
    let background = read(s, own, D::Cmc)?;
    let layout = s.dat.bl()?;
    let (cf90, cf91) = (s.dat.bl()?, s.dat.bl()?);
    let data_type = s.dat.bl()? as i64;
    let unit_type = s.dat.bl()? as i64;
    let format = s.tv()?;
    let rotation = s.dat.bd()?;
    let block_scale = s.dat.bd()?;
    let alignment = s.dat.bl()? as i64;
    let color = read(s, own, D::Cmc)?;
    let text_style = Handle(s.h_abs(own)?);
    let height = s.dat.bd()?;
    let margins = s.dat.bs()?;
    let borders = s.dat.bl()?;
    if overrides != 0 || layout != 1 || cf90 != 0 || cf91 != 0 || rotation != 0.0 || block_scale != 1.0 || margins != 0 || borders != 0 {
        return Err(Error::format("TABLESTYLE: cell style with overrides, margins or borders"));
    }
    let (fill_on, fill_color) = match background {
        Val::Color(0xC800_0000) => (false, 7),
        Val::Color(w) => (true, aci_of(w)),
        _ => (false, 7),
    };
    let text_color = match color {
        Val::Color(w) => aci_of(w),
        _ => 0,
    };
    Ok((RowStyle { text_style, height, alignment, text_color, fill_color, fill_on, data_type, unit_type, format }, merge))
}

fn aci_of(word: u32) -> i64 {
    let mut v = Vec::new();
    color_groups(62, word, &mut v);
    match v[0].1 {
        XrValue::Int16(i) => i as i64,
        _ => 7,
    }
}

fn write_cell_style(w: &mut ObjW, row: &RowStyle, merge: u32, text_style: Handle) {
    w.dat.bl(5);
    w.dat.bs(1);
    w.dat.bl(0);
    w.dat.bl(merge);
    let bg = if row.fill_on { word_of(row.fill_color, None) } else { 0xC800_0000 };
    write(w, D::Cmc, &Val::Color(bg)).ok();
    w.dat.bl(1);
    w.dat.bl(0);
    w.dat.bl(0);
    w.dat.bl(row.data_type as u32);
    w.dat.bl(row.unit_type as u32);
    w.tv(&row.format);
    w.dat.bd(0.0);
    w.dat.bd(1.0);
    w.dat.bl(row.alignment as u32);
    write(w, D::Cmc, &Val::Color(word_of(row.text_color, None))).ok();
    w.h(Ref::HardPtr, text_style);
    w.dat.bd(row.height);
    w.dat.bs(0);
    w.dat.bl(0);
}

/// Groups a table style's row may hold after its text style (7).
fn is_row_group(code: u16) -> bool {
    matches!(code, 140 | 170 | 62 | 63 | 283 | 90 | 91 | 1 | 274..=279 | 284..=289 | 64..=69)
}

/// One row style of a table style: its text style (7), then its groups up
/// to the next row's text style or the end, in whatever order the file has
/// them. AutoCAD interleaves each border's lineweight, visibility and
/// colour (274, 284, 64, 275, …); the DXF reference lists them by kind
/// (274–279, 284–289, 64–69), which other writers follow, and its older
/// layout has no cell data type, unit type or format (90, 91, 1), which
/// then take the values AutoCAD writes (512, 0, none).
fn row_style(c: &mut Cur<'_>, d: &Lookup) -> Result<RowStyle> {
    let style = c.str(7)?;
    let start = c.at;
    while c.peek().is_some_and(|code| code != 7) {
        c.at += 1;
    }
    let groups = c.items.get(start..c.at).unwrap_or_default();
    let kind = c.kind;
    if let Some((code, _)) = groups.iter().find(|(code, _)| !is_row_group(*code)) {
        return Err(Error::format(format!("{kind}: unexpected group {code} in a row style")));
    }
    let value = |code: u16| groups.iter().find(|(c, _)| *c == code).map(|(_, v)| v);
    let number = |code: u16, missing: Option<f64>| -> Result<f64> {
        match value(code).map(from_xr) {
            Some(Val::Int(i)) => Ok(i as f64),
            Some(Val::Real(f)) => Ok(f),
            Some(v) => Err(Error::format(format!("{kind}: group {code}: number expected, {v:?}"))),
            None => missing.ok_or_else(|| Error::format(format!("{kind}: a row style without group {code}"))),
        }
    };
    let int = |code: u16, missing: Option<i64>| number(code, missing.map(|m| m as f64)).map(|f| f as i64);
    let format = match value(1) {
        Some(XrValue::Str(s)) => s.clone(),
        Some(v) => return Err(Error::format(format!("{kind}: group 1: text expected, {v:?}"))),
        None => String::new(),
    };
    for k in 0..6u16 {
        let border = (int(274 + k, Some(DEFAULT_BORDER.0))?, int(284 + k, Some(DEFAULT_BORDER.1))?, int(64 + k, Some(DEFAULT_BORDER.2))?);
        if border != DEFAULT_BORDER {
            return Err(Error::unsupported("TABLESTYLE: borders other than the default"));
        }
    }
    Ok(RowStyle {
        text_style: d.style_handle(&style),
        height: number(140, None)?,
        alignment: int(170, None)?,
        text_color: int(62, None)?,
        fill_color: int(63, None)?,
        fill_on: int(283, None)? != 0,
        data_type: int(90, Some(512))?,
        unit_type: int(91, Some(0))?,
        format,
    })
}

fn encode_tablestyle(w: &mut ObjW, g: &Groups, d: &Lookup) -> Result<()> {
    let mut c = Cur::new(g);
    if c.str(100)? != "AcDbTableStyle" {
        return Err(Error::format("TABLESTYLE: subclass"));
    }
    // The DXF of AutoCAD 2007 and later starts with the version; the
    // reference's older layout, which other writers follow, has none.
    let version = if c.peek() == Some(280) { c.int(280)? } else { 0 };
    let description = c.str(3)?;
    let flow = c.int(70)?;
    let flags = c.int(71)?;
    let _h_margin = c.real(40)?;
    let _v_margin = c.real(41)?;
    let suppress_title = c.int(280)?;
    let suppress_header = c.int(281)?;
    if flags != 0 || suppress_title != 0 || suppress_header != 0 {
        return Err(Error::unsupported("TABLESTYLE: flags or suppressed rows"));
    }
    let mut rows = Vec::new();
    for _ in 0..3 {
        rows.push(row_style(&mut c, d)?);
    }
    c.done()?;
    let (data, title, header) = (&rows[0], &rows[1], &rows[2]);
    w.dat.rc(version as u8);
    w.tv(&description);
    w.dat.bl(flow as u32);
    w.dat.bl(101);
    w.h(Ref::HardOwner, Handle::NULL);
    // The "Table" cell style: AutoCAD's defaults, at the data rows' height.
    let table = RowStyle {
        text_style: Handle::NULL,
        alignment: 1,
        text_color: 0,
        fill_on: false,
        fill_color: 7,
        format: String::new(),
        data_type: 512,
        unit_type: 0,
        height: data.height,
    };
    write_cell_style(w, &table, 0, Handle::NULL);
    w.dat.bl(4);
    w.dat.bl(2);
    w.tv("Table");
    w.dat.bl(3);
    for (id, class, name, row, merge) in [(1u32, 1u32, "_TITLE", title, 0x8000u32), (2, 1, "_HEADER", header, 0), (3, 2, "_DATA", data, 0)] {
        w.dat.bl(id);
        write_cell_style(w, row, merge, row.text_style);
        w.dat.bl(id);
        w.dat.bl(class);
        w.tv(name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A style object's groups as DXF gives them, every value plain, and
    /// the repeated list claiming `claimed` items while holding two.
    fn groups(kind: &str, claimed: i32) -> Groups {
        let l = layout(kind).expect("a laid-out kind");
        let mut items = Vec::new();
        for f in &l.dxf {
            match f {
                G::M(name) => items.push((100, XrValue::Str(name.to_string()))),
                G::V(code, D::Tv) => items.push((*code, XrValue::Str(String::new()))),
                G::V(code, D::Bd) => items.push((*code, XrValue::Real(1.0))),
                G::V(code, D::H(_)) => items.push((*code, XrValue::Handle(Handle::NULL))),
                G::V(code, D::Cmc) => items.push((*code, XrValue::Int16(256))),
                G::V(code, _) => items.push((*code, XrValue::Int32(0))),
                G::X(code, v) => items.push((*code, XrValue::Int16(*v as i16))),
                G::OptH(_) => {}
                G::Tail(code, _, v) => items.push((*code, XrValue::Int16(*v as i16))),
                G::Rep(count, _, item, _) => {
                    items.push((*count, XrValue::Int32(claimed)));
                    items.push((*item, XrValue::Real(0.5)));
                    items.push((*item, XrValue::Real(0.25)));
                }
            }
        }
        Groups { kind: kind.into(), items }
    }

    #[test]
    fn a_claimed_count_does_not_size_the_list() {
        let l = layout("ACDBSECTIONVIEWSTYLE").expect("laid out");
        // Sound: two items, both written.
        let mut w = ObjW::new(V::R2018, Handle(0x100));
        encode_layout(&mut w, &groups("ACDBSECTIONVIEWSTYLE", 2), &l).unwrap();
        // Two billion claimed: the list holds what is there, and nothing
        // the size of the claim is allocated (that aborted the process).
        let mut w = ObjW::new(V::R2018, Handle(0x100));
        encode_layout(&mut w, &groups("ACDBSECTIONVIEWSTYLE", i32::MAX), &l).unwrap();
    }
}
