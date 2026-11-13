//! ACAD_TABLE (§20.4.98, and from AutoCAD 2010 on the table content the entity embeds): the
//! block reference it is drawn as, then its columns, rows and cells. The drawing keeps the
//! sizes, each cell's text, merges and text height; DraftCraft's own extended data (title and
//! header rows, styles, formulas, breaks) is read by the DXF reader's rule.
//!
//! The table's references sit between its fields (a pre-2010 cell's text is there only when its
//! value names no object), so this reads both streams together: the entity's common references
//! are read first.

use draftcraft_doc::TableCell;
use draftcraft_dwg::DwgVersion as V;
use draftcraft_dwg::file::{ColorValue, ObjectReader};
use draftcraft_dwg::records::{Error, Result as DwgResult};
use draftcraft_dxf::Tag;

/// Most rows, columns, cells, contents, borders or list items of one table that are read.
const MAX_ITEMS: u32 = 1_000_000;

/// A table's cells as read, before DraftCraft's extended data is applied.
pub(super) struct TableRead {
    pub col_widths: Vec<f64>,
    pub row_heights: Vec<f64>,
    pub cells: Vec<Vec<TableCell>>,
    /// The TABLESTYLE the table names.
    pub style: u64,
    /// What of the table the drawing has no place for.
    pub partial: Vec<&'static str>,
}

fn bounded(r: &mut ObjectReader, n: u32, min_bits: usize, what: &str) -> DwgResult<usize> {
    if n > MAX_ITEMS {
        return Err(Error::format(format!("too many {what}s: {n}")));
    }
    r.count(n, min_bits, what)
}

/// A colour with its name and book (a "true colour" record, whatever the version).
fn true_color(r: &mut ObjectReader) -> DwgResult<ColorValue> {
    let _index = r.bs()?;
    let rgb = r.bl()?;
    let flags = r.rc()?;
    if flags & 1 != 0 {
        let _name = r.text()?;
    }
    if flags & 2 != 0 {
        let _book = r.text()?;
    }
    Ok(match rgb >> 24 {
        0xC0 => ColorValue::ByLayer,
        0xC1 => ColorValue::ByBlock,
        0xC2 => ColorValue::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8),
        0xC3 => ColorValue::Index(rgb as u8),
        _ => ColorValue::None,
    })
}

/// A text a cell value holds: a byte count, then the bytes (UTF-16 with its terminator from
/// AutoCAD 2007 on, the file's code page before).
fn value_text(r: &mut ObjectReader) -> DwgResult<String> {
    let n = r.bl()?;
    let n = bounded(r, n, 8, "value byte")?;
    let bytes = r.bytes(n)?;
    if r.version() >= V::R2007 {
        let units: Vec<u16> = bytes
            .get(..n.saturating_sub(2))
            .unwrap_or(&[])
            .chunks(2)
            .map(|c| u16::from_le_bytes([c.first().copied().unwrap_or(0), c.get(1).copied().unwrap_or(0)]))
            .collect();
        Ok(String::from_utf16_lossy(&units))
    } else {
        let end = if bytes.last() == Some(&0) { n.saturating_sub(1) } else { n };
        Ok(r.ansi(bytes.get(..end).unwrap_or(&[])))
    }
}

/// A cell value: its type and data, then from AutoCAD 2007 on its unit, format and the value as
/// shown. Returns the text it shows: a string's, else the formatted value.
fn value(r: &mut ObjectReader, modern: bool) -> DwgResult<String> {
    let since_2007 = r.version() >= V::R2007;
    let flags = if since_2007 { r.bl()? } else { 0 };
    let code = r.bl()?;
    let kind = if modern { code } else { code & !0x200 };
    let mut text = None;
    if !since_2007 || flags & 1 == 0 {
        match kind {
            0 | 1 => {
                let _long = r.bl()?;
            }
            2 => {
                let _double = r.bd()?;
            }
            4 | 0x200 => text = Some(value_text(r)?),
            8 => {
                let n = r.bl()?;
                let n = bounded(r, n, 8, "date byte")?;
                r.skip_bytes(n)?;
            }
            0x10 => {
                if r.bl()? > 0 {
                    let _point = r.rd2()?;
                }
            }
            0x20 => {
                if r.bl()? > 0 {
                    let _point = (r.rd()?, r.rd()?, r.rd()?);
                }
            }
            0x40 => {
                let _object = r.reference()?;
            }
            _ => {}
        }
    }
    if since_2007 {
        let unit = r.bl()?;
        let _format = r.text()?;
        if unit != 12 {
            let formatted = r.text()?;
            if text.is_none() {
                text = Some(formatted);
            }
        }
    }
    Ok(text.unwrap_or_default())
}

