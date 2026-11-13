//! A DWG file opened to its parts: the version, the code page and the list of its objects, each
//! with its handle, its type and where its bytes are. This is the level below any drawing model:
//! a reader that builds its own drawing (DraftCraft's, in `draftcraft-io`) starts here and decodes
//! each object straight into that drawing.
//!
//! Reads R13 to AutoCAD 2018 and later.

pub use crate::read::model::{Dictionary, Groups, Handle, Layout, MLineStyle, NonGraphic, Scale, XRecord, XrValue};
pub use crate::records::text::{TextDecoding, code_page_name};
use std::collections::HashMap;

use crate::read::header::Val;
use crate::read::model::{ClassDef, Color, Diagnostic, LinetypeRef, Space, XDataApp, XValue};
use crate::read::{self, object};
use crate::records::Result;

/// One object of the file, before anything of its content is decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRef {
    /// The object's handle, from the object map.
    pub handle: u64,
    /// Where the object starts in the object data.
    pub offset: usize,
    /// The DWG type number (0x13 for LINE; 500 and up for class-defined types).
    pub type_code: u16,
    /// The DXF name of the type: `LINE`, `LAYER`, or the class's name. `?` when the object's
    /// first fields can't be read.
    pub type_name: String,
    /// True for entities (objects that are drawn).
    pub is_entity: bool,
}

/// A colour as DWG stores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorValue {
    ByLayer,
    ByBlock,
    /// An AutoCAD Color Index number, 1 to 255.
    Index(u8),
    /// A true colour.
    Rgb(u8, u8, u8),
    /// No colour.
    None,
}

/// A colour as a layer or a DBCOLOR object stores it, with the names of a colour book colour.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedColor {
    pub color: ColorValue,
    /// The index field as stored (its sign marks a layer that is off, before AutoCAD 2000).
    pub index: i16,
    /// The colour's name in its book (`RAL 1006`).
    pub name: Option<String>,
    /// The colour book's name (`RAL CLASSIC`).
    pub book: Option<String>,
}

/// A class of the file's objects or entities, as a DXF CLASS record has it (groups 1, 2, 3, 90,
/// 91, 280, 281).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassInfo {
    pub dxf_name: String,
    pub cpp_name: String,
    pub app_name: String,
    pub proxy_flags: u16,
    pub instance_count: u32,
    pub was_zombie: bool,
    pub is_entity: bool,
}

/// The value of one header variable, as the file stores it.
#[derive(Clone, Debug, PartialEq)]
pub enum HeaderValue {
    Bool(bool),
    Int(i64),
    Real(f64),
    Point2([f64; 2]),
    Point3([f64; 3]),
    Text(String),
    /// A reference to another object of the file (the current layer, a table's control object).
    Handle(u64),
    Color(ColorValue),
}

/// The header variables of a file.
#[derive(Clone, Debug, PartialEq)]
pub struct HeaderRead {
    /// Every variable in file order, by its DWG name. A file of an older version has fewer.
    pub vars: Vec<(&'static str, HeaderValue)>,
    /// True when the handles the header names point at objects of the right types, which shows
    /// the variables were read at their right places.
    pub verified: bool,
    /// MEASUREMENT (0 imperial, 1 metric), which DWG keeps outside the header section.
    pub measurement: Option<u16>,
    /// Who last saved the file, from its summary section (AutoCAD 2004 and later).
    pub last_saved_by: Option<String>,
}

fn color_value(c: Color) -> ColorValue {
    match c {
        Color::ByLayer => ColorValue::ByLayer,
        Color::ByBlock => ColorValue::ByBlock,
        Color::Index(i) => ColorValue::Index(i),
        Color::Rgb(r, g, b) => ColorValue::Rgb(r, g, b),
        Color::None => ColorValue::None,
    }
}

/// One value of an object's extended data (DXF groups 1000 to 1071).
#[derive(Clone, Debug, PartialEq)]
pub enum XDataItem {
    /// 1000: a string.
    Text(String),
    /// 1002 `{`.
    Open,
    /// 1002 `}`.
    Close,
    /// 1003: a layer, by handle.
    Layer(u64),
    /// 1004: bytes.
    Bytes(Vec<u8>),
    /// 1005: an object, by handle.
    Handle(u64),
    /// 1010: a point.
    Point([f64; 3]),
    /// 1011: a position in space.
    Position([f64; 3]),
    /// 1012: a displacement.
    Displacement([f64; 3]),
    /// 1013: a direction.
    Direction([f64; 3]),
    /// 1040: a number.
    Real(f64),
    /// 1041: a distance.
    Distance(f64),
    /// 1042: a scale factor.
    ScaleFactor(f64),
    /// 1070: a 16-bit integer.
    Int16(i16),
    /// 1071: a 32-bit integer.
    Int32(i32),
    /// A value the reader could not decode, as its bytes.
    Undecoded(Vec<u8>),
}

/// The extended data one application attached to an object.
#[derive(Clone, Debug, PartialEq)]
pub struct XDataBlock {
    /// The application, by the handle of its APPID table record.
    pub app: u64,
    pub items: Vec<XDataItem>,
}

/// The references every object starts with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommonRefs {
    /// The object that owns this one (its table, its block, its dictionary).
    pub owner: u64,
    /// The objects told when this one changes.
    pub reactors: Vec<u64>,
    /// The object's extension dictionary, 0 when it has none.
    pub xdictionary: u64,
}

