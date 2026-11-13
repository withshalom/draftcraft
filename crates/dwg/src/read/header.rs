//! AcDb:Header — header variables.
//!
//! Written from knowledge of the spec, chapter 9. The variable list is kept
//! as a table so corrections are one-line edits once it is checked against
//! the spec PDF. The parse is self-validating: it only counts as verified
//! when the control-object and space handles it reads resolve to objects
//! of the right types in the object map.

use super::section::{self, HEADER_END, HEADER_START};
use super::streams::{Streams, string_stream};
use super::version::DwgVersion as V;
use super::version::DwgVersion;
use crate::bits::BitReader;
use crate::read::error::{Error, Result};
use crate::read::geom::{Vec2, Vec3};
use crate::read::model::{Handle, HeaderVars};

#[derive(Clone, Copy, Debug)]
enum K {
    B,
    Bs,
    Bl,
    Bll,
    Bd,
    Rd2,
    Bd3,
    Rc,
    Tv,
    Cmc,
    /// Handle (handle stream from R2007).
    H,
    /// HANDSEED: a handle in the main stream.
    Seed,
    /// Julian day + milliseconds, two BLs.
    Time,
}

#[derive(Clone, Copy, Debug)]
enum W {
    All,
    R13R14,
    R13R2000,
    R2000P,
    R2004P,
    R2007P,
    R2010P,
    R2013P,
    /// CPSNID: only when CEPSNTYPE == 3.
    Cpsnid,
}

fn applies(w: W, v: DwgVersion, cepsntype: i64) -> bool {
    match w {
        W::All => true,
        W::R13R14 => v <= V::R14,
        W::R13R2000 => v <= V::R2000,
        W::R2000P => v >= V::R2000,
        W::R2004P => v >= V::R2004,
        W::R2007P => v >= V::R2007,
        W::R2010P => v >= V::R2010,
        W::R2013P => v >= V::R2013,
        W::Cpsnid => v >= V::R2000 && cepsntype == 3,
    }
}

use K::*;
use W::*;

