//! The header variables of a DWG file, into the drawing's header.
//!
//! The drawing keeps each variable under its DXF name with its DXF meaning, so the few that DWG
//! stores differently are converted here: the flags packed into one word, the variables DWG names
//! differently, and the two whose unit differs.

use std::collections::BTreeMap;

use draftcraft_doc::{Drawing, HVal};
use draftcraft_dwg::file::{ColorValue, DwgFile, HeaderValue};
use draftcraft_geom::Vec3;

/// The header's references to other objects of the file, by DWG variable name.
pub(super) struct HeaderRefs {
    /// Variables that hold a name in the drawing (the current layer, the current text style):
    /// they are set once the table they point into is read.
    pub names: BTreeMap<&'static str, u64>,
    /// Where the file's tables and dictionaries start (`LAYER_CONTROL`, `DICTIONARY_LAYOUTS`):
    /// not variables of the drawing, but the roots the later categories are read from.
    pub roots: BTreeMap<&'static str, u64>,
}

/// Lineweights in hundredths of a millimetre, by the index DWG stores.
const LINEWEIGHTS: [i64; 24] = [0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140, 158, 200, 211];

/// A lineweight as the header holds it: hundredths of a millimetre, or -1 ByLayer, -2 ByBlock,
/// -3 Default.
pub(super) fn lineweight(index: i64) -> i64 {
    match index {
        29 => -1,
        30 => -2,
        31 => -3,
        i => usize::try_from(i).ok().and_then(|i| LINEWEIGHTS.get(i)).copied().unwrap_or(-1),
    }
}

/// True for the handles that are a root of the file's structure, not a variable.
fn root(name: &str) -> bool {
    name.ends_with("_CONTROL")
        || name.starts_with("DICTIONARY_")
        || name.starts_with("BLOCK_RECORD_")
        || name.starts_with("LTYPE_")
        || name == "CURRENT_VIEWPORT_ENTITY_HEADER"
}