/// The property flag of a text height a content format or cell style sets itself (the others it
/// takes from what it is part of: the cell's style, its row's, its column's, the table's).
const TEXT_HEIGHT: u32 = 0x80;

/// A text height a content format or cell style sets: `Some(None)` when it sets one that can't
/// be used, `None` when it sets none.
type OwnHeight = Option<Option<f64>>;

/// The text height of a format whose override flags are `flags`: its own when it sets one.
fn own_height(flags: u32, h: f64) -> OwnHeight {
    (flags & TEXT_HEIGHT != 0).then_some(Some(h).filter(|h| h.is_finite() && *h > 0.0 && *h < 1e9))
}

/// What a content format says of the text: its override flags, its height (AutoCAD fills it in
/// from the style when the format sets none), alignment, colour and text style.
pub(super) struct Format {
    pub overrides: u32,
    pub height: f64,
    pub align: u32,
    pub color: ColorValue,
    pub text_style: u64,
}

/// A content format: its overrides, value type and format, rotation, scale, alignment, colour,
/// text style and text height.
fn content_format(r: &mut ObjectReader) -> DwgResult<Format> {
    let overrides = r.bl()?;
    let _flags = (r.bl()?, r.bl()?, r.bl()?);
    let _format = r.text()?;
    let _placement = (r.bd()?, r.bd()?);
    let align = r.bl()?;
    let color = true_color(r)?;
    let text_style = r.reference()?;
    Ok(Format { overrides, height: r.bd()?, align, color, text_style })
}

/// A cell border: its lineweight (as stored, -2 ByBlock), whether it shows and its colour.
pub(super) struct Border {
    pub lineweight: i16,
    pub visible: bool,
    pub color: ColorValue,
}

/// A cell border: overrides, type, colour, lineweight, linetype, visibility (0 shown), double
/// spacing.
fn border(r: &mut ObjectReader) -> DwgResult<Border> {
    let _ = (r.bl()?, r.bl()?);
    let color = true_color(r)?;
    let lineweight = i16::try_from(i32::from_ne_bytes(r.bl()?.to_ne_bytes())).unwrap_or(-2);
    let _linetype = r.reference()?;
    let invisible = r.bl()?;
    let _double_spacing = r.bd()?;
    Ok(Border { lineweight, visible: invisible == 0, color })
}

/// A cell style with data: its two flag words, background, content format, margins (top, left,
/// bottom, right, horizontal and vertical spacing) and borders, by the edge's place in a DXF
/// table style (top, horizontal inside, bottom, left, vertical inside, right).
pub(super) struct CellStyleData {
    pub flags: (u32, u32),
    pub background: ColorValue,
    pub format: Format,
    pub margins: Option<[f64; 6]>,
    pub borders: [Option<Border>; 6],
}

/// A cell style (of a cell, row, column, the whole table or a table style): its type, then when
/// it has data, its flag words, background, layout, content format, margins and borders.
pub(super) fn cell_style_data(r: &mut ObjectReader) -> DwgResult<Option<CellStyleData>> {
    let _kind = r.bl()?;
    if r.bs()? == 0 {
        return Ok(None);
    }
    let flags = (r.bl()?, r.bl()?);
    let background = true_color(r)?;
    let _layout = r.bl()?;
    let format = content_format(r)?;
    let margins = if r.bs()? & 1 != 0 { Some([r.bd()?, r.bd()?, r.bd()?, r.bd()?, r.bd()?, r.bd()?]) } else { None };
    let mut borders: [Option<Border>; 6] = Default::default();
    let n = r.bl()?;
    for _ in 0..bounded(r, n, 2, "cell border")? {
        let edge = r.bl()?;
        let at = match edge {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            0x10 => 4,
            0x20 => 5,
            _ => continue,
        };
        if let Some(slot) = borders.get_mut(at) {
            *slot = Some(border(r)?);
        }
    }
    Ok(Some(CellStyleData { flags, background, format, margins, borders }))
}

