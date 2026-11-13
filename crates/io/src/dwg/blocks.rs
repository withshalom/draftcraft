//! The blocks and layouts of a DWG file, into the drawing: the block table in its order, each
//! block definition's own properties, and each layout with its page setup. The entities a block
//! owns are listed here and read with the entities.

use std::collections::{HashMap, HashSet};

use draftcraft_doc::{Block, Drawing, Layout};
use draftcraft_dwg::DwgVersion as V;
use draftcraft_dwg::file::{DwgFile, ObjectReader, ObjectRef, XDataItem};
use draftcraft_dwg::records::Result as DwgResult;
use draftcraft_dxf::Tag;
use draftcraft_geom::{Vec2, Vec3};

use super::Unread;
use super::tables::Tables;

/// Most blocks, layouts or owned entities that are read: more than any real drawing has.
const MAX: usize = 1 << 22;

/// Where the entities of one block record go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Space {
    Model,
    /// A paper space block, with the name of its layout.
    Layout(String),
    /// A block definition, by its name in the drawing.
    Block(String),
    /// An external reference: its entities are the other drawing's.
    Xref,
}

/// One block record: where its entities go and which they are.
pub(super) struct BlockRecord {
    pub handle: u64,
    pub space: Space,
    /// The entities the block lists, in order (AutoCAD 2004 and later).
    pub owned: Vec<u64>,
    /// The first entity of the block's chain (up to AutoCAD 2000), 0 for none.
    pub first: u64,
}

/// The block table as read.
#[derive(Default)]
pub(super) struct Blocks {
    /// The name of every block record in the drawing, by handle.
    pub names: HashMap<u64, String>,
    /// The block records in table order.
    pub records: Vec<BlockRecord>,
    /// The records of model space and of the active paper space, which an entity may name by a
    /// flag in place of a reference.
    pub model: u64,
    pub paper: u64,
    /// The handles of the objects this category read.
    pub read: HashSet<u64>,
}

struct Header {
    name: String,
    anonymous: bool,
    xref: bool,
    overlay: bool,
    base: Vec3,
    xref_path: String,
    description: String,
    /// Insertion units and whether the block can be exploded (AutoCAD 2007 and later).
    props: Option<(u8, bool)>,
    block: u64,
    endblk: u64,
    owned: Vec<u64>,
    first: u64,
}

struct LayoutRead {
    layout: Layout,
    block: u64,
}

/// The name every table record starts with (see `tables::record_name`).
fn record_name(r: &mut ObjectReader) -> DwgResult<String> {
    let name = r.text()?;
    if r.has_string_stream() {
        r.bs()?;
    } else {
        r.b()?;
        r.bs()?;
        r.b()?;
    }
    Ok(name)
}

/// BLOCK_HEADER (specification §20.4.49).
fn header(r: &mut ObjectReader) -> DwgResult<Header> {
    let name = record_name(r)?;
    let ver = r.version();
    let anonymous = r.b()?;
    let _has_attributes = r.b()?;
    let xref = r.b()?;
    let overlay = r.b()?;
    if ver >= V::R2000 {
        let _loaded = r.b()?;
    }
    let lists = !xref && !overlay;
    let owned_count = if ver >= V::R2004 && lists { r.bl()? } else { 0 };
    let [x, y, z] = r.bd3()?;
    let xref_path = r.text()?;
    let mut inserts = 0usize;
    let mut description = String::new();
    if ver >= V::R2000 {
        // One non-zero byte per insert of the block, ended by a zero.
        while r.rc()? != 0 {
            inserts += 1;
            if inserts > MAX {
                return Err(draftcraft_dwg::records::Error::format("BLOCK_HEADER: too many inserts"));
            }
        }
        description = r.text()?;
        let preview = r.bl()?;
        r.skip_bytes(usize::try_from(preview).unwrap_or(usize::MAX))?;
    }
    let mut props = None;
    if ver >= V::R2007 {
        let units = r.bs()?;
        let explodable = r.b()?;
        let _scaling = r.rc()?;
        props = Some((u8::try_from(units).unwrap_or(0), explodable));
    }
    r.common_refs()?;
    let _null = r.reference()?;
    let block = r.reference()?;
    // The entities the block owns: their first and last up to AutoCAD 2000, each of them from
    // 2004.
    let (mut owned, mut first) = (Vec::new(), 0);
    if lists {
        if ver <= V::R2000 {
            first = r.reference()?;
            let _last = r.reference()?;
        } else {
            for _ in 0..usize::try_from(owned_count).unwrap_or(MAX).min(MAX) {
                owned.push(r.reference()?);
            }
        }
    }
    let endblk = r.reference()?;
    Ok(Header { name, anonymous, xref, overlay, base: Vec3::new(x, y, z), xref_path, description, props, block, endblk, owned, first })
}