/// Read the header variables into `d.header`. Returns the header's references to other objects,
/// or what went wrong.
pub(super) fn read(file: &DwgFile, d: &mut Drawing) -> Result<HeaderRefs, String> {
    let h = file.header().map_err(|e| e.to_string())?;
    let mut refs = HeaderRefs { names: BTreeMap::new(), roots: BTreeMap::new() };
    d.header.set_i64("ACADMAINTVER", i64::from(file.maintenance));
    if let Some(cp) = file.codepage_name() {
        d.header.set_str("DWGCODEPAGE", cp);
    }
    for (name, v) in h.vars {
        match (name, v) {
            // The handle a new object gets: a number, which DXF writes in hexadecimal.
            ("HANDSEED", HeaderValue::Handle(seed)) => d.header.set_str("HANDSEED", &format!("{seed:X}")),
            // References DXF writes as the handle itself. A null one is no variable.
            ("CMATERIAL" | "INTERFEREOBJVS" | "INTERFEREVPVS" | "DRAGVS", HeaderValue::Handle(x)) => {
                if x != 0 {
                    d.header.set_str(name, &format!("{x:X}"));
                }
            }
            // The handle AutoCAD 2013 added after the visual-style dictionary: no DXF variable,
            // and null in AutoCAD's own files.
            (n, HeaderValue::Handle(_)) if n.starts_with("UNKNOWN_") => {}
            (_, HeaderValue::Handle(x)) => {
                if root(name) {
                    refs.roots.insert(name, x);
                } else {
                    // Not known until its table is read: the new drawing's default must not
                    // stand in for it.
                    d.header.vars.remove(name);
                    refs.names.insert(name, x);
                }
            }
            // One word holds eight variables (R2000 and later).
            ("FLAGS", HeaderValue::Int(f)) => {
                d.header.set_i64("CELWEIGHT", lineweight(f & 0x1F));
                d.header.set_i64("ENDCAPS", (f >> 5) & 3);
                d.header.set_i64("JOINSTYLE", (f >> 7) & 3);
                d.header.set_i64("LWDISPLAY", i64::from(f & 0x200 == 0));
                d.header.set_i64("XEDIT", i64::from(f & 0x400 == 0));
                d.header.set_i64("EXTNAMES", i64::from(f & 0x800 != 0));
                d.header.set_i64("PSTYLEMODE", i64::from(f & 0x2000 != 0));
                d.header.set_i64("OLESTARTUP", i64::from(f & 0x4000 != 0));
            }
            // DWG's names for three DXF variables.
            ("MENUNAME", HeaderValue::Text(t)) => d.header.set_str("MENU", &t),
            ("OBSCUREDCOLOR", HeaderValue::Int(i)) => d.header.set_i64("OBSCOLOR", i),
            ("OBSCUREDLTYPE", HeaderValue::Int(i)) => d.header.set_i64("OBSLTYPE", i),
            // The two fields after DGNFRAME and CSHADOW that the specification leaves unnamed,
            // by their place in the DXF header's order and their values in AutoCAD's files.
            ("UNKNOWN_B_R2007", HeaderValue::Bool(b)) => d.header.set_i64("REALWORLDSCALE", i64::from(b)),
            ("UNKNOWN_BD_R2007_2", HeaderValue::Real(r)) => d.header.set_f64("SHADOWPLANELOCATION", r),
            // The other fields the specification gives no name: nothing a drawing uses.
            (n, _) if n.starts_with("UNKNOWN_") => {}
            // From AutoCAD 2000 on the two dates are stored in universal time, which DXF calls
            // TDUCREATE and TDUUPDATE. The local-time pair is not in the file.
            ("TDCREATE", HeaderValue::Real(t)) if !file.before_2000() => d.header.set_f64("TDUCREATE", t),
            ("TDUPDATE", HeaderValue::Real(t)) if !file.before_2000() => d.header.set_f64("TDUUPDATE", t),
            // The decimal separator is a character; DWG stores its code.
            ("DIMDSEP", HeaderValue::Int(c)) => {
                if let Some(c) = u32::try_from(c).ok().and_then(char::from_u32).filter(|c| !c.is_control()) {
                    d.header.set_str("DIMDSEP", &c.to_string());
                }
            }
            // DWG stores the jog angle in radians; the variable holds degrees.
            ("DIMJOGANG", HeaderValue::Real(r)) => d.header.set_f64("DIMJOGANG", (r.to_degrees() * 1e9).round() / 1e9),
            (_, HeaderValue::Bool(b)) => d.header.set_i64(name, i64::from(b)),
            (_, HeaderValue::Int(i)) => d.header.set_i64(name, i),
            (_, HeaderValue::Real(r)) => d.header.set_f64(name, r),
            (_, HeaderValue::Point2([x, y])) => d.header.set(name, HVal::Point(Vec3::new(x, y, 0.0))),
            (_, HeaderValue::Point3([x, y, z])) => d.header.set(name, HVal::Point(Vec3::new(x, y, z))),
            (_, HeaderValue::Text(t)) => d.header.set_str(name, &t),
            // A colour variable holds an AutoCAD Color Index number: 0 ByBlock, 256 ByLayer.
            (_, HeaderValue::Color(c)) => d.header.set_i64(
                name,
                match c {
                    ColorValue::ByBlock => 0,
                    ColorValue::ByLayer | ColorValue::None => 256,
                    ColorValue::Index(i) => i64::from(i),
                    ColorValue::Rgb(r, g, b) => i64::from(draftcraft_color::nearest_aci(draftcraft_color::Rgb::from_u32(
                        u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
                    ))),
                },
            ),
        }
    }
    if let Some(who) = h.last_saved_by.filter(|w| !w.is_empty()) {
        d.header.set_str("LASTSAVEDBY", &who);
    }
    if let Some(m) = h.measurement {
        d.header.set_i64("MEASUREMENT", i64::from(m));
    }
    Ok(refs)
}