/// Header variables in file order (spec chapter 9, from knowledge).
#[rustfmt::skip]
static VARS: &[(&str, K, W)] = &[
    ("REQUIREDVERSIONS", Bll, R2013P),
    ("UNKNOWN_BD_412148564080", Bd, All),
    ("UNKNOWN_BD_1A", Bd, All),
    ("UNKNOWN_BD_1B", Bd, All),
    ("UNKNOWN_BD_1C", Bd, All),
    ("UNKNOWN_TV_1", Tv, All),
    ("UNKNOWN_TV_2", Tv, All),
    ("UNKNOWN_TV_3", Tv, All),
    ("UNKNOWN_TV_4", Tv, All),
    ("UNKNOWN_BL_24", Bl, All),
    ("UNKNOWN_BL_0", Bl, All),
    ("UNKNOWN_BS_0", Bs, R13R14),
    ("CURRENT_VIEWPORT_ENTITY_HEADER", H, R13R2000),
    ("DIMASO", B, All),
    ("DIMSHO", B, All),
    ("DIMSAV", B, R13R14),
    ("PLINEGEN", B, All),
    ("ORTHOMODE", B, All),
    ("REGENMODE", B, All),
    ("FILLMODE", B, All),
    ("QTEXTMODE", B, All),
    ("PSLTSCALE", B, All),
    ("LIMCHECK", B, All),
    ("BLIPMODE", B, R13R14),
    ("UNKNOWN_B_R2004", B, R2004P),
    ("USRTIMER", B, All),
    ("SKPOLY", B, All),
    ("ANGDIR", B, All),
    ("SPLFRAME", B, All),
    ("ATTREQ", B, R13R14),
    ("ATTDIA", B, R13R14),
    ("MIRRTEXT", B, All),
    ("WORLDVIEW", B, All),
    ("WIREFRAME", B, R13R14),
    ("TILEMODE", B, All),
    ("PLIMCHECK", B, All),
    ("VISRETAIN", B, All),
    ("DELOBJ", B, R13R14),
    ("DISPSILH", B, All),
    ("PELLIPSE", B, All),
    ("PROXYGRAPHICS", Bs, All),
    ("DRAGMODE", Bs, R13R14),
    ("TREEDEPTH", Bs, All),
    ("LUNITS", Bs, All),
    ("LUPREC", Bs, All),
    ("AUNITS", Bs, All),
    ("AUPREC", Bs, All),
    ("OSMODE", Bs, R13R14),
    ("ATTMODE", Bs, All),
    ("COORDS", Bs, R13R14),
    ("PDMODE", Bs, All),
    ("PICKSTYLE", Bs, R13R14),
    ("UNKNOWN_BL_R2004_1", Bl, R2004P),
    ("UNKNOWN_BL_R2004_2", Bl, R2004P),
    ("UNKNOWN_BL_R2004_3", Bl, R2004P),
    ("USERI1", Bs, All),
    ("USERI2", Bs, All),
    ("USERI3", Bs, All),
    ("USERI4", Bs, All),
    ("USERI5", Bs, All),
    ("SPLINESEGS", Bs, All),
    ("SURFU", Bs, All),
    ("SURFV", Bs, All),
    ("SURFTYPE", Bs, All),
    ("SURFTAB1", Bs, All),
    ("SURFTAB2", Bs, All),
    ("SPLINETYPE", Bs, All),
    ("SHADEDGE", Bs, All),
    ("SHADEDIF", Bs, All),
    ("UNITMODE", Bs, All),
    ("MAXACTVP", Bs, All),
    ("ISOLINES", Bs, All),
    ("CMLJUST", Bs, All),
    ("TEXTQLTY", Bs, All),
    ("LTSCALE", Bd, All),
    ("TEXTSIZE", Bd, All),
    ("TRACEWID", Bd, All),
    ("SKETCHINC", Bd, All),
    ("FILLETRAD", Bd, All),
    ("THICKNESS", Bd, All),
    ("ANGBASE", Bd, All),
    ("PDSIZE", Bd, All),
    ("PLINEWID", Bd, All),
    ("USERR1", Bd, All),
    ("USERR2", Bd, All),
    ("USERR3", Bd, All),
    ("USERR4", Bd, All),
    ("USERR5", Bd, All),
    ("CHAMFERA", Bd, All),
    ("CHAMFERB", Bd, All),
    ("CHAMFERC", Bd, All),
    ("CHAMFERD", Bd, All),
    ("FACETRES", Bd, All),
    ("CMLSCALE", Bd, All),
    ("CELTSCALE", Bd, All),
    // The spec lists MENUNAME for R13–R2004 only, but AutoCAD 2027's
    // AC1024, AC1027 and AC1032 files store it too: without it every
    // later string is read one place early (DXFs of the same drawing
    // show $MENU ".", an empty $DIMPOST and the GUIDs where they belong).
    ("MENUNAME", Tv, All),
    ("TDCREATE", Time, All),
    ("TDUPDATE", Time, All),
    ("UNKNOWN_BL_R2004_4", Bl, R2004P),
    ("UNKNOWN_BL_R2004_5", Bl, R2004P),
    ("UNKNOWN_BL_R2004_6", Bl, R2004P),
    ("TDINDWG", Time, All),
    ("TDUSRTIMER", Time, All),
    ("CECOLOR", Cmc, All),
    ("HANDSEED", Seed, All),
    ("CLAYER", H, All),
    ("TEXTSTYLE", H, All),
    ("CELTYPE", H, All),
    ("CMATERIAL", H, R2007P),
    ("DIMSTYLE", H, All),
    ("CMLSTYLE", H, All),
    ("PSVPSCALE", Bd, R2000P),
    ("PINSBASE", Bd3, All),
    ("PEXTMIN", Bd3, All),
    ("PEXTMAX", Bd3, All),
    ("PLIMMIN", Rd2, All),
    ("PLIMMAX", Rd2, All),
    ("PELEVATION", Bd, All),
    ("PUCSORG", Bd3, All),
    ("PUCSXDIR", Bd3, All),
    ("PUCSYDIR", Bd3, All),
    ("PUCSNAME", H, All),
    ("PUCSORTHOREF", H, R2000P),
    ("PUCSORTHOVIEW", Bs, R2000P),
    ("PUCSBASE", H, R2000P),
    ("PUCSORGTOP", Bd3, R2000P),
    ("PUCSORGBOTTOM", Bd3, R2000P),
    ("PUCSORGLEFT", Bd3, R2000P),
    ("PUCSORGRIGHT", Bd3, R2000P),
    ("PUCSORGFRONT", Bd3, R2000P),
    ("PUCSORGBACK", Bd3, R2000P),
    ("INSBASE", Bd3, All),
    ("EXTMIN", Bd3, All),
    ("EXTMAX", Bd3, All),
    ("LIMMIN", Rd2, All),
    ("LIMMAX", Rd2, All),
    ("ELEVATION", Bd, All),
    ("UCSORG", Bd3, All),
    ("UCSXDIR", Bd3, All),
    ("UCSYDIR", Bd3, All),
    ("UCSNAME", H, All),
    ("UCSORTHOREF", H, R2000P),
    ("UCSORTHOVIEW", Bs, R2000P),
    ("UCSBASE", H, R2000P),
    ("UCSORGTOP", Bd3, R2000P),
    ("UCSORGBOTTOM", Bd3, R2000P),
    ("UCSORGLEFT", Bd3, R2000P),
    ("UCSORGRIGHT", Bd3, R2000P),
    ("UCSORGFRONT", Bd3, R2000P),
    ("UCSORGBACK", Bd3, R2000P),
    ("DIMPOST", Tv, R2000P),
    ("DIMAPOST", Tv, R2000P),
    ("DIMTOL", B, R13R14),
    ("DIMLIM", B, R13R14),
    ("DIMTIH", B, R13R14),
    ("DIMTOH", B, R13R14),
    ("DIMSE1", B, R13R14),
    ("DIMSE2", B, R13R14),
    ("DIMALT", B, R13R14),
    ("DIMTOFL", B, R13R14),
    ("DIMSAH", B, R13R14),
    ("DIMTIX", B, R13R14),
    ("DIMSOXD", B, R13R14),
    ("DIMALTD", Rc, R13R14),
    ("DIMZIN", Rc, R13R14),
    ("DIMSD1", B, R13R14),
    ("DIMSD2", B, R13R14),
    ("DIMTOLJ", Rc, R13R14),
    ("DIMJUST", Rc, R13R14),
    ("DIMFIT", Rc, R13R14),
    ("DIMUPT", B, R13R14),
    ("DIMTZIN", Rc, R13R14),
    ("DIMALTZ", Rc, R13R14),
    ("DIMALTTZ", Rc, R13R14),
    ("DIMTAD", Rc, R13R14),
    ("DIMUNIT", Bs, R13R14),
    ("DIMAUNIT", Bs, R13R14),
    ("DIMDEC", Bs, R13R14),
    ("DIMTDEC", Bs, R13R14),
    ("DIMALTU", Bs, R13R14),
    ("DIMALTTD", Bs, R13R14),
    ("DIMTXSTY", H, R13R14),
    ("DIMSCALE", Bd, All),
    ("DIMASZ", Bd, All),
    ("DIMEXO", Bd, All),
    ("DIMDLI", Bd, All),
    ("DIMEXE", Bd, All),
    ("DIMRND", Bd, All),
    ("DIMDLE", Bd, All),
    ("DIMTP", Bd, All),
    ("DIMTM", Bd, All),
    ("DIMFXL", Bd, R2007P),
    ("DIMJOGANG", Bd, R2007P),
    ("DIMTFILL", Bs, R2007P),
    ("DIMTFILLCLR", Cmc, R2007P),
    ("DIMTOL", B, R2000P),
    ("DIMLIM", B, R2000P),
    ("DIMTIH", B, R2000P),
    ("DIMTOH", B, R2000P),
    ("DIMSE1", B, R2000P),
    ("DIMSE2", B, R2000P),
    ("DIMTAD", Bs, R2000P),
    ("DIMZIN", Bs, R2000P),
    ("DIMAZIN", Bs, R2000P),
    ("DIMARCSYM", Bs, R2007P),
    ("DIMTXT", Bd, All),
    ("DIMCEN", Bd, All),
    ("DIMTSZ", Bd, All),
    ("DIMALTF", Bd, All),
    ("DIMLFAC", Bd, All),
    ("DIMTVP", Bd, All),
    ("DIMTFAC", Bd, All),
    ("DIMGAP", Bd, All),
    ("DIMPOST", Tv, R13R14),
    ("DIMAPOST", Tv, R13R14),
    ("DIMBLK", Tv, R13R14),
    ("DIMBLK1", Tv, R13R14),
    ("DIMBLK2", Tv, R13R14),
    ("DIMALTRND", Bd, R2000P),
    ("DIMALT", B, R2000P),
    ("DIMALTD", Bs, R2000P),
    ("DIMTOFL", B, R2000P),
    ("DIMSAH", B, R2000P),
    ("DIMTIX", B, R2000P),
    ("DIMSOXD", B, R2000P),
    ("DIMCLRD", Cmc, All),
    ("DIMCLRE", Cmc, All),
    ("DIMCLRT", Cmc, All),
    ("DIMADEC", Bs, R2000P),
    ("DIMDEC", Bs, R2000P),
    ("DIMTDEC", Bs, R2000P),
    ("DIMALTU", Bs, R2000P),
    ("DIMALTTD", Bs, R2000P),
    ("DIMAUNIT", Bs, R2000P),
    ("DIMFRAC", Bs, R2000P),
    ("DIMLUNIT", Bs, R2000P),
    ("DIMDSEP", Bs, R2000P),
    ("DIMTMOVE", Bs, R2000P),
    ("DIMJUST", Bs, R2000P),
    ("DIMSD1", B, R2000P),
    ("DIMSD2", B, R2000P),
    ("DIMTOLJ", Bs, R2000P),
    ("DIMTZIN", Bs, R2000P),
    ("DIMALTZ", Bs, R2000P),
    ("DIMALTTZ", Bs, R2000P),
    ("DIMUPT", B, R2000P),
    ("DIMATFIT", Bs, R2000P),
    ("DIMFXLON", B, R2007P),
    ("DIMTXTDIRECTION", B, R2010P),
    ("DIMALTMZF", Bd, R2010P),
    ("DIMALTMZS", Tv, R2010P),
    ("DIMMZF", Bd, R2010P),
    ("DIMMZS", Tv, R2010P),
    ("DIMTXSTY", H, R2000P),
    ("DIMLDRBLK", H, R2000P),
    ("DIMBLK", H, R2000P),
    ("DIMBLK1", H, R2000P),
    ("DIMBLK2", H, R2000P),
    ("DIMLTYPE", H, R2007P),
    ("DIMLTEX1", H, R2007P),
    ("DIMLTEX2", H, R2007P),
    ("DIMLWD", Bs, R2000P),
    ("DIMLWE", Bs, R2000P),
    ("BLOCK_CONTROL", H, All),
    ("LAYER_CONTROL", H, All),
    ("STYLE_CONTROL", H, All),
    ("LTYPE_CONTROL", H, All),
    ("VIEW_CONTROL", H, All),
    ("UCS_CONTROL", H, All),
    ("VPORT_CONTROL", H, All),
    ("APPID_CONTROL", H, All),
    ("DIMSTYLE_CONTROL", H, All),
    ("VX_CONTROL", H, R13R2000),
    ("DICTIONARY_ACAD_GROUP", H, All),
    ("DICTIONARY_ACAD_MLINESTYLE", H, All),
    ("DICTIONARY_NAMED_OBJECTS", H, All),
    ("TSTACKALIGN", Bs, R2000P),
    ("TSTACKSIZE", Bs, R2000P),
    ("HYPERLINKBASE", Tv, R2000P),
    ("STYLESHEET", Tv, R2000P),
    ("DICTIONARY_LAYOUTS", H, R2000P),
    ("DICTIONARY_PLOTSETTINGS", H, R2000P),
    ("DICTIONARY_PLOTSTYLES", H, R2000P),
    ("DICTIONARY_MATERIALS", H, R2004P),
    ("DICTIONARY_COLORS", H, R2004P),
    ("DICTIONARY_VISUALSTYLE", H, R2007P),
    ("UNKNOWN_H_R2013", H, R2013P),
    ("FLAGS", Bl, R2000P),
    ("INSUNITS", Bs, R2000P),
    ("CEPSNTYPE", Bs, R2000P),
    ("CPSNID", H, Cpsnid),
    ("FINGERPRINTGUID", Tv, R2000P),
    ("VERSIONGUID", Tv, R2000P),
    ("SORTENTS", Rc, R2004P),
    ("INDEXCTL", Rc, R2004P),
    ("HIDETEXT", Rc, R2004P),
    ("XCLIPFRAME", Rc, R2004P),
    ("DIMASSOC", Rc, R2004P),
    ("HALOGAP", Rc, R2004P),
    ("OBSCUREDCOLOR", Bs, R2004P),
    ("INTERSECTIONCOLOR", Bs, R2004P),
    ("OBSCUREDLTYPE", Rc, R2004P),
    ("INTERSECTIONDISPLAY", Rc, R2004P),
    ("PROJECTNAME", Tv, R2004P),
    ("BLOCK_RECORD_PAPER_SPACE", H, All),
    ("BLOCK_RECORD_MODEL_SPACE", H, All),
    ("LTYPE_BYLAYER", H, All),
    ("LTYPE_BYBLOCK", H, All),
    ("LTYPE_CONTINUOUS", H, All),
    ("CAMERADISPLAY", B, R2007P),
    ("UNKNOWN_BL_R2007_1", Bl, R2007P),
    ("UNKNOWN_BL_R2007_2", Bl, R2007P),
    ("UNKNOWN_BD_R2007_1", Bd, R2007P),
    ("STEPSPERSEC", Bd, R2007P),
    ("STEPSIZE", Bd, R2007P),
    ("3DDWFPREC", Bd, R2007P),
    ("LENSLENGTH", Bd, R2007P),
    ("CAMERAHEIGHT", Bd, R2007P),
    ("SOLIDHIST", Rc, R2007P),
    ("SHOWHIST", Rc, R2007P),
    ("PSOLWIDTH", Bd, R2007P),
    ("PSOLHEIGHT", Bd, R2007P),
    ("LOFTANG1", Bd, R2007P),
    ("LOFTANG2", Bd, R2007P),
    ("LOFTMAG1", Bd, R2007P),
    ("LOFTMAG2", Bd, R2007P),
    ("LOFTPARAM", Bs, R2007P),
    ("LOFTNORMALS", Rc, R2007P),
    ("LATITUDE", Bd, R2007P),
    ("LONGITUDE", Bd, R2007P),
    ("NORTHDIRECTION", Bd, R2007P),
    ("TIMEZONE", Bl, R2007P),
    ("LIGHTGLYPHDISPLAY", Rc, R2007P),
    ("TILEMODELIGHTSYNCH", Rc, R2007P),
    ("DWFFRAME", Rc, R2007P),
    ("DGNFRAME", Rc, R2007P),
    ("UNKNOWN_B_R2007", B, R2007P),
    ("INTERFERECOLOR", Cmc, R2007P),
    ("INTERFEREOBJVS", H, R2007P),
    ("INTERFEREVPVS", H, R2007P),
    ("DRAGVS", H, R2007P),
    ("CSHADOW", Rc, R2007P),
    ("UNKNOWN_BD_R2007_2", Bd, R2007P),
];

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    Bool(bool),
    Int(i64),
    Real(f64),
    P2(Vec2),
    P3(Vec3),
    Text(String),
    Handle(u64),
    Color(crate::read::model::Color),
}

