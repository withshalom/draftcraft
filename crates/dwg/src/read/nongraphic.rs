//! Non-graphical objects the drawing model keeps only as their original
//! DWG records (`Object::raw`): dictionaries, extension records, layouts,
//! scales and the like. The DXF writer turns them into DXF records, and
//! the DWG writer looks up the drawing's defaults (plot style, material)
//! through them.
//!
//! Layouts from the ODA specification, chapter 20, checked against the
//! objects of an AC1032 file written by AutoCAD 2027 and the DXF AutoCAD
//! saved of the same drawing (corpus/mosaiciron-mi201):
//! DICTIONARY §20.4.44, DICTIONARYWDFLT §20.4.45, MLINESTYLE §20.4.73,
//! DICTIONARYVAR §20.4.74, LAYOUT §20.4.84, SCALE §20.4.92, XRECORD
//! §20.4.104. ACDBPLACEHOLDER stores nothing of its own.

use super::object::{self, DecodeCtx};
use super::streams::Streams;
use super::version::DwgVersion as V;
use crate::read::error::{Error, Result};
use crate::read::geom::{Vec2, Vec3};
use crate::read::model::{ClassDef, Color, Drawing, Groups, Handle, Object, ObjectKind};
pub use crate::read::model::{
    Dictionary, DimStyleRecord, DimVars, Layout, MLineElement, MLineStyle, NonGraphic, PlotSettings, Scale, Vport, XRecord, XrValue,
};

/// Decode an object kept as its DWG record, when its type is one of the
/// above. `None` for other types; an error when the record does not fit
/// the layout.
pub fn decode(o: &Object, classes: &[ClassDef], codepage: u16) -> Option<Result<NonGraphic>> {
    if let ObjectKind::NonGraphic(n) = &o.kind {
        return Some(Ok(n.clone()));
    }
    let raw = o.raw.as_ref()?;
    kind_of(&o.type_name, false)?;
    let buf = object::raw_buffer(raw);
    let ctx = DecodeCtx { ver: raw.version, codepage, classes, text: Default::default() };
    let mut op = match object::open(&ctx, &buf, 0) {
        Ok(op) => op,
        Err(e) => return Some(Err(e)),
    };
    decode_opened(&mut op, &super::styles::Lookup::from_text_styles(Vec::new()), false).map(|r| r.map(|(n, _)| n))
}

/// The common handles of an object: its owner, its reactors and its extension dictionary.
pub(crate) type CommonHandles = (u64, Vec<Handle>, u64);

/// Which decoder reads an object of `type_name`; the style objects kept as their DXF groups
/// ([`Groups::KINDS`]) only when `groups`.
fn kind_of(type_name: &str, groups: bool) -> Option<u8> {
    Some(match type_name {
        "DICTIONARY" | "ACDBDICTIONARYWDFLT" => 0,
        "XRECORD" => 1,
        "ACDBPLACEHOLDER" => 2,
        "DICTIONARYVAR" => 3,
        "SCALE" => 4,
        "LAYOUT" => 5,
        "MLINESTYLE" => 6,
        k if groups && Groups::KINDS.contains(&k) => 7,
        _ => return None,
    })
}

/// [`decode`] for an object already opened, with its common handles; with `groups`, also the
/// style objects of [`Groups::KINDS`] as their DXF groups (text styles named by `lookup`).
/// `None` for another type, or a style object of a version its codec does not read.
pub(crate) fn decode_opened(
    op: &mut object::Opened<'_>,
    lookup: &super::styles::Lookup,
    groups: bool,
) -> Option<Result<(NonGraphic, CommonHandles)>> {
    let kind = kind_of(&op.type_name, groups)?;
    if kind == 7 {
        let common = match object::common_handles(op) {
            Ok((owner, reactors, xdictionary)) => (owner, reactors, xdictionary),
            Err(e) => return Some(Err(e)),
        };
        let (own, type_name) = (op.handle, op.type_name.clone());
        return super::styles::decode(&type_name, &mut op.s, own, lookup).map(|r| r.map(|g| (NonGraphic::Groups(g), common)));
    }
    Some((|| {
        let wdflt = op.type_name == "ACDBDICTIONARYWDFLT";
        // Type data first, then the handles: common ones, then the type's.
        let partial = match kind {
            0 => Partial::Dict(dictionary_data(&mut op.s)?),
            1 => Partial::XRec(xrecord_data(&mut op.s)?),
            2 => Partial::Done(NonGraphic::Placeholder),
            3 => {
                let schema = op.s.dat.rc()?;
                let value = op.s.tv()?;
                Partial::Done(NonGraphic::DictionaryVar { schema, value })
            }
            4 => Partial::Done(NonGraphic::Scale(Scale {
                flag: op.s.dat.bs()?,
                name: op.s.tv()?,
                paper_units: op.s.dat.bd()?,
                drawing_units: op.s.dat.bd()?,
                unit_scale: op.s.dat.b()?,
            })),
            5 => Partial::Layout(Box::new(layout_data(&mut op.s)?)),
            _ => Partial::MLine(mlinestyle_data(&mut op.s)?),
        };
        let common = object::common_handles(op)?;
        let own = op.handle;
        let s = &mut op.s;
        let n = match partial {
            Partial::Done(n) => n,
            Partial::Dict((mut d, names)) => {
                for name in names {
                    let h = Handle(s.h_abs(own)?);
                    d.entries.push((name, h));
                }
                if wdflt {
                    d.default = Some(Handle(s.h_abs(own)?));
                }
                NonGraphic::Dictionary(d)
            }
            Partial::XRec(x) => NonGraphic::XRecord(x),
            Partial::Layout(mut l) => {
                if s.ver >= V::R2004 {
                    l.plot.plot_view = Handle(s.h_abs(own)?);
                }
                if s.ver >= V::R2007 {
                    l.plot.visual_style = Handle(s.h_abs(own)?);
                }
                l.block = Handle(s.h_abs(own)?);
                l.last_viewport = Handle(s.h_abs(own)?);
                l.base_ucs = Handle(s.h_abs(own)?);
                l.named_ucs = Handle(s.h_abs(own)?);
                let n = l.viewports.len();
                l.viewports.clear();
                for _ in 0..n {
                    l.viewports.push(Handle(s.h_abs(own)?));
                }
                NonGraphic::Layout(l)
            }
            Partial::MLine(mut m) => {
                if s.ver >= V::R2018 {
                    for e in &mut m.elements {
                        e.linetype = Handle(s.h_abs(own)?);
                    }
                }
                NonGraphic::MLineStyle(m)
            }
        };
        Ok((n, common))
    })())
}