/// A cell style (of a cell, row, column or the whole table): when it has data, its two flag
/// words, background, layout, content format, margins and borders. Returns the text height it
/// sets. AutoCAD's own files set the property bits in the first word (the rows of its sample
/// tables: 0x80, the heights their cells have in AutoCAD's DXF; a cell's margins: 0x63C00);
/// acadrust, DraftCraft's writer until it has its own, sets them in the second.
fn cell_style(r: &mut ObjectReader) -> DwgResult<OwnHeight> {
    let Some(s) = cell_style_data(r)? else { return Ok(None) };
    Ok(own_height(s.flags.0 | s.flags.1 | s.format.overrides, s.format.height))
}

/// Custom data: a name and a value.
fn custom_data(r: &mut ObjectReader) -> DwgResult<()> {
    let n = r.bl()?;
    for _ in 0..bounded(r, n, 4, "custom data item")? {
        let _name = r.text()?;
        value(r, true)?;
    }
    Ok(())
}

/// One cell of the table content (2010 and later), with the text height it sets itself: its
/// first content's, else its own style's. A later content's doesn't stand for the cell.
fn cell(r: &mut ObjectReader, partial: &mut Vec<&'static str>) -> DwgResult<(TableCell, OwnHeight)> {
    let _state = r.bl()?;
    let _tooltip = r.text()?;
    let _custom = r.bl()?;
    custom_data(r)?;
    if r.bl()? & 1 != 0 {
        let _link = r.reference()?;
        let _ = (r.bl()?, r.bl()?, r.bl()?);
    }
    let mut text = String::new();
    let mut first: OwnHeight = None;
    let n = r.bl()?;
    for i in 0..bounded(r, n, 4, "cell content")? {
        match r.bl()? {
            1 => text.push_str(&value(r, true)?),
            2 => {
                let _field = r.reference()?;
                partial.push("TABLE cell field");
            }
            4 => {
                let _block = r.reference()?;
                partial.push("TABLE cell block");
            }
            _ => {}
        }
        let n = r.bl()?;
        for _ in 0..bounded(r, n, 4, "cell attribute")? {
            let _definition = r.reference()?;
            let _value = r.text()?;
            let _index = r.bl()?;
        }
        if r.bs()? != 0 {
            let f = content_format(r)?;
            if i == 0 {
                first = own_height(f.overrides, f.height);
            }
        }
    }
    let own = first.or(cell_style(r)?);
    let _style_id = r.bl()?;
    if r.bl()? != 0 {
        let _ = (r.bl()?, r.bd()?, r.bd()?);
        let _geometry = r.reference()?;
        let n = r.bl()?;
        for _ in 0..bounded(r, n, 16, "cell content geometry")? {
            let _ = (r.bd3()?, r.bd3()?, r.bd()?, r.bd()?, r.bd()?, r.bd()?, r.bl()?);
        }
    }
    Ok((TableCell { text, text_height: own.flatten(), ..TableCell::default() }, own))
}

