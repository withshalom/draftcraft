//! DWG files read straight into the drawing: each object of the file is decoded into
//! `draftcraft-doc`'s own types, with no other drawing model and no DXF in between.
//!
//! `draftcraft-dwg` opens the file to its parts ([`draftcraft_dwg::file::DwgFile`]: version,
//! sections, the object list); everything about what an object means is decided here, where the
//! drawing is known.
//!
//! The reader is built category by category. Whatever it does not put in the drawing yet is counted
//! in [`Unread`], by type, so nothing is left out silently: a drawing is the whole file only when
//! [`Unread::is_empty`].

mod blocks;
mod carry;
mod entities;
mod header;
mod objects;
mod table;
mod tables;

use std::collections::BTreeMap;

use draftcraft_doc::Drawing;
use draftcraft_dwg::file::DwgFile;

use crate::{IoError, Result};

/// The objects of a file that the reader did not put in the drawing, counted by type name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unread {
    pub by_type: BTreeMap<String, usize>,
    /// How many objects the file has.
    pub objects: usize,
}

impl Unread {
    /// True when the drawing holds everything the file does.
    pub fn is_empty(&self) -> bool {
        self.by_type.is_empty()
    }
    /// How many objects were not read.
    pub fn total(&self) -> usize {
        self.by_type.values().sum()
    }
    /// One line for messages: `LINE × 12, LAYER × 3`.
    pub fn summary(&self) -> String {
        self.by_type.iter().map(|(k, n)| format!("{k} × {n}")).collect::<Vec<_>>().join(", ")
    }
    /// The drawing's read note for what was left out, in the words the app uses for a file read
    /// in part: how many of the file's object records, of which types (the most frequent first),
    /// and what saving over the file would do. `None` when nothing was left out.
    pub fn note(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        /// Most types named in the note.
        const NOTE_TYPES: usize = 6;
        let mut types: Vec<(&String, &usize)> = self.by_type.iter().collect();
        types.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let mut list: Vec<String> = types.iter().take(NOTE_TYPES).map(|(t, n)| format!("{t} × {n}")).collect();
        if types.len() > NOTE_TYPES {
            list.push(format!("{} more types", types.len() - NOTE_TYPES));
        }
        // A key with a space is a part of an object (a property the drawing has no place for
        // yet), or objects of a type that could not be decoded: the others are whole objects.
        let whole: usize = self.by_type.iter().filter(|(k, _)| !k.contains(' ') || k.ends_with(" (not decoded)")).map(|(_, n)| n).sum();
        let what = match (whole, self.objects) {
            (0, _) => "parts of some object records".to_string(),
            (n, 0) => format!("{n} object records"),
            (n, of) => format!("{n} of {of} object records"),
        };
        let (were, them) = if whole == 1 { ("was", "it") } else { ("were", "them") };
        Some(format!("DWG: {what} couldn't be read and {were} left out ({}); saving over the original file would lose {them}", list.join(", ")))
    }
    fn count(&mut self, type_name: &str) {
        *self.by_type.entry(type_name.to_string()).or_default() += 1;
    }
}

/// Read a DWG file into a drawing, with what the reader does not read yet.
/// 8-bit text (up to AutoCAD 2004) by the DXF reader's rules: up to its end, in the code page
/// the file names (Windows-1252 when DraftCraft doesn't know it), with `\U+XXXX` (surrogate
/// pairs too) and `\M+nXXXX` escapes expanded.
fn ansi_text(bytes: &[u8], code_page: u16) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let bytes = bytes.get(..end).unwrap_or(&[]);
    let name = draftcraft_dwg::file::code_page_name(code_page).unwrap_or("ANSI_1252");
    let text = draftcraft_dxf::decode_code_page(bytes, name).or_else(|| draftcraft_dxf::decode_code_page(bytes, "ANSI_1252")).unwrap_or_default();
    draftcraft_dxf::unescape_unicode(&text)
}

/// UTF-16 text (from AutoCAD 2007) by the DXF reader's rules: its escapes expanded.
fn unicode_text(text: String) -> String {
    draftcraft_dxf::unescape_unicode(&text)
}