enum Partial {
    Done(NonGraphic),
    Dict((Dictionary, Vec<String>)),
    XRec(XRecord),
    Layout(Box<Layout>),
    MLine(MLineStyle),
}

fn dictionary_data(s: &mut Streams) -> Result<(Dictionary, Vec<String>)> {
    let n = s.dat.bl()?;
    if s.ver == V::R14 {
        let _unknown = s.dat.rc()?;
    }
    let (cloning, hard_owner) = if s.ver >= V::R2000 { (s.dat.bs()?, s.dat.rc()?) } else { (0, 0) };
    let n = s.check_handle_count(n, "dictionary entry")?;
    let mut names = Vec::with_capacity(n);
    for _ in 0..n {
        names.push(s.tv()?);
    }
    Ok((Dictionary { cloning, hard_owner, entries: Vec::new(), default: None }, names))
}

/// The binary form of an XRECORD value, by group code (DXF reference,
/// "Group Code Value Types"; sizes as R2007+ files store them).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum XrType {
    Str,
    Point,
    Real,
    Int8,
    Int16,
    Int32,
    Int64,
    Bool,
    Bytes,
    Handle,
}

/// The binary form of an XRECORD value with this group code, if it has one.
pub fn xr_type(code: u16) -> Option<XrType> {
    use XrType::*;
    Some(match code {
        0..=9 | 100..=102 | 300..=309 | 410..=419 | 430..=439 | 470..=479 | 999 | 1000..=1003 => Str,
        10..=18 | 110..=112 | 210 | 1010..=1013 => Point,
        19..=59 | 113..=149 | 211..=239 | 460..=469 | 1014..=1059 => Real,
        60..=79 | 170..=179 | 270..=279 | 370..=389 | 400..=409 | 1060..=1070 => Int16,
        280..=289 => Int8,
        90..=99 | 420..=429 | 440..=459 | 1071 => Int32,
        160..=169 => Int64,
        290..=299 => Bool,
        310..=319 | 1004 => Bytes,
        105 | 320..=369 | 390..=399 | 480..=481 | 1005 => Handle,
        _ => return None,
    })
}

fn xrecord_data(s: &mut Streams) -> Result<XRecord> {
    let n = s.dat.bl()? as usize;
    if n > s.dat.remaining() / 8 {
        return Err(Error::format("XRECORD data runs past the record"));
    }
    let bytes = s.dat.raw_bytes(n)?;
    let cloning = if s.ver >= V::R2000 { s.dat.bs()? } else { 0 };
    let items = xrecord_items(&bytes, s.ver, s.text.ansi, s.codepage)?;
    Ok(XRecord { cloning, items })
}

/// `b` as the bytes of an `N`-byte little-endian number. The callers take
/// exactly `N` bytes; were any missing, they would read as zero.
fn le<const N: usize>(b: &[u8]) -> [u8; N] {
    let mut a = [0u8; N];
    for (x, y) in a.iter_mut().zip(b) {
        *x = *y;
    }
    a
}