/// The table content (2010 and later): columns, rows of cells, fields, the table's own cell
/// style, the merged ranges and the table style.
fn content(r: &mut ObjectReader, t: &mut TableRead) -> DwgResult<()> {
    let _name = r.text()?;
    let _description = r.text()?;
    let n = r.bl()?;
    let mut column_heights: Vec<OwnHeight> = Vec::new();
    for _ in 0..bounded(r, n, 8, "table column")? {
        let _name = r.text()?;
        let _custom = r.bl()?;
        custom_data(r)?;
        column_heights.push(cell_style(r)?);
        let _style_id = r.bl()?;
        t.col_widths.push(r.bd()?);
    }
    let n = r.bl()?;
    for _ in 0..bounded(r, n, 8, "table row")? {
        let n = r.bl()?;
        let mut row = Vec::new();
        for _ in 0..bounded(r, n, 16, "table cell")? {
            row.push(cell(r, &mut t.partial)?);
        }
        let _custom = r.bl()?;
        custom_data(r)?;
        let row_height = cell_style(r)?;
        let _style_id = r.bl()?;
        t.row_heights.push(r.bd()?);
        // A cell that sets no text height takes its row's, else its column's; one none of them
        // sets takes the table's (the drawing's: no height of its own).
        let cells = row.into_iter().enumerate().map(|(j, (mut c, own))| {
            if own.is_none() {
                c.text_height = row_height.or(column_heights.get(j).copied().flatten()).flatten();
            }
            c
        });
        t.cells.push(cells.collect());
    }
    let n = r.bl()?;
    for _ in 0..bounded(r, n, 0, "table field")? {
        let _field = r.reference()?;
    }
    cell_style(r)?;
    let n = r.bl()?;
    for _ in 0..bounded(r, n, 8, "merged range")? {
        let [top, left, bottom, right] = [r.bl()?, r.bl()?, r.bl()?, r.bl()?];
        let rows = bottom.checked_sub(top).and_then(|d| d.checked_add(1));
        let cols = right.checked_sub(left).and_then(|d| d.checked_add(1));
        let at = t.cells.get_mut(usize::try_from(top).unwrap_or(usize::MAX)).and_then(|row| row.get_mut(usize::try_from(left).unwrap_or(usize::MAX)));
        if let (Some(c), Some(rows), Some(cols)) = (at, rows, cols) {
            c.merged = Some((rows, cols));
        }
    }
    t.style = r.reference()?;
    Ok(())
}

/// One cell before 2010: its type, edges, merge, rotation, value (text, or a block with
/// attribute values) and the overrides it has of its style.
fn legacy_cell(r: &mut ObjectReader, partial: &mut Vec<&'static str>) -> DwgResult<TableCell> {
    let ver = r.version();
    let kind = r.bs()?;
    let _edges = r.rc()?;
    let _merged = r.b()?;
    let _autofit = r.b()?;
    let (merge_width, merge_height) = (r.bl()?, r.bl()?);
    let _rotation = r.bd()?;
    let value_object = r.reference()?;
    let mut text = String::new();
    if kind == 2 {
        let _scale = r.bd()?;
        if r.b()? {
            let n = u32::from(r.bs()?);
            for _ in 0..bounded(r, n, 4, "cell attribute")? {
                let _definition = r.reference()?;
                let _index = r.bs()?;
                let _value = r.text()?;
            }
        }
        partial.push("TABLE cell block");
    } else if value_object == 0 && ver < V::R2007 {
        text = r.text()?;
    } else if value_object != 0 {
        partial.push("TABLE cell field");
    }
    let mut text_height = None;
    if r.b()? {
        let flags = r.bl()?;
        let _virtual_edge = r.rc()?;
        if flags & 0x01 != 0 {
            let _alignment = r.bs()?;
        }
        if flags & 0x02 != 0 {
            let _no_fill = r.b()?;
        }
        if flags & 0x04 != 0 {
            true_color(r)?;
        }
        if flags & 0x08 != 0 {
            true_color(r)?;
        }
        if flags & 0x10 != 0 {
            let _text_style = r.reference()?;
        }
        if flags & 0x20 != 0 {
            text_height = Some(r.bd()?).filter(|h| h.is_finite() && *h > 0.0 && *h < 1e9);
        }
        for (color, lineweight) in [(0x40, 0x400), (0x80, 0x800), (0x100, 0x1000), (0x200, 0x2000)] {
            if flags & color != 0 {
                true_color(r)?;
            }
            if flags & lineweight != 0 {
                let _ = (r.bs()?, r.bs()?);
            }
        }
    }
    if ver >= V::R2007 {
        let _flag = r.bl()?;
        let shown = value(r, ver >= V::R2007)?;
        if text.is_empty() {
            text = shown;
        }
    }
    let merged = (merge_height >= 1 && merge_width >= 1 && (merge_height, merge_width) != (1, 1)).then_some((merge_height, merge_width));
    Ok(TableCell { text, merged, text_height, ..TableCell::default() })
}