impl Val {
    fn display(&self) -> String {
        match self {
            Val::Bool(b) => b.to_string(),
            Val::Int(i) => i.to_string(),
            Val::Real(f) => format!("{f}"),
            Val::P2(p) => format!("({}, {})", p.x, p.y),
            Val::P3(p) => format!("({}, {}, {})", p.x, p.y, p.z),
            Val::Text(t) => format!("{t:?}"),
            Val::Handle(h) => format!("#{h:X}"),
            Val::Color(c) => color_text(*c),
        }
    }
}

fn read_value(s: &mut Streams, k: K) -> Result<Val> {
    Ok(match k {
        K::B => Val::Bool(s.dat.b()?),
        K::Bs => Val::Int(s.dat.bs()? as i16 as i64),
        K::Bl => Val::Int(s.dat.bl()? as i32 as i64),
        K::Bll => Val::Int(s.dat.bll()? as i64),
        K::Bd => Val::Real(s.dat.bd()?),
        K::Rd2 => Val::P2(Vec2::from_array(s.dat.rd2()?)),
        K::Bd3 => Val::P3(Vec3::from_array(s.dat.bd3()?)),
        K::Rc => Val::Int(s.dat.rc()? as i64),
        K::Tv => Val::Text(s.tv()?),
        K::Cmc => {
            let (color, index) = s.cmc()?;
            if s.ver < V::R2004 {
                Val::Int(index as i64)
            } else {
                // From R2004 the colour is in the method word; the index
                // field alone reads 0 for every colour.
                Val::Color(color)
            }
        }
        K::H => Val::Handle(s.h()?.value),
        K::Seed => Val::Handle(s.dat.handle()?.value),
        K::Time => {
            let day = s.dat.bl()? as i64;
            let ms = s.dat.bl()? as i64;
            Val::Real(day as f64 + ms as f64 / 86_400_000.0)
        }
    })
}