/// The items of XRECORD data `d`: 8-bit strings (before R2007) in the code page `codepage`, by
/// `ansi`.
fn xrecord_items(d: &[u8], ver: V, ansi: fn(&[u8], u16) -> String, codepage: u16) -> Result<Vec<(u16, XrValue)>> {
    let mut p = 0usize;
    let mut out = Vec::new();
    let take = |p: &mut usize, n: usize| -> Result<&[u8]> {
        let s = d.get(*p..*p + n).ok_or_else(|| Error::format("XRECORD value runs past its data"))?;
        *p += n;
        Ok(s)
    };
    while p < d.len() {
        let code = u16::from_le_bytes(le(take(&mut p, 2)?));
        let t = xr_type(code).ok_or_else(|| Error::format(format!("XRECORD group code {code} not known")))?;
        let f64_at = |b: &[u8]| f64::from_le_bytes(le(b));
        let v = match t {
            XrType::Str => {
                if ver.has_string_stream() {
                    let len = u16::from_le_bytes(le(take(&mut p, 2)?)) as usize;
                    let units: Vec<u16> = take(&mut p, len * 2)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
                    XrValue::Str(String::from_utf16_lossy(&units))
                } else {
                    // Before R2007: a 16-bit length, the code page, the bytes (AutoCAD's and
                    // acadrust's files alike).
                    let len = u16::from_le_bytes(le(take(&mut p, 2)?)) as usize;
                    let _codepage = take(&mut p, 1)?;
                    XrValue::Str(ansi(take(&mut p, len)?, codepage))
                }
            }
            XrType::Point => {
                let b = take(&mut p, 24)?;
                XrValue::Point(Vec3::new(f64_at(&b[..8]), f64_at(&b[8..16]), f64_at(&b[16..])))
            }
            XrType::Real => XrValue::Real(f64_at(take(&mut p, 8)?)),
            XrType::Int8 => XrValue::Int8(take(&mut p, 1)?[0] as i8),
            XrType::Int16 => XrValue::Int16(i16::from_le_bytes(le(take(&mut p, 2)?))),
            XrType::Int32 => XrValue::Int32(i32::from_le_bytes(le(take(&mut p, 4)?))),
            XrType::Int64 => XrValue::Int64(i64::from_le_bytes(le(take(&mut p, 8)?))),
            XrType::Bool => XrValue::Bool(take(&mut p, 1)?[0] != 0),
            XrType::Bytes => {
                let len = take(&mut p, 1)?[0] as usize;
                XrValue::Bytes(take(&mut p, len)?.to_vec())
            }
            XrType::Handle => XrValue::Handle(Handle(u64::from_le_bytes(le(take(&mut p, 8)?)))),
        };
        out.push((code, v));
    }
    Ok(out)
}

fn v2(a: [f64; 2]) -> Vec2 {
    Vec2::from_array(a)
}
fn v3(a: [f64; 3]) -> Vec3 {
    Vec3::from_array(a)
}

fn layout_data(s: &mut Streams) -> Result<Layout> {
    let mut p = PlotSettings { page_setup_name: s.tv()?, printer: s.tv()?, flags: s.dat.bs()?, ..Default::default() };
    for m in &mut p.margins {
        *m = s.dat.bd()?;
    }
    p.paper_width = s.dat.bd()?;
    p.paper_height = s.dat.bd()?;
    p.paper_size = s.tv()?;
    p.origin = v2(s.dat.bd2()?);
    p.paper_units = s.dat.bs()?;
    p.rotation = s.dat.bs()?;
    p.plot_type = s.dat.bs()?;
    p.window_min = v2(s.dat.bd2()?);
    p.window_max = v2(s.dat.bd2()?);
    if s.ver <= V::R2000 {
        let _view_name = s.tv()?;
    }
    p.real_world_units = s.dat.bd()?;
    p.drawing_units = s.dat.bd()?;
    p.style_sheet = s.tv()?;
    p.scale_type = s.dat.bs()?;
    p.scale_factor = s.dat.bd()?;
    p.image_origin = v2(s.dat.bd2()?);
    if s.ver >= V::R2004 {
        p.shade_plot_mode = s.dat.bs()?;
        p.shade_plot_res_level = s.dat.bs()?;
        p.shade_plot_custom_dpi = s.dat.bs()?;
    }
    let mut l = Layout {
        plot: p,
        name: s.tv()?,
        tab_order: s.dat.bl()?,
        flags: s.dat.bs()?,
        ucs_origin: v3(s.dat.bd3()?),
        limmin: v2(s.dat.rd2()?),
        limmax: v2(s.dat.rd2()?),
        insbase: v3(s.dat.bd3()?),
        ucs_x: v3(s.dat.bd3()?),
        ucs_y: v3(s.dat.bd3()?),
        elevation: s.dat.bd()?,
        ortho_type: s.dat.bs()?,
        extmin: v3(s.dat.bd3()?),
        extmax: v3(s.dat.bd3()?),
        ..Default::default()
    };
    if s.ver >= V::R2004 {
        // The spec gives an RL; AutoCAD 2027's AC1032 layouts store a BL
        // (only that reading ends where their strings begin).
        let n = s.dat.bl()?;
        let n = s.check_handle_count(n, "layout viewport")?;
        l.viewports = vec![Handle::NULL; n];
    }
    Ok(l)
}

/// DWG keeps the MLINESTYLE flag bits in a different order from DXF 70.
const MLINE_FLAG_BITS: [(u16, u16); 8] = [(1, 2), (2, 1), (16, 16), (32, 64), (64, 32), (256, 256), (512, 1024), (1024, 512)];