/// The table's own fields after its block reference's, up to the end of the record.
pub(super) fn read(r: &mut ObjectReader) -> DwgResult<TableRead> {
    let mut t = TableRead { col_widths: Vec::new(), row_heights: Vec::new(), cells: Vec::new(), style: 0, partial: Vec::new() };
    if r.version() >= V::R2010 {
        modern(r, &mut t)?
    } else {
        legacy(r, &mut t)?
    }
    Ok(t)
}

/// The table from AutoCAD 2010 on: a few fields, the table content, the direction and the
/// break settings.
fn modern(r: &mut ObjectReader, t: &mut TableRead) -> DwgResult<()> {
    let _unknown = r.rc()?;
    let _object = r.reference()?;
    let _ = r.bl()?;
    if r.version() >= V::R2013 {
        let _ = r.bl()?;
    } else {
        let _ = r.b()?;
    }
    content(r, t)?;
    let _unknown = r.bs()?;
    let _direction = r.bd3()?;
    if r.bl()? == 1 {
        let _ = (r.bl()?, r.bl()?, r.bd()?, r.bl()?, r.bl()?);
        let n = r.bl()?;
        for _ in 0..bounded(r, n, 10, "table break")? {
            let _ = (r.bd3()?, r.bd()?, r.bl()?);
        }
    }
    let n = r.bl()?;
    for _ in 0..bounded(r, n, 10, "table break range")? {
        let _ = (r.bd3()?, r.bl()?, r.bl()?);
    }
    if t.cells.iter().any(|row| row.len() != t.col_widths.len()) {
        return Err(Error::format("table rows of other lengths than its columns"));
    }
    Ok(())
}

/// The table before 2010: its sizes, style and cells, then its overrides.
fn legacy(r: &mut ObjectReader, t: &mut TableRead) -> DwgResult<()> {
    let _flags = r.bs()?;
    let _direction = r.bd3()?;
    let (cols, rows) = (r.bl()?, r.bl()?);
    let cells = u64::from(cols).saturating_mul(u64::from(rows));
    if cells > u64::from(MAX_ITEMS) {
        return Err(Error::format(format!("table of {rows} × {cols} cells")));
    }
    let cols = bounded(r, cols, 2, "table column")?;
    let rows = bounded(r, rows, 2, "table row")?;
    for _ in 0..cols {
        t.col_widths.push(r.bd()?);
    }
    for _ in 0..rows {
        t.row_heights.push(r.bd()?);
    }
    t.style = r.reference()?;
    for _ in 0..rows {
        let mut row = Vec::with_capacity(cols);
        for _ in 0..cols {
            row.push(legacy_cell(r, &mut t.partial)?);
        }
        t.cells.push(row);
    }
    legacy_overrides(r)
}

/// The overrides of the table style, then of its borders' colours, lineweights and visibility,
/// each there when its flag is: none of these is kept by the drawing.
fn legacy_overrides(r: &mut ObjectReader) -> DwgResult<()> {
    if r.b()? {
        let flags = r.bl()?;
        let has = |bits: &[u32]| bits.iter().filter(|b| flags & **b != 0).count();
        for _ in 0..has(&[0x01, 0x02]) {
            let _ = r.b()?;
        }
        if flags & 0x04 != 0 {
            let _ = r.bs()?;
        }
        for _ in 0..has(&[0x08, 0x10]) {
            let _ = r.bd()?;
        }
        for _ in 0..has(&[0x20, 0x40, 0x80]) {
            true_color(r)?;
        }
        for _ in 0..has(&[0x100, 0x200, 0x400]) {
            let _ = r.b()?;
        }
        for _ in 0..has(&[0x800, 0x1000, 0x2000]) {
            true_color(r)?;
        }
        for _ in 0..has(&[0x4000, 0x8000, 0x10000]) {
            let _ = r.bs()?;
        }
        for _ in 0..has(&[0x20000, 0x40000, 0x80000]) {
            let _ = r.reference()?;
        }
        for _ in 0..has(&[0x100000, 0x200000, 0x400000]) {
            let _ = r.bd()?;
        }
    }
    if r.b()? {
        let flags = r.bl()?;
        for _ in 0..(0..18).filter(|bit| flags & (1 << bit) != 0).count() {
            true_color(r)?;
        }
    }
    for _ in 0..2 {
        if r.b()? {
            let flags = r.bl()?;
            for _ in 0..(0..18).filter(|bit| flags & (1 << bit) != 0).count() {
                let _ = r.bs()?;
            }
        }
    }
    Ok(())
}