/// A CmColor as the header's display text keeps it: `BYLAYER`,
/// `BYBLOCK`, an ACI number, `RGB(r,g,b)` or `NONE`. Read back by
/// [`crate::read::hvars`].
pub(crate) fn color_text(c: crate::read::model::Color) -> String {
    use crate::read::model::Color;
    match c {
        Color::ByLayer => "BYLAYER".into(),
        Color::ByBlock => "BYBLOCK".into(),
        Color::Index(i) => i.to_string(),
        Color::Rgb(r, g, b) => format!("RGB({r},{g},{b})"),
        Color::None => "NONE".into(),
    }
}

/// The variable list, for writers: name, kind and versions.
pub(crate) fn vars() -> impl Iterator<Item = (&'static str, VarKind, fn(DwgVersion, i64) -> bool)> {
    VARS.iter().map(|&(name, k, w)| {
        let applies_fn: fn(DwgVersion, i64) -> bool = match w {
            W::All => |_, _| true,
            W::R13R14 => |v, _| v <= V::R14,
            W::R13R2000 => |v, _| v <= V::R2000,
            W::R2000P => |v, _| v >= V::R2000,
            W::R2004P => |v, _| v >= V::R2004,
            W::R2007P => |v, _| v >= V::R2007,
            W::R2010P => |v, _| v >= V::R2010,
            W::R2013P => |v, _| v >= V::R2013,
            W::Cpsnid => |v, c| v >= V::R2000 && c == 3,
        };
        let kind = match k {
            K::B => VarKind::B,
            K::Bs => VarKind::Bs,
            K::Bl => VarKind::Bl,
            K::Bll => VarKind::Bll,
            K::Bd => VarKind::Bd,
            K::Rd2 => VarKind::Rd2,
            K::Bd3 => VarKind::Bd3,
            K::Rc => VarKind::Rc,
            K::Tv => VarKind::Tv,
            K::Cmc => VarKind::Cmc,
            K::H => VarKind::H,
            K::Seed => VarKind::Seed,
            K::Time => VarKind::Time,
        };
        (name, kind, applies_fn)
    })
}