/// The block control object: the block records in table order, then model space and paper space.
fn control(r: &mut ObjectReader) -> DwgResult<Vec<u64>> {
    let n = usize::try_from(r.bl()?).unwrap_or(MAX).min(MAX);
    r.common_refs()?;
    let mut entries = Vec::new();
    for _ in 0..n {
        entries.push(r.reference()?);
    }
    let model = r.reference()?;
    let paper = r.reference()?;
    let mut order = vec![model, paper];
    order.extend(entries);
    Ok(order)
}

/// LAYOUT (specification §20.4.84): the plot settings, then the layout's own fields. The page
/// setup is made from its DXF groups by the rule the DXF reader uses.
fn layout(r: &mut ObjectReader, apps: &HashMap<u64, String>) -> DwgResult<LayoutRead> {
    let ver = r.version();
    let mut plot: Vec<Tag> = Vec::with_capacity(32);
    let page_setup_name = r.text()?;
    plot.push(Tag::s(1, page_setup_name));
    let printer = r.text()?;
    plot.push(Tag::s(2, printer));
    let flags = r.bs()?;
    plot.push(Tag::i(70, i64::from(flags)));
    for code in [40, 41, 42, 43, 44, 45] {
        let v = r.bd()?;
        plot.push(Tag::f(code, v));
    }
    let paper = r.text()?;
    plot.push(Tag::s(4, paper));
    let [ox, oy] = r.bd2()?;
    plot.push(Tag::f(46, ox));
    plot.push(Tag::f(47, oy));
    for code in [72, 73, 74] {
        let v = r.bs()?;
        plot.push(Tag::i(code, i64::from(v)));
    }
    for code in [48, 140] {
        let [x, y] = r.bd2()?;
        plot.push(Tag::f(code, x));
        plot.push(Tag::f(if code == 48 { 49 } else { 141 }, y));
    }
    if ver <= V::R2000 {
        let _plot_view = r.text()?;
    }
    for code in [142, 143] {
        let v = r.bd()?;
        plot.push(Tag::f(code, v));
    }
    let style_sheet = r.text()?;
    plot.push(Tag::s(7, style_sheet));
    let scale_type = r.bs()?;
    plot.push(Tag::i(75, i64::from(scale_type)));
    let scale = r.bd()?;
    plot.push(Tag::f(147, scale));
    let [ix, iy] = r.bd2()?;
    plot.push(Tag::f(148, ix));
    plot.push(Tag::f(149, iy));
    if ver >= V::R2004 {
        for _ in 0..3 {
            r.bs()?;
        }
    }
    let name = r.text()?;
    let tab_order = r.bl()?;
    let _flags = r.bs()?;
    let _ucs_origin = r.bd3()?;
    let [x0, y0] = r.rd2()?;
    let [x1, y1] = r.rd2()?;
    let own = [Tag::f(10, x0), Tag::f(20, y0), Tag::f(11, x1), Tag::f(21, y1)];
    // The rest of the fields (insertion base, UCS, extents, viewports) are nothing the drawing
    // keeps per layout; the references follow.
    let _insbase = r.bd3()?;
    let _ucs_x = r.bd3()?;
    let _ucs_y = r.bd3()?;
    let _elevation = r.bd()?;
    let _ortho = r.bs()?;
    let _extmin = r.bd3()?;
    let _extmax = r.bd3()?;
    if ver >= V::R2004 {
        let _viewports = r.bl()?;
    }
    // The layout's saved paper space view, from DraftCraft's own extended data.
    let x = r.xdata();
    let mine = x.iter().find(|b| apps.get(&b.app).is_some_and(|n| n.eq_ignore_ascii_case(crate::dxf_ext::APP))).map_or(&[][..], |b| &b.items);
    let start = mine.iter().position(|v| matches!(v, XDataItem::Text(s) if s == "PSVIEW"));
    let view = start.and_then(|i| {
        let list = mine.get(i + 1..)?;
        let point = list.iter().find_map(|v| if let XDataItem::Point(p) = v { Some(Vec2::new(p[0], p[1])) } else { None })?;
        let height = list.iter().find_map(|v| if let XDataItem::Real(h) = v { Some(*h) } else { None })?;
        (point.is_finite() && height.is_finite()).then_some((point, height))
    });
    r.common_refs()?;
    if ver >= V::R2004 {
        let _plot_view = r.reference()?;
    }
    if ver >= V::R2007 {
        let _visual_style = r.reference()?;
    }
    let block = r.reference()?;
    let layout = Layout {
        page: crate::dxf_read::page_setup_of(&plot),
        limits: crate::dxf_read::layout_limits_of(&own),
        view,
        ..Layout::new(&name, tab_order)
    };
    Ok(LayoutRead { layout, block })
}