pub(crate) fn mline_flags_to_dxf(dwg: u16) -> u16 {
    MLINE_FLAG_BITS.iter().filter(|(d, _)| dwg & d != 0).fold(0, |acc, (_, x)| acc | x)
}

pub(crate) fn mline_flags_to_dwg(dxf: u16) -> u16 {
    MLINE_FLAG_BITS.iter().filter(|(_, x)| dxf & x != 0).fold(0, |acc, (d, _)| acc | d)
}

fn mlinestyle_data(s: &mut Streams) -> Result<MLineStyle> {
    let name = s.tv()?;
    let description = s.tv()?;
    let flags = mline_flags_to_dxf(s.dat.bs()?);
    let (fill_color, _) = s.cmc()?;
    let start_angle = s.dat.bd()?;
    let end_angle = s.dat.bd()?;
    let n = s.dat.rc()? as usize;
    let mut elements = Vec::with_capacity(n);
    for _ in 0..n {
        let offset = s.dat.bd()?;
        let (color, _) = s.cmc()?;
        let linetype_index = if s.ver < V::R2018 { s.dat.bs()? as i16 } else { 0 };
        elements.push(MLineElement { offset, color, linetype: Handle::NULL, linetype_index });
    }
    Ok(MLineStyle { name, description, flags, fill_color, start_angle, end_angle, elements })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Option<Drawing> {
        let p = std::path::PathBuf::from(std::env::var_os("MI201_CORPUS")?).join("mi201-r2018.dwg");
        std::fs::read(p).ok().map(|b| super::super::read_bytes(&b).unwrap())
    }

    /// Every dictionary, extension record, layout, scale and style of the
    /// AutoCAD 2027 file decodes, and the values match what AutoCAD's own
    /// DXF of the drawing shows.
    #[test]
    fn autocad_objects_decode_as_its_dxf_shows() {
        let Some(d) = corpus() else { return };
        let mut counts = std::collections::BTreeMap::<&str, usize>::new();
        for o in d.objects.values() {
            match decode(o, &d.classes, d.codepage) {
                None => {}
                Some(Ok(_)) => *counts.entry(o.type_name.as_str()).or_default() += 1,
                Some(Err(e)) => panic!("{} {:X}: {e}", o.type_name, o.handle.0),
            }
        }
        assert_eq!(counts["DICTIONARY"], 19);
        assert_eq!(counts["XRECORD"], 20);
        assert_eq!(counts["SCALE"], 17);
        assert_eq!(counts["LAYOUT"], 2);
        let get = |h: u64| decode(&d.objects[&Handle(h)], &d.classes, d.codepage).unwrap().unwrap();
        // The named object dictionary.
        match get(0x3F) {
            NonGraphic::Dictionary(dict) => {
                assert!(dict.entries.iter().any(|e| e == &("ACAD_GROUP".to_string(), Handle(0x3E))));
                assert!(dict.entries.iter().any(|e| e == &("ACAD_LAYOUT".to_string(), Handle(0x3D0C9))));
            }
            other => panic!("{other:?}"),
        }
        match get(0x3D094) {
            NonGraphic::Dictionary(dict) => {
                assert_eq!(dict.entries, vec![("Normal".to_string(), Handle(0x3D095))]);
                assert_eq!(dict.default, Some(Handle(0x3D095)));
            }
            other => panic!("{other:?}"),
        }
        match get(0x3D0EA) {
            NonGraphic::XRecord(x) => {
                assert_eq!(x.cloning, 1);
                assert_eq!(x.items, vec![(300, XrValue::Str("ACD".into())), (300, XrValue::Str("2027".into())), (300, XrValue::Str(String::new())),]);
            }
            other => panic!("{other:?}"),
        }
        match get(0x3D0A2) {
            NonGraphic::XRecord(x) => {
                assert_eq!(x.items, vec![(270, XrValue::Int16(1)), (271, XrValue::Int16(1))]);
            }
            other => panic!("{other:?}"),
        }
        match get(0x3D0E7) {
            NonGraphic::Layout(l) => {
                assert_eq!(l.name, "Model");
                assert_eq!(l.block, Handle(0x17));
                assert_eq!(l.last_viewport, Handle(0x3));
            }
            other => panic!("{other:?}"),
        }
        match get(0x3D0CD) {
            NonGraphic::MLineStyle(m) => {
                assert_eq!(m.name, "STANDARD");
                assert_eq!(m.elements.len(), 2);
                assert_eq!(m.elements[0].linetype, Handle(0x6));
                assert_eq!(m.elements[0].offset, 0.5);
            }
            other => panic!("{other:?}"),
        }
    }
}