/// How a header variable is stored (see the spec, chapter 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VarKind {
    B,
    Bs,
    Bl,
    Bll,
    Bd,
    Rd2,
    Bd3,
    Rc,
    Tv,
    Cmc,
    H,
    Seed,
    Time,
}

/// Read all variables in order.
fn read_all(mut s: Streams) -> Result<(Vec<(&'static str, Val)>, usize)> {
    let mut out = Vec::with_capacity(VARS.len());
    let mut cepsntype = 0i64;
    for &(name, k, w) in VARS {
        if !applies(w, s.ver, cepsntype) {
            continue;
        }
        let v = read_value(&mut s, k).map_err(|e| e.context(format!("header variable {name}")))?;
        if name == "CEPSNTYPE"
            && let Val::Int(i) = v
        {
            cepsntype = i;
        }
        out.push((name, v));
    }
    Ok((out, s.dat.position()))
}

/// Object types the header's control handles must point at.
const EXPECTED_TYPES: &[(&str, u16)] = &[
    ("BLOCK_CONTROL", 0x30),
    ("LAYER_CONTROL", 0x32),
    ("STYLE_CONTROL", 0x34),
    ("LTYPE_CONTROL", 0x38),
    ("VIEW_CONTROL", 0x3C),
    ("UCS_CONTROL", 0x3E),
    ("VPORT_CONTROL", 0x40),
    ("APPID_CONTROL", 0x42),
    ("DIMSTYLE_CONTROL", 0x44),
    ("BLOCK_RECORD_PAPER_SPACE", 0x31),
    ("BLOCK_RECORD_MODEL_SPACE", 0x31),
];

fn validate(vals: &[(&'static str, Val)], type_of: &dyn Fn(u64) -> Option<u16>) -> std::result::Result<(), String> {
    for &(name, expected) in EXPECTED_TYPES {
        let h = vals.iter().find_map(|(n, v)| match v {
            Val::Handle(h) if *n == name => Some(*h),
            _ => None,
        });
        match h {
            None => return Err(format!("{name} not read")),
            Some(h) => match type_of(h) {
                Some(t) if t == expected => {}
                Some(t) => return Err(format!("{name} #{h:X} is type {t:#X}, expected {expected:#X}")),
                None => return Err(format!("{name} #{h:X} not in object map")),
            },
        }
    }
    Ok(())
}

fn to_header(vals: Vec<(&'static str, Val)>, verified: bool, note: String) -> HeaderVars {
    let mut hv = HeaderVars { verified, note, ..Default::default() };
    for (name, v) in &vals {
        match (*name, v) {
            ("INSUNITS", Val::Int(i)) => hv.insunits = u16::try_from(*i).ok(),
            ("EXTMIN", Val::P3(p)) => hv.extmin = Some(*p),
            ("EXTMAX", Val::P3(p)) => hv.extmax = Some(*p),
            ("LIMMIN", Val::P2(p)) => hv.limmin = Some(*p),
            ("LIMMAX", Val::P2(p)) => hv.limmax = Some(*p),
            ("INSBASE", Val::P3(p)) => hv.insbase = Some(*p),
            ("LTSCALE", Val::Real(f)) => hv.ltscale = Some(*f),
            ("TEXTSIZE", Val::Real(f)) => hv.textsize = Some(*f),
            ("CELTSCALE", Val::Real(f)) => hv.celtscale = Some(*f),
            ("LUNITS", Val::Int(i)) => hv.lunits = u16::try_from(*i).ok(),
            ("LUPREC", Val::Int(i)) => hv.luprec = u16::try_from(*i).ok(),
            ("AUNITS", Val::Int(i)) => hv.aunits = u16::try_from(*i).ok(),
            ("AUPREC", Val::Int(i)) => hv.auprec = u16::try_from(*i).ok(),
            ("ANGBASE", Val::Real(f)) => hv.angbase = Some(*f),
            ("ANGDIR", Val::Bool(b)) => hv.angdir = Some(*b as u16),
            ("TEXTSTYLE", Val::Handle(h)) => hv.textstyle = Handle(*h),
            ("CELTYPE", Val::Handle(h)) => hv.celtype = Handle(*h),
            ("DIMSTYLE", Val::Handle(h)) => hv.dimstyle = Handle(*h),
            ("DIMSTYLE_CONTROL", Val::Handle(h)) => hv.dimstyle_control = Handle(*h),
            ("HANDSEED", Val::Handle(h)) => hv.handseed = Some(*h),
            ("CLAYER", Val::Handle(h)) => hv.clayer = Handle(*h),
            ("BLOCK_CONTROL", Val::Handle(h)) => hv.block_control = Handle(*h),
            ("LAYER_CONTROL", Val::Handle(h)) => hv.layer_control = Handle(*h),
            ("STYLE_CONTROL", Val::Handle(h)) => hv.style_control = Handle(*h),
            ("LTYPE_CONTROL", Val::Handle(h)) => hv.linetype_control = Handle(*h),
            ("BLOCK_RECORD_MODEL_SPACE", Val::Handle(h)) => hv.model_space = Handle(*h),
            ("BLOCK_RECORD_PAPER_SPACE", Val::Handle(h)) => hv.paper_space = Handle(*h),
            ("DICTIONARY_NAMED_OBJECTS", Val::Handle(h)) => hv.named_objects_dict = Handle(*h),
            _ => {}
        }
    }
    hv.raw = vals.iter().map(|(n, v)| (n.to_string(), v.display())).collect();
    hv
}

/// Parse the header section. Never fails outright: an unreadable header
/// yields `HeaderVars { verified: false, note }`.
pub(crate) fn parse(data: &[u8], ver: DwgVersion, maintenance: u8, codepage: u16, type_of: &dyn Fn(u64) -> Option<u16>) -> HeaderVars {
    match parse_values(data, ver, maintenance, codepage, Default::default(), type_of) {
        Ok((vals, verified, note)) => to_header(vals, verified, note),
        Err(e) => HeaderVars { verified: false, note: e.to_string(), ..Default::default() },
    }
}

/// The header's variables in file order, whether the reading was verified
/// (its control handles point at objects of the right types), and a note
/// on how it was read.
pub(crate) fn parse_values(
    data: &[u8],
    ver: DwgVersion,
    maintenance: u8,
    codepage: u16,
    text: crate::records::text::TextDecoding,
    type_of: &dyn Fn(u64) -> Option<u16>,
) -> Result<(Vec<(&'static str, Val)>, bool, String)> {
    let wrapped = section::unwrap(data, &HEADER_START, &HEADER_END, ver, maintenance)?;
    let body = wrapped.body;
    if ver < V::R2007 {
        let s = Streams { ver, codepage, dat: BitReader::new(body), strs: None, hdl: None, text };
        let (vals, _end) = read_all(s)?;
        return Ok(match validate(&vals, type_of) {
            Ok(()) => (vals, true, String::new()),
            Err(why) => (vals, false, format!("header not verified: {why}")),
        });
    }
    // R2007+: main, string and handle streams. The layout of the leading
    // bit-size field is not confirmed; try the plausible origins.
    if body.len() < 4 {
        return Err(Error::truncated("header body too short"));
    }
    let bitsize = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;
    let mut notes = Vec::new();
    let mut best: Option<Vec<(&'static str, Val)>> = None;
    for (label, main_start, endbit) in
        [("bitsize from body start", 32usize, bitsize), ("bitsize after field", 32usize, bitsize + 32), ("no bitsize field", 0usize, body.len() * 8)]
    {
        if endbit == 0 || endbit > body.len() * 8 || main_start >= endbit {
            notes.push(format!("{label}: out of range"));
            continue;
        }
        let strs = match string_stream(body, endbit) {
            Ok(Some((sr, _))) => Some(sr),
            Ok(None) => None,
            Err(e) => {
                notes.push(format!("{label}: {e}"));
                continue;
            }
        };
        let hdl = BitReader::with_bounds(body, endbit, body.len() * 8);
        let dat = BitReader::with_bounds(body, main_start, endbit);
        let s = Streams { ver, codepage, dat, strs, hdl: Some(hdl), text };
        match read_all(s) {
            Ok((vals, _)) => match validate(&vals, type_of) {
                Ok(()) => return Ok((vals, true, format!("layout: {label}"))),
                Err(why) => {
                    notes.push(format!("{label}: {why}"));
                    if best.is_none() {
                        best = Some(vals);
                    }
                }
            },
            Err(e) => notes.push(format!("{label}: {e}")),
        }
    }
    let note = format!("header not verified ({})", notes.join("; "));
    match best {
        Some(vals) => Ok((vals, false, note)),
        None => Err(Error::format(note)),
    }
}