pub fn read(bytes: &[u8]) -> Result<(Drawing, Unread)> {
    let limit = draftcraft_dwg::Limits::DEFAULT.max_dwg_bytes;
    if bytes.len() > limit {
        return Err(IoError::Format(format!("DWG: the file is larger than the {} MB DraftCraft opens", limit >> 20)));
    }
    let mut file = DwgFile::open(bytes).map_err(|e| IoError::Format(format!("DWG: {e}")))?;
    // Text reads as the DXF reader reads it: in the file's code page, escapes expanded.
    file.set_text_decoding(draftcraft_dwg::file::TextDecoding { ansi: ansi_text, unicode: unicode_text });
    let mut d = Drawing::new_imperial();
    d.header.set_str("ACADVER", file.version_code());
    // A file without the variable (before AutoCAD 2000) has no units.
    d.header.set_i64("INSUNITS", 0);
    let objects: Vec<_> = file.objects().collect();
    let mut unread = Unread { objects: objects.len(), ..Unread::default() };
    let refs = match header::read(&file, &mut d) {
        Ok(refs) => Some(refs),
        Err(_) => {
            unread.count("header");
            None
        }
    };
    let mut tables = tables::read(&file, &objects, &mut d, &mut unread);
    let block_table = blocks::read(&file, &objects, &tables, &mut d, &mut unread);
    let blocks = &block_table.names;
    tables::read_more(&file, &objects, blocks, &mut tables, &mut d, &mut unread);
    // The header variables that name a table record, now that the tables are read. The others
    // (a dimension style, a multiline style, a block, a UCS) wait for their categories.
    for (name, handle) in refs.iter().flat_map(|r| &r.names) {
        let table = match *name {
            "CLAYER" => &tables.layers,
            "CELTYPE" | "DIMLTYPE" | "DIMLTEX1" | "DIMLTEX2" => &tables.linetypes,
            "TEXTSTYLE" | "DIMTXSTY" => &tables.styles,
            "DIMSTYLE" => &tables.dim_styles,
            "UCSNAME" | "PUCSNAME" | "UCSBASE" | "PUCSBASE" | "UCSORTHOREF" | "PUCSORTHOREF" => &tables.ucss,
            "DIMBLK" | "DIMBLK1" | "DIMBLK2" | "DIMLDRBLK" => blocks,
            // A multiline style: named once the objects' names are read, below.
            "CMLSTYLE" => continue,
            _ => {
                unread.count(&format!("header ${name}"));
                continue;
            }
        };
        match table.get(handle) {
            Some(record) => d.header.set_str(name, record),
            // No reference: the variable is empty, as AutoCAD's DXF has it.
            None if *handle == 0 => d.header.set_str(name, ""),
            None => unread.count(&format!("header ${name}")),
        }
    }
    // What entities need from the objects: multileader style names, image and underlay
    // definitions, by the named-object dictionary.
    let root = refs.as_ref().and_then(|r| r.roots.get("DICTIONARY_NAMED_OBJECTS")).copied().unwrap_or(0);
    let named = objects::named(&file, &objects, root);
    // The current multiline style by its key in the ACAD_MLINESTYLE dictionary, as AutoCAD's
    // DXF writes it ("Standard" for the style called STANDARD).
    if let Some(&h) = refs.as_ref().and_then(|r| r.names.get("CMLSTYLE")) {
        let dict = refs.as_ref().and_then(|r| r.roots.get("DICTIONARY_ACAD_MLINESTYLE")).copied().unwrap_or(0);
        match objects::entries_by_handle(&file, &objects, dict).get(&h).or_else(|| named.mline_styles.get(&h)) {
            Some(name) => d.header.set_str("CMLSTYLE", name),
            None if h == 0 => d.header.set_str("CMLSTYLE", ""),
            None => unread.count("header $CMLSTYLE"),
        }
    }
    // Every handle another program's file uses is taken before anything gets a new one. A file
    // DraftCraft wrote names its own next handle (#495): its table records and objects are
    // numbered above it and rebuilt on save, so following the file's handles would drift every
    // save. As the DXF reader has it.
    let ours = objects::entries_by_handle(&file, &objects, root).values().any(|k| k == crate::dxf_ext::HANDSEED_KEY);
    let taken = objects.iter().map(|o| o.handle).max().filter(|_| !ours);
    if let Some(seed) = taken {
        d.bump_handseed(draftcraft_doc::Handle(seed));
    }
    let (entities, links) = entities::read(&file, &objects, &tables, &block_table, &named, &mut d, &mut unread);
    let others = objects::read(&file, &objects, &tables, &named, &mut d, &mut unread);
    let done: std::collections::HashSet<u64> = tables.read.iter().chain(&block_table.read).chain(&entities).chain(&others).copied().collect();
    let carried = carry::read(
        &carry::Source { file: &file, objects: &objects, tables: &tables, blocks: &block_table, named: &named, links: &links, root, done: &done },
        &mut d,
        &mut unread,
    );
    // A block reference's clip, from the kept filter objects, as the DXF reader takes it.
    crate::dxf_clip::read(&mut d);
    for o in &objects {
        if !done.contains(&o.handle) && !carried.contains(&o.handle) {
            unread.count(&o.type_name);
        }
    }
    Ok((d, unread))
}