/// The dimension variables the header holds (the current style's values
/// unless overridden).
pub fn dim_vars_from_header(hv: &super::hvars::Hv) -> DimVars {
    let r = |n: &str, d: f64| hv.real(n, d);
    let i = |n: &str, d: i64| hv.int(n, d) as u16;
    let b = |n: &str, d: bool| hv.bool(n, d);
    DimVars {
        post: hv.text("DIMPOST", ""),
        apost: hv.text("DIMAPOST", ""),
        scale: r("DIMSCALE", 1.0),
        asz: r("DIMASZ", 0.18),
        exo: r("DIMEXO", 0.0625),
        dli: r("DIMDLI", 0.38),
        exe: r("DIMEXE", 0.18),
        rnd: r("DIMRND", 0.0),
        dle: r("DIMDLE", 0.0),
        tp: r("DIMTP", 0.0),
        tm: r("DIMTM", 0.0),
        fxl: r("DIMFXL", 1.0),
        jogang: r("DIMJOGANG", std::f64::consts::FRAC_PI_4),
        tfill: i("DIMTFILL", 0),
        tfillclr: hv.color("DIMTFILLCLR", Color::ByBlock),
        tol: b("DIMTOL", false),
        lim: b("DIMLIM", false),
        tih: b("DIMTIH", true),
        toh: b("DIMTOH", true),
        se1: b("DIMSE1", false),
        se2: b("DIMSE2", false),
        tad: i("DIMTAD", 0),
        zin: i("DIMZIN", 0),
        azin: i("DIMAZIN", 0),
        arcsym: i("DIMARCSYM", 0),
        txt: r("DIMTXT", 0.18),
        cen: r("DIMCEN", 0.09),
        tsz: r("DIMTSZ", 0.0),
        altf: r("DIMALTF", 25.4),
        lfac: r("DIMLFAC", 1.0),
        tvp: r("DIMTVP", 0.0),
        tfac: r("DIMTFAC", 1.0),
        gap: r("DIMGAP", 0.09),
        altrnd: r("DIMALTRND", 0.0),
        alt: b("DIMALT", false),
        altd: i("DIMALTD", 2),
        tofl: b("DIMTOFL", false),
        sah: b("DIMSAH", false),
        tix: b("DIMTIX", false),
        soxd: b("DIMSOXD", false),
        clrd: hv.color("DIMCLRD", Color::ByBlock),
        clre: hv.color("DIMCLRE", Color::ByBlock),
        clrt: hv.color("DIMCLRT", Color::ByBlock),
        adec: i("DIMADEC", 0),
        dec: i("DIMDEC", 4),
        tdec: i("DIMTDEC", 4),
        altu: i("DIMALTU", 2),
        alttd: i("DIMALTTD", 2),
        aunit: i("DIMAUNIT", 0),
        frac: i("DIMFRAC", 0),
        lunit: i("DIMLUNIT", 2),
        dsep: i("DIMDSEP", b'.' as i64),
        tmove: i("DIMTMOVE", 0),
        just: i("DIMJUST", 0),
        sd1: b("DIMSD1", false),
        sd2: b("DIMSD2", false),
        tolj: i("DIMTOLJ", 1),
        tzin: i("DIMTZIN", 0),
        altz: i("DIMALTZ", 0),
        alttz: i("DIMALTTZ", 0),
        upt: b("DIMUPT", false),
        fit: i("DIMATFIT", 3),
        fxlon: b("DIMFXLON", false),
        txtdirection: b("DIMTXTDIRECTION", false),
        altmzf: r("DIMALTMZF", 100.0),
        altmzs: hv.text("DIMALTMZS", ""),
        mzf: r("DIMMZF", 100.0),
        mzs: hv.text("DIMMZS", ""),
        lwd: hv.int("DIMLWD", -2) as i16,
        lwe: hv.int("DIMLWE", -2) as i16,
        flag_bit: false,
        txsty: hv.handle("DIMTXSTY"),
        ldrblk: hv.handle("DIMLDRBLK"),
        blk: hv.handle("DIMBLK"),
        blk1: hv.handle("DIMBLK1"),
        blk2: hv.handle("DIMBLK2"),
        ltype: hv.handle("DIMLTYPE"),
        ltex1: hv.handle("DIMLTEX1"),
        ltex2: hv.handle("DIMLTEX2"),
    }
}