/// Where an entity lives, as the entity itself says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntitySpace {
    Model,
    Paper,
    /// In the block that owns it ([`EntityRefs::owner`]).
    Owned,
}

/// An entity's linetype.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityLinetype {
    ByLayer,
    ByBlock,
    Continuous,
    /// A linetype table record, by handle.
    Record(u64),
}

/// The properties every entity has, as stored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityHead {
    pub space: EntitySpace,
    pub color: ColorValue,
    /// The transparency word (DXF group 440), when the entity has one.
    pub transparency: Option<u32>,
    pub linetype_scale: f64,
    /// The lineweight, as the index DWG stores (29 ByLayer, 30 ByBlock, 31 Default).
    pub lineweight: u8,
    pub invisible: bool,
}

/// The references every entity starts with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityRefs {
    /// The block record that owns the entity, 0 when [`EntityHead::space`] says model or paper.
    pub owner: u64,
    pub reactors: Vec<u64>,
    pub xdictionary: u64,
    pub layer: u64,
    pub linetype: EntityLinetype,
    pub material: u64,
    pub plotstyle: u64,
    /// The DBCOLOR object of a colour book colour (AutoCAD 2004 on), whose true colour and names
    /// the entity takes ([`ObjectReader::named_color`] reads it); 0 for none.
    pub color_book: u64,
    /// The next entity of the block's chain when the entity stores it (up to AutoCAD 2000);
    /// `None` when the next object of the file follows, or from AutoCAD 2004 on, where the block
    /// lists its entities.
    pub next: Option<u64>,
}

/// One object opened for reading: its fields are read in the order the format stores them, by
/// the reader that knows the object's type. Every read is bounds-checked and returns an error at
/// the end of the object's data.
pub struct ObjectReader<'a> {
    o: object::Opened<'a>,
}