/// One row of a table style as a DXF TABLESTYLE writes it (groups 7 to 69).
struct RowStyle {
    text_style: String,
    height: f64,
    align: i64,
    text_color: ColorValue,
    fill: Option<ColorValue>,
    borders: [(i16, bool, ColorValue); 6],
}

/// A colour as the AutoCAD Color Index number a DXF table style holds (0 ByBlock, 256 ByLayer).
fn aci(c: ColorValue) -> i64 {
    match c {
        ColorValue::ByBlock => 0,
        ColorValue::ByLayer | ColorValue::None => 256,
        ColorValue::Index(i) => i64::from(i),
        ColorValue::Rgb(r, g, b) => i64::from(draftcraft_color::Color::True(draftcraft_color::Rgb(r, g, b)).to_aci()),
    }
}

/// The DXF groups of a table style, as AutoCAD writes them and the DXF reader reads them: the
/// version (from 2010), description, flow, flags, margins and suppressed rows, then the data,
/// title and header rows. A row style's text style is by name (`styles`, by handle).
fn style_tags(
    version: Option<i64>,
    description: String,
    flow: i64,
    flags: i64,
    margins: (f64, f64),
    suppressed: (bool, bool),
    rows: [&RowStyle; 3],
) -> Vec<Tag> {
    let mut t = vec![Tag::s(100, "AcDbTableStyle")];
    if let Some(v) = version {
        t.push(Tag::i(280, v));
    }
    t.extend([
        Tag::s(3, description),
        Tag::i(70, flow),
        Tag::i(71, flags),
        Tag::f(40, margins.0),
        Tag::f(41, margins.1),
        Tag::i(280, i64::from(suppressed.0)),
        Tag::i(281, i64::from(suppressed.1)),
    ]);
    for row in rows {
        t.extend([
            Tag::s(7, row.text_style.clone()),
            Tag::f(140, row.height),
            Tag::i(170, row.align),
            Tag::i(62, aci(row.text_color)),
            Tag::i(63, aci(row.fill.unwrap_or(ColorValue::Index(7)))),
            Tag::i(283, i64::from(row.fill.is_some())),
        ]);
        t.extend(row.borders.iter().enumerate().map(|(k, b)| Tag::i(274 + k as i32, i64::from(b.0))));
        t.extend(row.borders.iter().enumerate().map(|(k, b)| Tag::i(284 + k as i32, i64::from(b.1))));
        t.extend(row.borders.iter().enumerate().map(|(k, b)| Tag::i(64 + k as i32, aci(b.2))));
    }
    t
}