/// A DIMSTYLE record kept as its DWG bytes: the name, flags and every
/// variable. `None` for R13–R14 records (laid out differently).
pub fn dimstyle(o: &Object, classes: &[ClassDef], codepage: u16) -> Option<Result<(String, DimVars)>> {
    if let ObjectKind::DimStyleRecord(r) = &o.kind {
        return Some(Ok((r.name.clone(), r.vars.clone())));
    }
    let raw = o.raw.as_ref()?;
    if o.type_code != 0x45 || raw.version < V::R2000 {
        return None;
    }
    let buf = object::raw_buffer(raw);
    let ctx = DecodeCtx { ver: raw.version, codepage, classes, text: Default::default() };
    Some((|| {
        let mut op = object::open(&ctx, &buf, 0)?;
        let s = &mut op.s;
        let name = record_name(s)?;
        let ver = s.ver;
        let post = s.tv()?;
        let apost = s.tv()?;
        let (scale, asz, exo, dli, exe, rnd, dle, tp, tm) =
            (s.dat.bd()?, s.dat.bd()?, s.dat.bd()?, s.dat.bd()?, s.dat.bd()?, s.dat.bd()?, s.dat.bd()?, s.dat.bd()?, s.dat.bd()?);
        let (mut fxl, mut jogang, mut tfill, mut tfillclr) = (1.0, std::f64::consts::FRAC_PI_4, 0, Color::ByBlock);
        if ver >= V::R2007 {
            fxl = s.dat.bd()?;
            jogang = s.dat.bd()?;
            tfill = s.dat.bs()?;
            tfillclr = s.cmc()?.0;
        }
        let (tol, lim, tih, toh, se1, se2) = (s.dat.b()?, s.dat.b()?, s.dat.b()?, s.dat.b()?, s.dat.b()?, s.dat.b()?);
        let (tad, zin, azin) = (s.dat.bs()?, s.dat.bs()?, s.dat.bs()?);
        let arcsym = if ver >= V::R2007 { s.dat.bs()? } else { 0 };
        let (txt, cen, tsz, altf, lfac, tvp, tfac, gap, altrnd) =
            (s.dat.bd()?, s.dat.bd()?, s.dat.bd()?, s.dat.bd()?, s.dat.bd()?, s.dat.bd()?, s.dat.bd()?, s.dat.bd()?, s.dat.bd()?);
        let alt = s.dat.b()?;
        let altd = s.dat.bs()?;
        let (tofl, sah, tix, soxd) = (s.dat.b()?, s.dat.b()?, s.dat.b()?, s.dat.b()?);
        let clrd = s.cmc()?.0;
        let clre = s.cmc()?.0;
        let clrt = s.cmc()?.0;
        let mut n = [0u16; 10];
        for v in &mut n {
            *v = s.dat.bs()?;
        }
        let [adec, dec, tdec, altu, alttd, aunit, frac, lunit, dsep, tmove] = n;
        let just = s.dat.bs()?;
        let sd1 = s.dat.b()?;
        let sd2 = s.dat.b()?;
        let (tolj, tzin, altz, alttz) = (s.dat.bs()?, s.dat.bs()?, s.dat.bs()?, s.dat.bs()?);
        let upt = s.dat.b()?;
        let fit = s.dat.bs()?;
        let fxlon = if ver >= V::R2007 { s.dat.b()? } else { false };
        let (mut txtdirection, mut altmzf, mut altmzs, mut mzf, mut mzs) = (false, 100.0, String::new(), 100.0, String::new());
        if ver >= V::R2010 {
            txtdirection = s.dat.b()?;
            altmzf = s.dat.bd()?;
            altmzs = s.tv()?;
            mzf = s.dat.bd()?;
            mzs = s.tv()?;
        }
        let lwd = s.dat.bs()? as i16;
        let lwe = s.dat.bs()? as i16;
        let flag_bit = s.dat.b()?;
        let _ = object::common_handles(&mut op)?;
        let own = op.handle;
        let s = &mut op.s;
        let _xref_block = s.h()?;
        let mut h = || -> Result<Handle> { Ok(Handle(s.h_abs(own)?)) };
        let txsty = h()?;
        let (ldrblk, blk, blk1, blk2) = (h()?, h()?, h()?, h()?);
        let (mut ltype, mut ltex1, mut ltex2) = (Handle::NULL, Handle::NULL, Handle::NULL);
        if ver >= V::R2007 {
            ltype = h()?;
            ltex1 = h()?;
            ltex2 = h()?;
        }
        Ok((
            name,
            DimVars {
                post,
                apost,
                scale,
                asz,
                exo,
                dli,
                exe,
                rnd,
                dle,
                tp,
                tm,
                fxl,
                jogang,
                tfill,
                tfillclr,
                tol,
                lim,
                tih,
                toh,
                se1,
                se2,
                tad,
                zin,
                azin,
                arcsym,
                txt,
                cen,
                tsz,
                altf,
                lfac,
                tvp,
                tfac,
                gap,
                altrnd,
                alt,
                altd,
                tofl,
                sah,
                tix,
                soxd,
                clrd,
                clre,
                clrt,
                adec,
                dec,
                tdec,
                altu,
                alttd,
                aunit,
                frac,
                lunit,
                dsep,
                tmove,
                just,
                sd1,
                sd2,
                tolj,
                tzin,
                altz,
                alttz,
                upt,
                fit,
                fxlon,
                txtdirection,
                altmzf,
                altmzs,
                mzf,
                mzs,
                lwd,
                lwe,
                flag_bit,
                txsty,
                ldrblk,
                blk,
                blk1,
                blk2,
                ltype,
                ltex1,
                ltex2,
            },
        ))
    })())
}

/// The common start of a table record: the name and (before R2007) the
/// 64-flag, xref index and xref-dependent bit.
fn record_name(s: &mut Streams) -> Result<String> {
    let name = s.tv()?;
    if !s.ver.has_string_stream() {
        let _flag64 = s.dat.b()?;
    }
    let _xref_index = s.dat.bs()?;
    if !s.ver.has_string_stream() {
        let _xdep = s.dat.b()?;
    }
    Ok(name)
}