impl ObjectReader<'_> {
    /// The file's version, which decides what many objects store.
    pub fn version(&self) -> read::DwgVersion {
        self.o.s.ver
    }
    /// True when the file keeps text apart from the other fields (AutoCAD 2007 and later).
    pub fn has_string_stream(&self) -> bool {
        self.o.s.ver.has_string_stream()
    }
    /// The object's own handle.
    pub fn handle(&self) -> u64 {
        self.o.handle
    }
    /// The object's extended data, one block per application.
    pub fn xdata(&self) -> Vec<XDataBlock> {
        self.o
            .xdata
            .iter()
            .filter_map(|x| {
                let XDataApp::Handle(app) = &x.app else { return None };
                let p = |v: &crate::read::geom::Vec3| [v.x, v.y, v.z];
                let items = x
                    .items
                    .iter()
                    .map(|v| match v {
                        XValue::Str(s) => XDataItem::Text(s.clone()),
                        XValue::Open => XDataItem::Open,
                        XValue::Close => XDataItem::Close,
                        XValue::Layer(h) => XDataItem::Layer(h.0),
                        XValue::Bytes(b) => XDataItem::Bytes(b.clone()),
                        XValue::Handle(h) => XDataItem::Handle(h.0),
                        XValue::Point(v) => XDataItem::Point(p(v)),
                        XValue::Position(v) => XDataItem::Position(p(v)),
                        XValue::Displacement(v) => XDataItem::Displacement(p(v)),
                        XValue::Direction(v) => XDataItem::Direction(p(v)),
                        XValue::Real(r) => XDataItem::Real(*r),
                        XValue::Distance(r) => XDataItem::Distance(*r),
                        XValue::ScaleFactor(r) => XDataItem::ScaleFactor(*r),
                        XValue::Int16(i) => XDataItem::Int16(*i),
                        XValue::Int32(i) => XDataItem::Int32(*i),
                        XValue::Undecoded { data, .. } => XDataItem::Undecoded(data.clone()),
                    })
                    .collect();
                Some(XDataBlock { app: app.0, items })
            })
            .collect()
    }
    /// The properties every entity has; `None` for an object that is not an entity.
    pub fn entity(&self) -> Option<EntityHead> {
        let c = self.o.common.as_ref()?;
        Some(EntityHead {
            space: match c.space {
                Space::Model => EntitySpace::Model,
                Space::Paper => EntitySpace::Paper,
                Space::Owned => EntitySpace::Owned,
            },
            color: color_value(c.color),
            transparency: c.transparency,
            linetype_scale: c.linetype_scale,
            lineweight: c.lineweight,
            invisible: c.invisible,
        })
    }
    /// An entity's owner, reactors, extension dictionary, layer, linetype, material and plot
    /// style. They come first among its references, so this is called before any
    /// [`ObjectReader::reference`].
    pub fn entity_refs(&mut self) -> Result<EntityRefs> {
        let (owner, reactors, xdictionary) = object::common_handles(&mut self.o)?;
        let c = self.o.common.as_ref().ok_or_else(|| crate::records::Error::format("not an entity"))?;
        Ok(EntityRefs {
            owner,
            reactors: reactors.into_iter().map(|h| h.0).collect(),
            xdictionary,
            layer: c.layer.0,
            linetype: match c.linetype {
                LinetypeRef::ByLayer => EntityLinetype::ByLayer,
                LinetypeRef::ByBlock => EntityLinetype::ByBlock,
                LinetypeRef::Continuous => EntityLinetype::Continuous,
                LinetypeRef::Handle(h) => EntityLinetype::Record(h.0),
            },
            material: c.material.0,
            plotstyle: c.plotstyle.0,
            color_book: c.color_book.0,
            next: self.o.next_entity,
        })
    }
    /// The object as the codec keeps a non-graphical object, when its type is one it decodes:
    /// dictionaries, extension records, placeholders, dictionary variables, scales, layouts and
    /// multiline styles, and the style objects it holds as their DXF groups (a table style names
    /// its text style by `text_styles`: name, handle). With the references every object starts
    /// with. `None` for another type, or a style object of a version its codec does not read.
    /// Reads the whole object: nothing else is read of it after this.
    pub fn non_graphic(&mut self, text_styles: &[(String, u64)]) -> Option<Result<(NonGraphic, CommonRefs)>> {
        let lookup = crate::read::styles::Lookup::from_text_styles(text_styles.iter().map(|(n, h)| (n.clone(), Handle(*h))).collect());
        crate::read::nongraphic::decode_opened(&mut self.o, &lookup, true).map(|r| {
            r.map(|(n, (owner, reactors, xdictionary))| (n, CommonRefs { owner, reactors: reactors.into_iter().map(|h| h.0).collect(), xdictionary }))
        })
    }
    /// The owner, reactors and extension dictionary. They come first among the object's
    /// references, so this is called before any [`ObjectReader::reference`].
    pub fn common_refs(&mut self) -> Result<CommonRefs> {
        let (owner, reactors, xdictionary) = object::common_handles(&mut self.o)?;
        Ok(CommonRefs { owner, reactors: reactors.into_iter().map(|h| h.0).collect(), xdictionary })
    }
    /// The next reference to another object, as that object's handle (0 for none).
    pub fn reference(&mut self) -> Result<u64> {
        let own = self.o.handle;
        self.o.s.h_abs(own)
    }
    /// A bit.
    pub fn b(&mut self) -> Result<bool> {
        self.o.s.dat.b()
    }
    /// Two bits.
    pub fn bb(&mut self) -> Result<u8> {
        self.o.s.dat.bb()
    }
    /// A bit-coded 16-bit integer.
    pub fn bs(&mut self) -> Result<u16> {
        self.o.s.dat.bs()
    }
    /// A bit-coded 32-bit integer.
    pub fn bl(&mut self) -> Result<u32> {
        self.o.s.dat.bl()
    }
    /// A bit-coded double.
    pub fn bd(&mut self) -> Result<f64> {
        self.o.s.dat.bd()
    }
    /// Two bit-coded doubles.
    pub fn bd2(&mut self) -> Result<[f64; 2]> {
        self.o.s.dat.bd2()
    }
    /// Three bit-coded doubles.
    pub fn bd3(&mut self) -> Result<[f64; 3]> {
        self.o.s.dat.bd3()
    }
    /// A double stored against a default (the previous coordinate, or 1).
    pub fn dd(&mut self, default: f64) -> Result<f64> {
        self.o.s.dat.dd(default)
    }
    /// A thickness.
    pub fn thickness(&mut self) -> Result<f64> {
        let v = self.o.s.ver;
        self.o.s.dat.bt(v)
    }
    /// An extrusion direction.
    pub fn extrusion(&mut self) -> Result<[f64; 3]> {
        let v = self.o.s.ver;
        self.o.s.dat.be(v)
    }
    /// A count of items that each take at least `min_bits` bits, refused when the object's
    /// data is too short to hold that many.
    pub fn count(&mut self, n: u32, min_bits: usize, what: &str) -> Result<usize> {
        self.o.s.check_count(n, min_bits, what)
    }
    /// A byte.
    pub fn rc(&mut self) -> Result<u8> {
        self.o.s.dat.rc()
    }
    /// A raw double.
    pub fn rd(&mut self) -> Result<f64> {
        self.o.s.dat.rd()
    }
    /// Two raw doubles.
    pub fn rd2(&mut self) -> Result<[f64; 2]> {
        self.o.s.dat.rd2()
    }
    /// `n` bytes.
    pub fn bytes(&mut self, n: usize) -> Result<Vec<u8>> {
        self.o.s.dat.raw_bytes(n)
    }
    /// Skip `n` bytes.
    pub fn skip_bytes(&mut self, n: usize) -> Result<()> {
        self.o.s.dat.skip_bits(n.saturating_mul(8))
    }
    /// A text value, from the string stream where the file has one.
    pub fn text(&mut self) -> Result<String> {
        self.o.s.tv()
    }
    /// A colour, with the index field as stored (its sign marks a layer that is off, before
    /// AutoCAD 2000).
    pub fn color(&mut self) -> Result<(ColorValue, i16)> {
        let (c, index) = self.o.s.cmc()?;
        Ok((color_value(c), index))
    }
    /// A colour with the colour book names it carries (AutoCAD 2004 on), as a layer or a DBCOLOR
    /// object stores it.
    pub fn named_color(&mut self) -> Result<NamedColor> {
        let (c, index, name, book) = self.o.s.cmc_named()?;
        Ok(NamedColor { color: color_value(c), index, name, book })
    }
    /// 8-bit text in the file's code page, as a string.
    pub fn ansi(&self, bytes: &[u8]) -> String {
        (self.o.s.text.ansi)(bytes, self.o.s.codepage)
    }
    /// How many bits of the object's fields are left to read.
    pub fn remaining_bits(&self) -> usize {
        self.o.s.dat.remaining()
    }
}