/// Read the block table and the layouts into `d`.
pub(super) fn read(file: &DwgFile, objects: &[ObjectRef], tables: &Tables, d: &mut Drawing, unread: &mut Unread) -> Blocks {
    let mut out = Blocks::default();
    let by_handle: HashMap<u64, &ObjectRef> = objects.iter().map(|o| (o.handle, o)).collect();
    // The table's order: model space, paper space, the table's entries, then any record the
    // table does not list, in handle order.
    let mut order: Vec<u64> = Vec::new();
    for o in objects.iter().filter(|o| o.type_name == "BLOCK_CONTROL").take(1) {
        if let Ok(list) = file.read_object(o).and_then(|mut r| control(&mut r)) {
            order = list;
        }
    }
    // The active paper space is the one the control object names: a file may have more records
    // called *Paper_Space (some writers don't number the others).
    let control_paper = order.get(1).copied().filter(|h| *h != 0);
    order.extend(objects.iter().filter(|o| o.type_name == "BLOCK_HEADER").map(|o| o.handle));
    let mut seen = HashSet::new();
    let mut headers: Vec<(u64, Header)> = Vec::new();
    for h in order.into_iter().filter(|h| seen.insert(*h)).take(MAX) {
        let Some(o) = by_handle.get(&h).filter(|o| o.type_name == "BLOCK_HEADER") else { continue };
        match file.read_object(o).and_then(|mut r| header(&mut r)) {
            Ok(b) => headers.push((h, b)),
            Err(_) => unread.count("BLOCK_HEADER (not decoded)"),
        }
    }
    // DWG stores anonymous blocks unnumbered (`*D`, `*U`); each gets a number per prefix in table
    // order, as AutoCAD numbers them in a DXF (`*D0`, `*D1`), skipping names already used.
    let mut taken: HashSet<String> = headers.iter().map(|(_, b)| b.name.to_ascii_uppercase()).collect();
    let mut next: HashMap<String, usize> = HashMap::new();
    for (h, b) in &headers {
        let bare = b.name.len() >= 2
            && b.name.starts_with('*')
            && b.name.chars().skip(1).all(|c| c.is_ascii_alphabetic())
            && !b.name.eq_ignore_ascii_case("*Model_Space")
            && !b.name.eq_ignore_ascii_case("*Paper_Space");
        let name = if bare {
            let n = next.entry(b.name.to_ascii_uppercase()).or_insert(0);
            loop {
                let candidate = format!("{}{}", b.name, n);
                *n += 1;
                if taken.insert(candidate.to_ascii_uppercase()) {
                    break candidate;
                }
            }
        } else {
            b.name.clone()
        };
        out.names.insert(*h, name);
    }
    // Layouts, in tab order. Model space has a layout object too; it is not a tab of its own.
    let mut layouts: Vec<LayoutRead> = Vec::new();
    for o in objects.iter().filter(|o| o.type_name == "LAYOUT").take(MAX) {
        match file.read_object(o).and_then(|mut r| layout(&mut r, &tables.apps)) {
            Ok(l) => {
                layouts.push(l);
                out.read.insert(o.handle);
            }
            Err(_) => unread.count("LAYOUT (not decoded)"),
        }
    }
    layouts.sort_by_key(|l| l.layout.tab_order);
    d.layouts.clear();
    let mut layout_of_block: HashMap<u64, String> = HashMap::new();
    for l in layouts {
        if l.layout.name.eq_ignore_ascii_case("Model") {
            continue;
        }
        layout_of_block.insert(l.block, l.layout.name.clone());
        d.layouts.push(l.layout);
    }
    let paper_space = control_paper
        .filter(|p| headers.iter().any(|(h, _)| h == p))
        .or_else(|| headers.iter().find(|(_, b)| b.name.eq_ignore_ascii_case("*Paper_Space")).map(|(h, _)| *h));
    if d.layouts.is_empty() {
        d.layouts.push(Layout::new("Layout1", 1));
        if let Some(h) = paper_space {
            layout_of_block.insert(h, "Layout1".into());
        }
    }
    // The layout in `*Paper_Space` is the active one, kept when it isn't the first tab.
    let active = paper_space.and_then(|h| layout_of_block.get(&h).cloned());
    let first_tab = d.layouts.iter().min_by_key(|l| l.tab_order).map(|l| l.name.clone());
    d.active_layout = active.clone().filter(|a| Some(a) != first_tab.as_ref());
    let first_layout = active.or_else(|| d.layouts.first().map(|l| l.name.clone())).unwrap_or_else(|| "Layout1".into());
    for (h, b) in headers {
        let name = out.names.get(&h).cloned().unwrap_or_else(|| b.name.clone());
        let up = name.to_ascii_uppercase();
        let space = if up == "*MODEL_SPACE" {
            out.model = h;
            Space::Model
        } else if up.starts_with("*PAPER_SPACE") {
            if Some(h) == paper_space {
                out.paper = h;
            }
            Space::Layout(layout_of_block.get(&h).cloned().unwrap_or_else(|| first_layout.clone()))
        } else {
            let mut block = Block::new(&name);
            block.base = b.base;
            block.description.clone_from(&b.description);
            block.anonymous = b.anonymous || name.starts_with('*');
            if let Some((units, explodable)) = b.props {
                (block.units, block.explodable) = (units, explodable);
            }
            if b.xref {
                block.xref_path = Some(b.xref_path.clone());
                block.xref_overlay = b.overlay;
            }
            d.blocks.insert(name.clone(), std::sync::Arc::new(block));
            if b.xref { Space::Xref } else { Space::Block(name) }
        };
        out.records.push(BlockRecord { handle: h, space, owned: b.owned, first: b.first });
        // The record, and the BLOCK and ENDBLK objects that open and close its entities: the
        // block itself carries what they hold.
        out.read.extend([h, b.block, b.endblk]);
    }
    // A table's control object holds the list of its records and nothing of its own.
    out.read.extend(objects.iter().filter(|o| o.type_name.ends_with("_CONTROL")).map(|o| o.handle));
    out
}