/// A VPORT record kept as its DWG bytes. `None` before R2000.
pub fn vport(o: &Object, classes: &[ClassDef], codepage: u16) -> Option<Result<Vport>> {
    if let ObjectKind::Viewport(v) = &o.kind {
        return Some(Ok((**v).clone()));
    }
    let raw = o.raw.as_ref()?;
    if o.type_code != 0x41 || raw.version < V::R2000 {
        return None;
    }
    let buf = object::raw_buffer(raw);
    let ctx = DecodeCtx { ver: raw.version, codepage, classes, text: Default::default() };
    Some((|| {
        let mut op = object::open(&ctx, &buf, 0)?;
        let s = &mut op.s;
        let ver = s.ver;
        let name = record_name(s)?;
        let view_height = s.dat.bd()?;
        let width = s.dat.bd()?;
        let center = v2(s.dat.rd2()?);
        let target = v3(s.dat.bd3()?);
        let direction = v3(s.dat.bd3()?);
        let twist = s.dat.bd()?;
        let lens = s.dat.bd()?;
        let front_clip = s.dat.bd()?;
        let back_clip = s.dat.bd()?;
        let mode = s.dat.bits(4)? as u16;
        // Bits 0–2 are 71's bits 0–2; bit 3 is the opposite of 71's 16.
        let mut view_mode = ((mode >> 3) & 1) | (((mode >> 2) & 1) << 1) | (((mode >> 1) & 1) << 2);
        if mode & 1 == 0 {
            view_mode |= 16;
        }
        let render_mode = s.dat.rc()?;
        let mut v = Vport::looking_at(Vec2::ZERO, Vec2::new(1.0, 1.0));
        if ver >= V::R2007 {
            v.default_lights = s.dat.b()?;
            v.lighting_type = s.dat.rc()?;
            v.brightness = s.dat.bd()?;
            v.contrast = s.dat.bd()?;
            v.ambient = s.cmc()?.0;
        }
        v.lower_left = v2(s.dat.rd2()?);
        v.upper_right = v2(s.dat.rd2()?);
        if s.dat.b()? {
            view_mode |= 8;
        }
        v.circle_zoom = s.dat.bs()?;
        v.fast_zoom = s.dat.b()?;
        v.ucsicon = s.dat.bits(2)? as u8;
        v.grid_on = s.dat.b()?;
        v.grid_spacing = v2(s.dat.rd2()?);
        v.snap_on = s.dat.b()?;
        v.snap_style = s.dat.b()?;
        v.snap_isopair = s.dat.bs()?;
        v.snap_rotation = s.dat.bd()?;
        v.snap_base = v2(s.dat.rd2()?);
        v.snap_spacing = v2(s.dat.rd2()?);
        v.unknown_bit = s.dat.b()?;
        v.ucs_per_viewport = s.dat.b()?;
        v.ucs_origin = v3(s.dat.bd3()?);
        v.ucs_x = v3(s.dat.bd3()?);
        v.ucs_y = v3(s.dat.bd3()?);
        v.ucs_elevation = s.dat.bd()?;
        v.ucs_ortho_type = s.dat.bs()?;
        if ver >= V::R2007 {
            v.grid_flags = s.dat.bs()?;
            v.grid_major = s.dat.bs()?;
        }
        let _ = object::common_handles(&mut op)?;
        let own = op.handle;
        let s = &mut op.s;
        let _xref_block = s.h()?;
        if ver >= V::R2007 {
            v.background = Handle(s.h_abs(own)?);
            v.visual_style = Handle(s.h_abs(own)?);
            v.sun = Handle(s.h_abs(own)?);
        }
        v.named_ucs = Handle(s.h_abs(own)?);
        v.base_ucs = Handle(s.h_abs(own)?);
        Ok(Vport {
            name,
            view_height,
            aspect: if view_height != 0.0 { width / view_height } else { 1.0 },
            center,
            target,
            direction,
            twist,
            lens,
            front_clip,
            back_clip,
            view_mode,
            render_mode,
            ..v
        })
    })())
}

#[cfg(test)]
mod table_tests {
    use super::*;

    fn corpus() -> Option<Drawing> {
        let p = std::path::PathBuf::from(std::env::var_os("MI201_CORPUS")?).join("mi201-r2018.dwg");
        std::fs::read(p).ok().map(|b| super::super::read_bytes(&b).unwrap())
    }