/// A DWG file opened to its sections.
pub struct DwgFile {
    version: read::DwgVersion,
    /// The maintenance release the file records.
    pub maintenance: u8,
    /// The code page of the file's 8-bit text.
    pub codepage: u16,
    /// How the file's text becomes strings ([`DwgFile::set_text_decoding`]).
    text: crate::records::text::TextDecoding,
    classes: Vec<ClassDef>,
    header: Option<Vec<u8>>,
    others: Vec<(String, Vec<u8>)>,
    objects: Vec<u8>,
    entries: Vec<(u64, usize)>,
    /// What opening the file had to say (sections skipped, checksums that failed).
    pub notes: Vec<String>,
}

impl DwgFile {
    /// Open a DWG file's container and read its class list and object map.
    pub fn open(bytes: &[u8]) -> Result<DwgFile> {
        let (version, c) = read::open_container(bytes)?;
        let mut diagnostics: Vec<Diagnostic> = c.notes;
        let classes = match &c.classes {
            Some(data) => match read::classes::parse(data, version, c.maintenance, c.codepage) {
                Ok((list, _)) => list,
                Err(e) => {
                    diagnostics.push(Diagnostic { severity: read::model::Severity::Warning, message: format!("classes: {e}") });
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        let map = read::choose_object_map(&c.handles, version, &c.objects, &mut diagnostics)?;
        let len = c.objects.len();
        let entries = map
            .iter()
            .filter(|e| e.handle != 0)
            .filter_map(|e| usize::try_from(e.location).ok().filter(|&o| o < len).map(|o| (e.handle, o)))
            .collect();
        Ok(DwgFile {
            version,
            maintenance: c.maintenance,
            codepage: c.codepage,
            text: crate::records::text::TextDecoding::default(),
            classes,
            header: c.header,
            others: c.others,
            objects: c.objects,
            entries,
            notes: diagnostics.into_iter().map(|d| d.message).collect(),
        })
    }

    /// The file's version code, e.g. `AC1032`.
    pub fn version_code(&self) -> &'static str {
        self.version.magic()
    }

    /// The DXF name of the file's code page (`ANSI_1252`, `ANSI_932`…), when the number it
    /// stores names one.
    pub fn codepage_name(&self) -> Option<&'static str> {
        crate::records::text::code_page_name(self.codepage)
    }

    /// How the file's text becomes strings from now on: every object, header and extended data
    /// string read after this goes through `text` (see [`TextDecoding`]).
    pub fn set_text_decoding(&mut self, text: TextDecoding) {
        self.text = text;
    }

    /// The class a type of the file belongs to, by its DXF name: what a DXF CLASS record says of
    /// it. `None` for a built-in type, or one the file defines no class for.
    pub fn class_of(&self, dxf_name: &str) -> Option<ClassInfo> {
        self.classes.iter().find(|c| c.dxf_name.eq_ignore_ascii_case(dxf_name)).map(|c| ClassInfo {
            dxf_name: c.dxf_name.clone(),
            cpp_name: c.cpp_name.clone(),
            app_name: c.app_name.clone(),
            proxy_flags: c.proxy_flags,
            instance_count: c.instance_count,
            was_zombie: c.was_zombie,
            is_entity: c.item_class_id == 0x1F2,
        })
    }

    /// True for the versions before AutoCAD 2000 (R13 and R14).
    pub fn before_2000(&self) -> bool {
        self.version < read::DwgVersion::R2000
    }

    /// How many objects the object map lists.
    pub fn object_count(&self) -> usize {
        self.entries.len()
    }

    /// The header variables. An error when the file has no header section or it can't be read.
    pub fn header(&self) -> Result<HeaderRead> {
        let data = self.header.as_ref().ok_or_else(|| crate::records::Error::format("no header section"))?;
        let by_handle: HashMap<u64, usize> = self.entries.iter().copied().collect();
        let type_of = |h: u64| by_handle.get(&h).and_then(|&o| object::peek(self.version, &self.objects, o).ok()).map(|(_, code)| code);
        let (vals, verified, _note) = read::header::parse_values(data, self.version, self.maintenance, self.codepage, self.text, &type_of)?;
        let vars = vals
            .into_iter()
            .map(|(name, v)| {
                let v = match v {
                    Val::Bool(b) => HeaderValue::Bool(b),
                    Val::Int(i) => HeaderValue::Int(i),
                    Val::Real(r) => HeaderValue::Real(r),
                    Val::P2(p) => HeaderValue::Point2([p.x, p.y]),
                    Val::P3(p) => HeaderValue::Point3([p.x, p.y, p.z]),
                    Val::Text(t) => HeaderValue::Text(t),
                    Val::Handle(h) => HeaderValue::Handle(h),
                    Val::Color(c) => HeaderValue::Color(color_value(c)),
                };
                (name, v)
            })
            .collect();
        let measurement = self.others.iter().find(|(n, _)| n == "AcDb:Template").and_then(|(_, s)| read::template_measurement(s));
        let wide = self.version >= read::DwgVersion::R2007;
        let last_saved_by =
            self.others.iter().find(|(n, _)| n == "AcDb:SummaryInfo").and_then(|(_, s)| read::summary_info(s, wide)).map(|p| p.last_saved_by);
        Ok(HeaderRead { vars, verified, measurement, last_saved_by })
    }

    /// Open one object to read its fields.
    pub fn read_object(&self, object: &ObjectRef) -> Result<ObjectReader<'_>> {
        let ctx = object::DecodeCtx { ver: self.version, codepage: self.codepage, classes: &self.classes, text: self.text };
        Ok(ObjectReader { o: object::open(&ctx, &self.objects, object.offset)? })
    }

    /// Every object of the file, in handle order as the object map lists them.
    pub fn objects(&self) -> impl Iterator<Item = ObjectRef> + '_ {
        self.entries.iter().map(|&(handle, offset)| match object::peek(self.version, &self.objects, offset) {
            Ok((_, type_code)) => {
                let (type_name, is_entity) = object::classify(type_code, &self.classes);
                ObjectRef { handle, offset, type_code, type_name, is_entity }
            }
            Err(_) => ObjectRef { handle, offset, type_code: 0, type_name: "?".into(), is_entity: false },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_a_dwg_is_an_error() {
        assert!(DwgFile::open(b"hello").is_err());
        assert!(DwgFile::open(b"").is_err());
        assert!(DwgFile::open(b"AC1032").is_err());
    }
}