/// TABLESTYLE (§20.4.?), as the groups of AutoCAD's DXF of it (`style_tags`). Before 2010: the
/// description, flow, flags, margins, suppressed rows, then the data, title and header rows, each
/// its text style, height, alignment, colours, fill and six borders (and from 2007 its value's
/// type, unit and format). From 2010: a version, the description, the flags (1 title, 2 header
/// suppressed), a number, then the named cell styles, each its data then its id, class and name:
/// the table's own ("Table", whose margins the style's are), then the rows, _TITLE, _HEADER and
/// _DATA (ids 1, 2, 3), each after its key. AutoCAD's own files (ACadSharp's samples) read to the
/// end of the record this way, with AutoCAD's DXF's values.
pub(super) fn table_style(r: &mut ObjectReader, styles: &std::collections::HashMap<u64, String>) -> DwgResult<Vec<Tag>> {
    let ver = r.version();
    r.common_refs()?;
    let name_of = |h: u64| styles.get(&h).cloned().unwrap_or_else(|| "Standard".into());
    if ver < V::R2010 {
        let description = r.text()?;
        let flow = i64::from(r.bs()?);
        let flags = i64::from(r.bs()?);
        let margins = (r.bd()?, r.bd()?);
        let suppressed = (r.b()?, r.b()?);
        let mut rows = Vec::with_capacity(3);
        for _ in 0..3 {
            let text_style = name_of(r.reference()?);
            let height = r.bd()?;
            let align = i64::from(r.bs()?);
            // Colours as the true-colour record, in every version (the type came with AutoCAD
            // 2005; MI-201's R14 and AutoCAD's 2004 files read so).
            let text_color = true_color(r)?;
            let fill_color = true_color(r)?;
            let filled = r.b()?;
            let mut borders = [(-2i16, true, ColorValue::ByBlock); 6];
            for b in &mut borders {
                let lineweight = i16::from_ne_bytes(r.bs()?.to_ne_bytes());
                let visible = r.b()?;
                *b = (lineweight, visible, true_color(r)?);
            }
            if ver >= V::R2007 {
                let _value = (r.bl()?, r.bl()?, r.text()?);
            }
            rows.push(RowStyle { text_style, height, align, text_color, fill: filled.then_some(fill_color), borders });
        }
        let [data, title, header] = rows.as_slice() else { return Err(Error::format("table style rows")) };
        return Ok(style_tags(None, description, flow, flags, margins, suppressed, [data, title, header]));
    }
    let version = i64::from(r.rc()?);
    let description = r.text()?;
    let flags = r.bl()?;
    let _number = r.bl()?;
    let _cell_style = r.reference()?;
    // A named cell style: its data, then its id, class and name.
    let named = |r: &mut ObjectReader| -> DwgResult<(Option<CellStyleData>, u32, String)> {
        let data = cell_style_data(r)?;
        let id = r.bl()?;
        let _class = r.bl()?;
        Ok((data, id, r.text()?))
    };
    let (table, _, _) = named(r)?;
    let n = r.bl()?;
    let mut rows: Vec<(u32, String, Option<CellStyleData>)> = Vec::new();
    for _ in 0..bounded(r, n, 16, "table style cell style")? {
        let _key = r.bl()?;
        let (data, id, name) = named(r)?;
        rows.push((id, name, data));
    }
    let margins = table.as_ref().and_then(|t| t.margins).map_or((0.06, 0.06), |m| (m[1], m[0]));
    let flow = i64::from(table.as_ref().is_some_and(|t| t.flags.1 & 0x1_0000 != 0));
    let row = |name: &str, id: u32| -> RowStyle {
        let data = rows.iter().find(|(i, n, _)| n.eq_ignore_ascii_case(name) || (n.is_empty() && *i == id)).and_then(|(_, _, d)| d.as_ref());
        match data {
            Some(s) => RowStyle {
                text_style: name_of(s.format.text_style),
                height: s.format.height,
                align: i64::from(s.format.align),
                text_color: s.format.color,
                fill: (s.background != ColorValue::None).then_some(s.background),
                borders: s.borders.each_ref().map(|b| b.as_ref().map_or((-2, true, ColorValue::ByBlock), |b| (b.lineweight, b.visible, b.color))),
            },
            None => RowStyle {
                text_style: "Standard".into(),
                height: 0.18,
                align: 5,
                text_color: ColorValue::ByBlock,
                fill: None,
                borders: [(-2, true, ColorValue::ByBlock); 6],
            },
        }
    };
    let (data, title, header) = (row("_DATA", 3), row("_TITLE", 1), row("_HEADER", 2));
    let suppressed = (flags & 1 != 0, flags & 2 != 0);
    Ok(style_tags(Some(version), description, flow, i64::from(flags), margins, suppressed, [&data, &title, &header]))
}