    /// The *Active viewport and the Standard dimension style of the
    /// AutoCAD 2027 file hold what AutoCAD's DXF of it shows.
    #[test]
    fn vport_and_dimstyle_records_decode_as_the_dxf_shows() {
        let Some(d) = corpus() else { return };
        let v = vport(&d.objects[&Handle(3)], &d.classes, d.codepage).unwrap().unwrap();
        assert_eq!(v.name, "*Active");
        assert!((v.center.x - 129.0894530191407).abs() < 1e-9);
        assert!((v.center.y - 39.35701577680084).abs() < 1e-9);
        assert!((v.view_height - 16.58941231582867).abs() < 1e-9);
        assert!((v.aspect - 1.856966707768187).abs() < 1e-9);
        assert_eq!(v.lens, 50.0);
        assert_eq!((v.view_mode, v.circle_zoom, v.fast_zoom, v.ucsicon), (0, 100, true, 3));
        assert_eq!(v.snap_spacing, Vec2::new(0.5, 0.5));
        assert_eq!(v.grid_spacing, Vec2::new(0.5, 0.5));
        assert_eq!((v.grid_flags, v.grid_major), (3, 5));
        assert!(v.ucs_per_viewport && v.default_lights);
        assert_eq!(v.lighting_type, 1);
        assert_eq!(v.visual_style, Handle(0x3D0B5));
        let (name, s) = dimstyle(&d.objects[&Handle(0x15)], &d.classes, d.codepage).unwrap().unwrap();
        assert_eq!(name, "Standard");
        assert_eq!(s.txsty, Handle(0x11));
        assert_eq!((s.asz, s.exo, s.dli, s.txt, s.gap), (0.18, 0.0625, 0.38, 0.18, 0.09));
        assert_eq!((s.dec, s.tdec, s.lunit, s.dsep, s.fit, s.tolj), (4, 4, 2, 46, 3, 1));
        assert_eq!((s.clrd, s.lwd, s.lwe), (Color::ByBlock, -2, -2));
        assert_eq!(s.post, "");
    }
}

/// Turn objects kept as DWG records into the model's records where they
/// decode: VPORT and DIMSTYLE table records, the non-graphical objects
/// above, and tolerances, multilines, images and wipeouts. Their DWG bytes are then dropped; the DWG writer encodes them
/// again. Objects that do not decode stay as they were. Used by the DWG
/// reader, and by the native reader for files of format 2.1 and earlier.
/// A style object (`Groups::KINDS`) from its DWG record.
pub(crate) fn decode_groups(o: &Object, classes: &[ClassDef], codepage: u16, lookup: &super::styles::Lookup) -> Option<Result<Groups>> {
    if let ObjectKind::NonGraphic(NonGraphic::Groups(g)) = &o.kind {
        return Some(Ok(g.clone()));
    }
    if !Groups::KINDS.contains(&o.type_name.as_str()) {
        return None;
    }
    let raw = o.raw.as_ref()?;
    let buf = object::raw_buffer(raw);
    let ctx = DecodeCtx { ver: raw.version, codepage, classes, text: Default::default() };
    let mut op = match object::open(&ctx, &buf, 0) {
        Ok(op) => op,
        Err(e) => return Some(Err(e)),
    };
    if let Err(e) = object::common_handles(&mut op) {
        return Some(Err(e));
    }
    let own = op.handle;
    let kind = op.type_name.clone();
    super::styles::decode(&kind, &mut op.s, own, lookup)
}

pub fn promote(d: &mut Drawing) {
    let (classes, codepage) = (d.classes.clone(), d.codepage);
    let lookup = super::styles::Lookup::new(d);
    for o in d.objects.values_mut() {
        if o.raw.is_none() {
            continue;
        }
        let kind = match (o.type_code, &o.kind) {
            (0x41, ObjectKind::NamedRecord(_)) => match vport(o, &classes, codepage) {
                Some(Ok(v)) => Some(ObjectKind::Viewport(Box::new(v))),
                _ => None,
            },
            (0x45, ObjectKind::NamedRecord(_)) => match dimstyle(o, &classes, codepage) {
                Some(Ok((name, vars))) => Some(ObjectKind::DimStyleRecord(Box::new(DimStyleRecord { name, vars }))),
                _ => None,
            },
            // Entities kept as bytes by readers from before their types
            // were decoded (tolerances, multilines, images, wipeouts in
            // older .cad files): decoded now, when the record fits.
            (_, ObjectKind::Unsupported) if matches!(o.type_code, 0x2E | 0x2F) || matches!(o.type_name.as_str(), "IMAGE" | "WIPEOUT") => {
                o.raw.as_ref().and_then(|raw| {
                    let ctx = DecodeCtx { ver: raw.version, codepage, classes: &classes, text: Default::default() };
                    let k = object::decode(&ctx, &object::raw_buffer(raw), 0).ok()?.kind;
                    matches!(k, ObjectKind::Tolerance(_) | ObjectKind::MLine(_) | ObjectKind::Image(_) | ObjectKind::Wipeout(_)).then_some(k)
                })
            }
            (_, ObjectKind::Unsupported) => match decode(o, &classes, codepage) {
                Some(Ok(n)) => Some(ObjectKind::NonGraphic(n)),
                _ => match decode_groups(o, &classes, codepage, &lookup) {
                    Some(Ok(g)) => Some(ObjectKind::NonGraphic(NonGraphic::Groups(g))),
                    _ => None,
                },
            },
            _ => None,
        };
        if let Some(k) = kind {
            o.kind = k;
            o.raw = None;
        }
    }
}
