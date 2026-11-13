//! The drawing database: every object in a DWG file keyed by its handle,
//! with typed data for the objects the core understands.
//!
//! The model mirrors DWG's own object graph (handles, owners, reactors,
//! extension dictionaries, extended data) rather than a simplified geometry
//! list, so that
//! DWG maps onto it, and onto the native `.cad` format, without losing
//! structure.

pub mod color;

pub mod lineweight;
mod objects;
mod store;

pub use color::Color;

pub use lineweight::Lineweight;
pub use objects::*;
pub use store::{Journal, ListOp, ObjectMap, Touch};

use crate::records::geom::{Vec2, Vec3};
use crate::records::version::DwgVersion;
use std::collections::BTreeMap;
use std::fmt;

/// An object's database handle. 0 means "no object".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Handle(pub u64);

impl Handle {
    pub const NULL: Handle = Handle(0);
    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:X}", self.0)
    }
}

/// Where an entity lives, from the entity-mode bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Space {
    /// Owned by a block, polyline, insert… (see `Object::owner`).
    #[default]
    Owned,
    /// The active paper-space layout.
    Paper,
    /// Model space.
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinetypeRef {
    #[default]
    ByLayer,
    ByBlock,
    Continuous,
    Handle(Handle),
}

/// Properties shared by every entity.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityCommon {
    pub space: Space,
    pub layer: Handle,
    pub linetype: LinetypeRef,
    pub color: Color,
    /// Raw transparency word when present (R2004+).
    pub transparency: Option<u32>,
    pub linetype_scale: f64,
    /// Lineweight code as DWG stores it (see [`Lineweight`]); files older
    /// than R2000 have none, and their entities read as by layer.
    pub lineweight: u8,
    pub invisible: bool,
    pub plotstyle: Handle,
    pub material: Handle,
    /// The DBCOLOR object of a color book color (R2004+), whose true color and names the
    /// entity takes; [`Handle::NULL`] for none.
    pub color_book: Handle,
}

impl Default for EntityCommon {
    /// An entity that takes everything from its layer: colour, linetype
    /// and lineweight by layer, linetype scale 1.
    fn default() -> Self {
        EntityCommon {
            space: Space::Owned,
            layer: Handle::NULL,
            linetype: LinetypeRef::ByLayer,
            color: Color::ByLayer,
            transparency: None,
            linetype_scale: 1.0,
            lineweight: Lineweight::CODE_BY_LAYER,
            invisible: false,
            plotstyle: Handle::NULL,
            material: Handle::NULL,
            color_book: Handle::NULL,
        }
    }
}

impl EntityCommon {
    /// A model-space entity on `layer`, everything else by layer.
    pub fn in_model_space(layer: Handle) -> Self {
        EntityCommon { space: Space::Model, layer, ..Default::default() }
    }

    /// The lineweight code, interpreted.
    pub fn weight(&self) -> Lineweight {
        Lineweight::from_code(self.lineweight)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Line {
    pub start: Vec3,
    pub end: Vec3,
    pub thickness: f64,
    pub extrusion: Vec3,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Circle {
    /// Centre in the entity's OCS.
    pub center: Vec3,
    pub radius: f64,
    pub thickness: f64,
    pub extrusion: Vec3,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Arc {
    /// Centre in the entity's OCS.
    pub center: Vec3,
    pub radius: f64,
    pub thickness: f64,
    pub extrusion: Vec3,
    /// Radians, counter-clockwise in the OCS.
    pub start_angle: f64,
    pub end_angle: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Point {
    pub position: Vec3,
    pub thickness: f64,
    pub extrusion: Vec3,
    pub x_axis_angle: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ellipse {
    /// WCS centre.
    pub center: Vec3,
    /// WCS vector from the centre to the end of the major axis.
    pub major_axis: Vec3,
    pub extrusion: Vec3,
    pub axis_ratio: f64,
    pub start_param: f64,
    pub end_param: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LwPolyline {
    pub flags: u16,
    pub const_width: f64,
    pub elevation: f64,
    pub thickness: f64,
    pub extrusion: Vec3,
    /// OCS vertices.
    pub points: Vec<Vec2>,
    /// One bulge per vertex, or empty when none are stored.
    pub bulges: Vec<f64>,
    pub widths: Vec<(f64, f64)>,
    pub vertex_ids: Vec<u32>,
}

impl LwPolyline {
    pub const FLAG_CLOSED: u16 = 0x200;
    pub fn is_closed(&self) -> bool {
        self.flags & Self::FLAG_CLOSED != 0
    }
    pub fn bulge(&self, i: usize) -> f64 {
        self.bulges.get(i).copied().unwrap_or(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PolylineKind {
    #[default]
    Polyline2d,
    Polyline3d,
    PolyfaceMesh,
    PolygonMesh,
}

/// Old-style POLYLINE header. Its vertices are separate VERTEX objects.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Polyline {
    pub kind: PolylineKind,
    pub flags: u16,
    pub curve_type: u16,
    pub start_width: f64,
    pub end_width: f64,
    pub thickness: f64,
    pub elevation: f64,
    pub extrusion: Vec3,
    /// Vertex handles in order when the file lists them (R2004+), or the
    /// first/last vertex (R13–R2000).
    pub vertices: Vec<Handle>,
    pub first_vertex: Handle,
    pub last_vertex: Handle,
    pub seqend: Handle,
}

impl Polyline {
    pub fn is_closed(&self) -> bool {
        self.flags & 1 != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VertexKind {
    #[default]
    Vertex2d,
    Vertex3d,
    Mesh,
    PfaceVertex,
    PfaceFace,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Vertex {
    pub kind: VertexKind,
    pub flags: u8,
    pub point: Vec3,
    pub start_width: f64,
    pub end_width: f64,
    pub bulge: f64,
    pub tangent_dir: f64,
    pub id: u32,
    /// Polyface face vertex indices (1-based, negative = invisible edge).
    pub face: [i16; 4],
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Text {
    /// OCS insertion point (x, y); z is `elevation`.
    pub insertion: Vec2,
    pub alignment: Vec2,
    pub elevation: f64,
    pub extrusion: Vec3,
    pub thickness: f64,
    pub oblique: f64,
    pub rotation: f64,
    pub height: f64,
    pub width_factor: f64,
    pub value: String,
    pub generation: u16,
    pub h_align: u16,
    pub v_align: u16,
    pub style: Handle,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Attribute {
    pub text: Text,
    pub tag: String,
    pub prompt: String,
    pub flags: u8,
    pub field_length: u16,
    pub is_definition: bool,
    /// Lock position (AutoCAD 2007 and later): the attribute stays where
    /// it is in its block reference, with no grip of its own. A reference's
    /// attributes take it from their definitions.
    pub lock_position: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MText {
    pub insertion: Vec3,
    pub extrusion: Vec3,
    pub x_axis: Vec3,
    pub rect_width: f64,
    pub rect_height: f64,
    pub text_height: f64,
    pub attachment: u16,
    pub drawing_dir: u16,
    /// Raw contents with inline formatting codes.
    pub contents: String,
    pub linespacing_factor: f64,
    /// Line spacing "exactly" (DXF 73 = 2) rather than "at least" (1, the
    /// default), where lines grow for taller characters.
    pub linespacing_exact: bool,
    pub style: Handle,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Insert {
    pub insertion: Vec3,
    pub scale: Vec3,
    pub rotation: f64,
    pub extrusion: Vec3,
    pub has_attribs: bool,
    pub block_header: Handle,
    pub attribs: Vec<Handle>,
    pub first_attrib: Handle,
    pub last_attrib: Handle,
    pub seqend: Handle,
    /// MINSERT array: columns, rows, column spacing, row spacing.
    pub array: Option<(u16, u16, f64, f64)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Solid {
    pub thickness: f64,
    pub elevation: f64,
    pub corners: [Vec2; 4],
    pub extrusion: Vec3,
    pub is_trace: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Face3d {
    pub corners: [Vec3; 4],
    pub invisible_edges: u16,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spline {
    pub degree: u32,
    pub rational: bool,
    pub closed: bool,
    pub periodic: bool,
    pub knots: Vec<f64>,
    pub control_points: Vec<Vec3>,
    pub weights: Vec<f64>,
    pub fit_points: Vec<Vec3>,
    pub fit_tolerance: f64,
    pub start_tangent: Vec3,
    pub end_tangent: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DimensionKind {
    #[default]
    Linear,
    Aligned,
    Ordinate,
    Angular3Pt,
    Angular2Line,
    Radius,
    Diameter,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dimension {
    pub kind: DimensionKind,
    pub extrusion: Vec3,
    pub text_midpoint: Vec2,
    pub elevation: f64,
    pub flags: u8,
    pub user_text: String,
    pub text_rotation: f64,
    pub horizontal_direction: f64,
    pub insert_scale: Vec3,
    pub insert_rotation: f64,
    pub attachment: u16,
    pub measurement: f64,
    pub clone_insertion: Vec2,
    /// Definition points by DXF group number (10, 13, 14, 15, 16).
    pub pt10: Vec3,
    pub pt13: Vec3,
    pub pt14: Vec3,
    pub pt15: Vec3,
    pub pt16: Vec2,
    pub ext_line_rotation: f64,
    pub dim_rotation: f64,
    pub leader_length: f64,
    pub dimstyle: Handle,
    /// The anonymous "*D" block holding the dimension's graphics.
    pub block: Handle,
}

impl Dimension {
    fn base(kind: DimensionKind) -> Dimension {
        Dimension { kind, extrusion: Vec3::Z, insert_scale: Vec3::new(1.0, 1.0, 1.0), attachment: 5, ..Default::default() }
    }

    /// A linear dimension between `p1` and `p2`, measured along the
    /// direction `rotation` (radians: 0 horizontal, π/2 vertical), with its
    /// dimension line through `line_at`. The text sits in the middle of
    /// the dimension line.
    pub fn linear(p1: Vec2, p2: Vec2, line_at: Vec2, rotation: f64) -> Dimension {
        let u = Vec2::from_angle(rotation);
        let a = line_at + u * (p1 - line_at).dot(u);
        let b = line_at + u * (p2 - line_at).dot(u);
        Dimension {
            pt10: b.extend(0.0),
            pt13: p1.extend(0.0),
            pt14: p2.extend(0.0),
            dim_rotation: rotation,
            text_midpoint: (a + b) * 0.5,
            measurement: (p2 - p1).dot(u).abs(),
            ..Dimension::base(DimensionKind::Linear)
        }
    }

    /// An aligned dimension between `p1` and `p2`, measured along the line
    /// between them, its dimension line `offset` to the left of it
    /// (looking from `p1` to `p2`; negative for the right).
    pub fn aligned(p1: Vec2, p2: Vec2, offset: f64) -> Dimension {
        let n = (p2 - p1).normalized().perp();
        let (a, b) = (p1 + n * offset, p2 + n * offset);
        Dimension {
            pt10: b.extend(0.0),
            pt13: p1.extend(0.0),
            pt14: p2.extend(0.0),
            text_midpoint: (a + b) * 0.5,
            measurement: p1.distance(p2),
            ..Dimension::base(DimensionKind::Aligned)
        }
    }

    /// A diameter dimension across a circle, its line at `angle`
    /// (radians) through the centre.
    pub fn diameter(center: Vec2, radius: f64, angle: f64) -> Dimension {
        let d = Vec2::from_angle(angle) * radius;
        Dimension {
            pt15: (center + d).extend(0.0),
            pt10: (center - d).extend(0.0),
            text_midpoint: center,
            measurement: 2.0 * radius,
            ..Dimension::base(DimensionKind::Diameter)
        }
    }

    /// A radius dimension pointing at the arc at `angle` (radians). The
    /// text goes outside, along that direction (its position is left to
    /// the renderer: a zero `text_midpoint` means "default").
    pub fn radius(center: Vec2, radius: f64, angle: f64) -> Dimension {
        Dimension {
            pt10: center.extend(0.0),
            pt15: (center + Vec2::from_angle(angle) * radius).extend(0.0),
            measurement: radius,
            ..Dimension::base(DimensionKind::Radius)
        }
    }

    /// An angular dimension of the angle at `vertex` between the rays
    /// through `p1` and `p2` (three-point DIMANGULAR), its arc through
    /// `arc_at`. The angle measured is the one on `arc_at`'s side: placed
    /// outside the angle between the rays, the dimension measures the
    /// rest of the turn.
    pub fn angular(vertex: Vec2, p1: Vec2, p2: Vec2, arc_at: Vec2) -> Dimension {
        let mut d = Dimension {
            pt10: arc_at.extend(0.0),
            pt13: p1.extend(0.0),
            pt14: p2.extend(0.0),
            pt15: vertex.extend(0.0),
            ..Dimension::base(DimensionKind::Angular3Pt)
        };
        d.measurement = d.angular_arc().map(|a| a.sweep).unwrap_or(0.0);
        d
    }

    /// An angular dimension between two lines, `a1`–`a2` and `b1`–`b2`
    /// (DIMANGULAR on two lines), its arc through `arc_at`: of the four
    /// angles the lines make where they meet, the one `arc_at` is in.
    /// `None` when the lines are parallel.
    pub fn angular_lines(a1: Vec2, a2: Vec2, b1: Vec2, b2: Vec2, arc_at: Vec2) -> Option<Dimension> {
        let mut d = Dimension {
            pt13: a1.extend(0.0),
            pt14: a2.extend(0.0),
            pt15: b1.extend(0.0),
            pt10: b2.extend(0.0),
            pt16: arc_at,
            ..Dimension::base(DimensionKind::Angular2Line)
        };
        d.measurement = d.angular_arc()?.sweep;
        Some(d)
    }

    /// The arc of an angular dimension: where it is and which angle it
    /// spans. `None` for other kinds, and for degenerate definitions
    /// (parallel lines, points on the vertex).
    pub fn angular_arc(&self) -> Option<AngularArc> {
        use std::f64::consts::{PI, TAU};
        let norm = |a: f64| {
            let a = a % TAU;
            if a < 0.0 { a + TAU } else { a }
        };
        match self.kind {
            DimensionKind::Angular3Pt => {
                let c = self.pt15.xy();
                let (p1, p2, at) = (self.pt13.xy(), self.pt14.xy(), self.pt10.xy());
                let (d1, d2) = (p1.distance(c), p2.distance(c));
                if d1 < 1e-12 || d2 < 1e-12 {
                    return None;
                }
                let (a1, a2) = ((p1 - c).angle(), (p2 - c).angle());
                let s12 = norm(a2 - a1);
                if s12 < 1e-12 {
                    return None;
                }
                let r = at.distance(c);
                let r = if r > 1e-12 { r } else { d1.min(d2) };
                let inside = at.distance(c) < 1e-12 || norm((at - c).angle() - a1) <= s12;
                Some(if inside {
                    AngularArc { center: c, start: a1, sweep: s12, radius: r, legs: [(0.0, d1), (0.0, d2)] }
                } else {
                    AngularArc { center: c, start: a2, sweep: TAU - s12, radius: r, legs: [(0.0, d2), (0.0, d1)] }
                })
            }
            DimensionKind::Angular2Line => {
                let (p, q) = (self.pt13.xy(), self.pt14.xy());
                let (s, e) = (self.pt15.xy(), self.pt10.xy());
                let (u, w) = (q - p, e - s);
                let den = u.cross(w);
                if u.length() < 1e-12 || w.length() < 1e-12 || den.abs() < 1e-12 * u.length() * w.length() {
                    return None;
                }
                let c = p + u * ((s - p).cross(w) / den);
                let at = self.pt16;
                let r = at.distance(c);
                if r < 1e-12 {
                    return None;
                }
                // The four rays from the vertex, by line.
                let (b1, b2) = (u.angle(), w.angle());
                let rays = [(b1, 0usize), (b1 + PI, 0), (b2, 1), (b2 + PI, 1)];
                let la = (at - c).angle();
                // The ray at or before the location going clockwise opens
                // the sector; the next one counter-clockwise closes it.
                let start = rays.iter().min_by(|x, y| norm(la - x.0).total_cmp(&norm(la - y.0))).copied()?;
                let end = rays.iter().filter(|x| x.1 != start.1).min_by(|x, y| norm(x.0 - start.0).total_cmp(&norm(y.0 - start.0))).copied()?;
                let sweep = norm(end.0 - start.0);
                let extent = |line: usize, ang: f64| {
                    let dir = Vec2::from_angle(ang);
                    let (x, y) = if line == 0 { (p, q) } else { (s, e) };
                    let (tx, ty) = ((x - c).dot(dir), (y - c).dot(dir));
                    (tx.min(ty), tx.max(ty))
                };
                Some(AngularArc { center: c, start: norm(start.0), sweep, radius: r, legs: [extent(start.1, start.0), extent(end.1, end.0)] })
            }
            _ => None,
        }
    }
}

/// Where an angular dimension's arc runs ([`Dimension::angular_arc`]):
/// counter-clockwise from `start` through `sweep` (radians) at `radius`
/// about `center`. `legs` gives, for the ray at each end of the arc, how
/// far along it the measured feature runs (nearest and farthest distance
/// from the centre): extension lines bridge the gap from there to the
/// arc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AngularArc {
    pub center: Vec2,
    pub start: f64,
    pub sweep: f64,
    pub radius: f64,
    pub legs: [(f64, f64); 2],
}

impl Line {
    /// A line in the XY plane.
    pub fn new(a: Vec2, b: Vec2) -> Line {
        Line { start: a.extend(0.0), end: b.extend(0.0), thickness: 0.0, extrusion: Vec3::Z }
    }
}

impl Circle {
    /// A circle in the XY plane.
    pub fn new(center: Vec2, radius: f64) -> Circle {
        Circle { center: center.extend(0.0), radius, thickness: 0.0, extrusion: Vec3::Z }
    }
}

impl Arc {
    /// An arc in the XY plane, counter-clockwise from `start` to `end`
    /// (radians).
    pub fn new(center: Vec2, radius: f64, start: f64, end: f64) -> Arc {
        Arc { center: center.extend(0.0), radius, thickness: 0.0, extrusion: Vec3::Z, start_angle: start, end_angle: end }
    }
}

impl LwPolyline {
    /// A polyline of straight segments in the XY plane.
    pub fn new(points: Vec<Vec2>, closed: bool) -> LwPolyline {
        LwPolyline { flags: if closed { Self::FLAG_CLOSED } else { 0 }, extrusion: Vec3::Z, points, ..Default::default() }
    }
}

impl Text {
    /// Single-line text with its left end on the baseline at `position`.
    pub fn new(position: Vec2, height: f64, value: &str) -> Text {
        Text { insertion: position, alignment: position, extrusion: Vec3::Z, height, width_factor: 1.0, value: value.into(), ..Default::default() }
    }

    /// Single-line text aligned at `position`: `h_align` 0 left, 1 centre,
    /// 2 right, 4 middle; `v_align` 0 baseline, 1 bottom, 2 middle, 3 top
    /// (DXF groups 72 and 73).
    pub fn aligned(position: Vec2, height: f64, value: &str, h_align: u16, v_align: u16) -> Text {
        Text { h_align, v_align, ..Text::new(position, height, value) }
    }
}

impl MText {
    /// Multi-line text: `width` 0 for no wrapping; `attachment` 1–9 (top,
    /// middle, bottom rows; left, centre, right columns). Lines break at
    /// `\P`.
    pub fn new(insertion: Vec2, height: f64, width: f64, contents: &str, attachment: u16) -> MText {
        MText {
            insertion: insertion.extend(0.0),
            extrusion: Vec3::Z,
            x_axis: Vec3::X,
            rect_width: width,
            text_height: height,
            attachment,
            drawing_dir: 1,
            contents: contents.into(),
            linespacing_factor: 1.0,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HatchEdge {
    Line { a: Vec2, b: Vec2 },
    Arc { center: Vec2, radius: f64, start: f64, end: f64, ccw: bool },
    Ellipse { center: Vec2, major: Vec2, ratio: f64, start: f64, end: f64, ccw: bool },
    Spline { degree: u32, knots: Vec<f64>, control_points: Vec<Vec2>, weights: Vec<f64> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum HatchPath {
    Edges { flags: u32, edges: Vec<HatchEdge> },
    Polyline { flags: u32, closed: bool, vertices: Vec<(Vec2, f64)> },
}

/// One family of parallel lines of a hatch pattern, in the hatch's own
/// coordinates with its angle and scale already applied (DXF groups 53,
/// 43–46, 79, 49): lines at `angle` through `base`, each the last moved by
/// `offset`, dashed by `dashes` (positive dash, negative gap, zero dot;
/// empty for solid lines).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HatchPatternLine {
    pub angle: f64,
    pub base: Vec2,
    pub offset: Vec2,
    pub dashes: Vec<f64>,
}

/// A program's hatch pattern library as the DWG and DXF writers ask it:
/// given the drawing and a pattern hatch that holds no line families, the
/// line families to store, so that other programs draw the pattern.
pub type HatchLibrary<'a> = dyn Fn(&Drawing, &Hatch) -> Vec<HatchPatternLine> + 'a;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hatch {
    pub elevation: f64,
    pub extrusion: Vec3,
    pub pattern_name: String,
    pub solid_fill: bool,
    pub associative: bool,
    pub is_gradient: bool,
    pub paths: Vec<HatchPath>,
    pub style: u16,
    pub pattern_type: u16,
    pub pattern_angle: f64,
    pub pattern_scale: f64,
    /// A user-defined pattern drawn twice, the second set at 90°.
    pub pattern_double: bool,
    /// The pattern's line families as the file stores them. Empty for a
    /// solid fill, and in drawings saved before format 2.4, where the
    /// renderer takes the pattern from the library by name.
    pub pattern_lines: Vec<HatchPatternLine>,
    /// Points picked inside the boundary when the hatch was made.
    pub seed_points: Vec<Vec2>,
    /// The objects each path was made from, in path order: the boundary of
    /// an associative hatch, which carries the hatch as a persistent
    /// reactor (DWG's boundary object handles, DXF 97 and 330). A path past
    /// the end of the list has none.
    pub sources: Vec<Vec<Handle>>,
}

/// XLINE and RAY: a construction line through `base` along `direction`
/// (WCS, a unit vector), infinite both ways (XLINE) or from `base` on
/// (RAY).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConstructionLine {
    pub base: Vec3,
    pub direction: Vec3,
}

impl ConstructionLine {
    /// A construction line in the XY plane.
    pub fn new(base: Vec2, direction: Vec2) -> ConstructionLine {
        let d = direction.normalized();
        ConstructionLine { base: base.extend(0.0), direction: Vec3::new(d.x, d.y, 0.0) }
    }
}

/// LEADER (ODA spec §20.4.47, DXF LEADER): an arrow and a line through
/// `vertices` to an annotation, usually an MTEXT.
#[derive(Debug, Clone, PartialEq)]
pub struct Leader {
    /// WCS vertices, the arrow at the first.
    pub vertices: Vec<Vec3>,
    /// DXF 71.
    pub arrowhead: bool,
    /// DXF 72: false straight segments, true a spline through the vertices.
    pub spline: bool,
    /// DXF 73: 0 MTEXT, 1 tolerance, 2 block reference, 3 none.
    pub annotation_type: u16,
    /// DXF 74: the hook line runs along the horizontal direction.
    pub hookline_on_x: bool,
    /// DXF 75.
    pub hookline: bool,
    /// DXF 40, 41: the annotation's extents.
    pub text_height: f64,
    pub text_width: f64,
    /// The leader plane's origin (DWG only; the first vertex by default).
    pub origin: Vec3,
    pub extrusion: Vec3,
    /// DXF 211: the "horizontal" direction.
    pub x_direction: Vec3,
    /// DXF 212: offset of the last vertex from a block annotation's
    /// insertion point.
    pub block_offset: Vec3,
    /// DXF 213: offset of the last vertex from the annotation.
    pub annotation_offset: Vec3,
    /// DXF 340: the annotation.
    pub annotation: Handle,
    pub dimstyle: Handle,
    /// Fields DWG stores whose meaning is not documented, kept as read:
    /// the leading bit, then (R2000+) a short and two bits.
    pub unknown: (bool, u16, bool, bool),
}

impl Default for Leader {
    fn default() -> Self {
        Leader {
            vertices: Vec::new(),
            arrowhead: true,
            spline: false,
            annotation_type: 3,
            hookline_on_x: true,
            hookline: false,
            text_height: 0.0,
            text_width: 0.0,
            origin: Vec3::ZERO,
            extrusion: Vec3::Z,
            x_direction: Vec3::X,
            block_offset: Vec3::ZERO,
            annotation_offset: Vec3::ZERO,
            annotation: Handle::NULL,
            dimstyle: Handle::NULL,
            unknown: (false, 0, false, false),
        }
    }
}

/// TOLERANCE (ODA spec §20.4.49, DXF TOLERANCE): a geometric tolerance
/// feature control frame. Its text holds the frame's contents in the GD&T
/// codes the DXF reference describes (`%%v` separates boxes, `^J` starts a
/// new line, `{\Fgdt;…}` symbols); its size, gap and colours come from the
/// dimension style.
#[derive(Debug, Clone, PartialEq)]
pub struct Tolerance {
    /// DXF 10: the middle of the left edge of the first frame line (WCS).
    pub insertion: Vec3,
    /// DXF 11: the frame's X direction (WCS).
    pub x_direction: Vec3,
    /// DXF 210.
    pub extrusion: Vec3,
    /// DXF 1: the frame contents.
    pub text: String,
    /// DXF 3 (by name) / DWG hard pointer: the dimension style.
    pub dimstyle: Handle,
    /// R13 and R14 only: an undocumented short, the text height and the
    /// gap (DIMGAP × DIMSCALE when it was made). Later versions take both
    /// from the dimension style and don't store them; kept as read.
    pub r14: Option<(u16, f64, f64)>,
}

impl Default for Tolerance {
    fn default() -> Self {
        Tolerance { insertion: Vec3::ZERO, x_direction: Vec3::X, extrusion: Vec3::Z, text: String::new(), dimstyle: Handle::NULL, r14: None }
    }
}

/// One vertex of a multiline (ODA spec §20.4.50, DXF MLINE 11, 12, 13,
/// 74, 41, 75, 42).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MLineVertex {
    /// DXF 11 (WCS).
    pub point: Vec3,
    /// DXF 12: the direction of the segment that starts here.
    pub direction: Vec3,
    /// DXF 13: the miter direction at this vertex.
    pub miter: Vec3,
    /// Per element of the style, in style order: the segment parameters
    /// (DXF 41: where the element's line starts along the miter, then
    /// where its breaks start and end along the segment) and the area fill
    /// parameters (DXF 42).
    pub params: Vec<(Vec<f64>, Vec<f64>)>,
}

/// MLINE (ODA spec §20.4.50, DXF MLINE): a multiline, parallel lines drawn
/// with the elements of its MLINESTYLE.
#[derive(Debug, Clone, PartialEq)]
pub struct MLine {
    /// DXF 340 (and 2, by name): the MLINESTYLE.
    pub style: Handle,
    /// DXF 40: element offsets are multiplied by this.
    pub scale: f64,
    /// DXF 70: 0 top, 1 zero, 2 bottom.
    pub justification: u8,
    /// DXF 10: the start point (WCS).
    pub base: Vec3,
    /// DXF 210.
    pub extrusion: Vec3,
    /// DXF 71: 1 has vertices, 2 closed, 4 start caps suppressed, 8 end
    /// caps suppressed. DWG stores the same word ("open 1, closed 3").
    pub flags: u16,
    /// DXF 73: the number of elements of the style when it was drawn.
    pub lines_in_style: u8,
    pub vertices: Vec<MLineVertex>,
}

impl Default for MLine {
    fn default() -> Self {
        MLine {
            style: Handle::NULL,
            scale: 1.0,
            justification: 0,
            base: Vec3::ZERO,
            extrusion: Vec3::Z,
            flags: 1,
            lines_in_style: 0,
            vertices: Vec::new(),
        }
    }
}

/// IMAGE and WIPEOUT (AcDbRasterImage; ODA spec §20.4.80, DXF IMAGE and
/// WIPEOUT): a raster image placed by its pixel vectors, or a wipeout, an
/// image-less raster entity whose clip polygon masks what lies under it.
/// An image's file is its IMAGEDEF object's ([`Self::definition`]).
#[derive(Debug, Clone, PartialEq)]
pub struct RasterImage {
    /// DXF 90.
    pub class_version: u32,
    /// DXF 10: the lower-left corner of the image (WCS).
    pub insertion: Vec3,
    /// DXF 11, 12: one pixel's width (U) and height (V) vectors.
    pub u: Vec3,
    pub v: Vec3,
    /// DXF 13: the image size in pixels.
    pub size: Vec2,
    /// DXF 70: 1 show the image, 2 show it when not aligned with the
    /// screen, 4 use the clip boundary, 8 transparency on.
    pub display: u16,
    /// DXF 280.
    pub clipping: bool,
    /// DXF 281, 282, 283: brightness, contrast and fade, 0 to 100.
    pub brightness: u8,
    pub contrast: u8,
    pub fade: u8,
    /// DXF 290 (R2010+): the inside of the clip boundary is clipped away.
    pub clip_inside: bool,
    /// DXF 71: 1 rectangle (two opposite corners), 2 polygon.
    pub clip_type: u16,
    /// DXF 14: the clip boundary in pixel coordinates.
    pub clip: Vec<Vec2>,
    /// DXF 340: the IMAGEDEF (null for a wipeout).
    pub definition: Handle,
    /// DXF 360: the IMAGEDEF_REACTOR the image owns (null for a wipeout).
    pub reactor: Handle,
}

impl Default for RasterImage {
    fn default() -> Self {
        RasterImage {
            class_version: 0,
            insertion: Vec3::ZERO,
            u: Vec3::X,
            v: Vec3::Y,
            size: Vec2::new(1.0, 1.0),
            display: 7,
            clipping: false,
            brightness: 50,
            contrast: 50,
            fade: 0,
            clip_inside: false,
            clip_type: 1,
            clip: Vec::new(),
            definition: Handle::NULL,
            reactor: Handle::NULL,
        }
    }
}

/// A VIEWPORT entity: a window on a layout that shows model space at a
/// scale (ODA spec §20.4.38, DXF VIEWPORT). The first viewport of each
/// layout (id 1) is the layout's own view of the paper, not a window.
#[derive(Debug, Clone, PartialEq)]
pub struct PaperViewport {
    /// Centre on the paper, and size, in paper units.
    pub center: Vec3,
    pub width: f64,
    pub height: f64,
    /// DXF 69: 1 for the layout's own view; windows count from 2. DWG does
    /// not store it: the reader numbers viewports in block order.
    pub id: i16,
    pub view_target: Vec3,
    pub view_direction: Vec3,
    pub twist: f64,
    /// Height of the model shown, in model units: the scale is
    /// `height / view_height`.
    pub view_height: f64,
    pub lens: f64,
    pub front_clip: f64,
    pub back_clip: f64,
    pub snap_angle: f64,
    /// Model point at the viewport's centre (display coordinates).
    pub view_center: Vec2,
    pub snap_base: Vec2,
    pub snap_spacing: Vec2,
    pub grid_spacing: Vec2,
    pub circle_zoom: u16,
    pub grid_major: u16,
    pub frozen_layers: Vec<Handle>,
    /// DXF 90 status bits (0x20000: switched off).
    pub status: u32,
    pub style_sheet: String,
    pub render_mode: u8,
    pub ucs_at_origin: bool,
    pub ucs_per_viewport: bool,
    pub ucs_origin: Vec3,
    pub ucs_x: Vec3,
    pub ucs_y: Vec3,
    pub ucs_elevation: f64,
    pub ucs_ortho_type: u16,
    pub shade_plot_mode: u16,
    pub default_lights: bool,
    pub lighting_type: u8,
    pub brightness: f64,
    pub contrast: f64,
    pub ambient: Color,
    pub clip_boundary: Handle,
    pub named_ucs: Handle,
    pub base_ucs: Handle,
    pub background: Handle,
    pub visual_style: Handle,
    pub shade_plot: Handle,
    pub sun: Handle,
}

impl PaperViewport {
    /// Status bits a new viewport starts with: on, UCS icon shown, fast
    /// zoom, adaptive grid.
    pub const DEFAULT_STATUS: u32 = 0x8000 | 0x20 | 0x40 | 0x80 | 0x40000 | 0x80000;
    pub const STATUS_OFF: u32 = 0x20000;

    /// A viewport `width` × `height` centred at `center` on the paper,
    /// showing model space around `view_center` at `scale` (paper units
    /// per model unit).
    pub fn new(center: Vec2, width: f64, height: f64, view_center: Vec2, scale: f64, id: i16) -> PaperViewport {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        PaperViewport {
            center: center.extend(0.0),
            width,
            height,
            id,
            view_target: Vec3::ZERO,
            view_direction: Vec3::Z,
            twist: 0.0,
            view_height: height / scale,
            lens: 50.0,
            front_clip: 0.0,
            back_clip: 0.0,
            snap_angle: 0.0,
            view_center,
            snap_base: Vec2::ZERO,
            snap_spacing: Vec2::new(10.0, 10.0),
            grid_spacing: Vec2::new(10.0, 10.0),
            circle_zoom: 1000,
            grid_major: 5,
            frozen_layers: Vec::new(),
            status: Self::DEFAULT_STATUS,
            style_sheet: String::new(),
            render_mode: 0,
            ucs_at_origin: true,
            ucs_per_viewport: true,
            ucs_origin: Vec3::ZERO,
            ucs_x: Vec3::X,
            ucs_y: Vec3::Y,
            ucs_elevation: 0.0,
            ucs_ortho_type: 0,
            shade_plot_mode: 0,
            default_lights: true,
            lighting_type: 1,
            brightness: 0.0,
            contrast: 0.0,
            ambient: Color::Index(250),
            clip_boundary: Handle::NULL,
            named_ucs: Handle::NULL,
            base_ucs: Handle::NULL,
            background: Handle::NULL,
            visual_style: Handle::NULL,
            shade_plot: Handle::NULL,
            sun: Handle::NULL,
        }
    }

    /// Paper units per model unit.
    pub fn scale(&self) -> f64 {
        if self.view_height > 0.0 && self.view_height.is_finite() { self.height / self.view_height } else { 1.0 }
    }

    pub fn is_on(&self) -> bool {
        self.status & Self::STATUS_OFF == 0
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BlockHeader {
    pub name: String,
    pub anonymous: bool,
    pub has_attribs: bool,
    pub is_xref: bool,
    pub is_overlaid: bool,
    pub base_point: Vec3,
    pub xref_path: String,
    pub description: String,
    pub insert_units: u16,
    /// Whether EXPLODE may break its references apart (DXF 280, DWG from
    /// R2007). AutoCAD's blocks are explodable unless set otherwise, so the
    /// model stores the exception.
    pub not_explodable: bool,
    pub block_entity: Handle,
    /// Owned entities in order (R2004+). Empty for R13–R2000, where the
    /// first/last pair and entity owners are used instead.
    pub entities: Vec<Handle>,
    pub first_entity: Handle,
    pub last_entity: Handle,
    pub endblk: Handle,
    pub layout: Handle,
    /// What the block is as a manufactured part, when it is one (the
    /// model space record holds it for a drawing of a single part).
    pub part: Part,
}

/// A block as a manufactured part: the bill-of-materials data a drawing
/// carries (Proposed, see FORMAT.md). Empty strings and zeros are unset.
/// Material, grade and thickness describe what the part is made of; how a
/// machine cuts it is not part of the drawing (DESIGN §1).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Part {
    /// For example "Steel", "Stainless steel", "Aluminium".
    pub material: String,
    /// For example "S355JR", "304", "5083-H111".
    pub grade: String,
    /// Sheet or plate thickness in drawing units; 0 unset.
    pub thickness: f64,
    /// How many to make; 0 unset.
    pub quantity: u32,
}

impl Part {
    pub fn is_empty(&self) -> bool {
        *self == Part::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    /// Raw flag word (R2000+) or reconstructed from the R13–R14 bits.
    pub flags: u16,
    pub frozen: bool,
    pub off: bool,
    pub locked: bool,
    pub plot: bool,
    pub color: Color,
    pub linetype: Handle,
    /// Lineweight code as DWG stores it (see [`Lineweight`]).
    pub lineweight: u8,
}

impl Default for Layer {
    /// An unnamed layer that is on, thawed, unlocked and plots: white,
    /// no linetype record, the default lineweight.
    fn default() -> Self {
        Layer {
            name: String::new(),
            flags: 0,
            frozen: false,
            off: false,
            locked: false,
            plot: true,
            color: Color::Index(7),
            linetype: Handle::NULL,
            lineweight: Lineweight::CODE_DEFAULT,
        }
    }
}

impl Layer {
    /// The lineweight code, interpreted.
    pub fn weight(&self) -> Lineweight {
        Lineweight::from_code(self.lineweight)
    }

    /// The flag word as R2000+ DWG lays it out (ODA spec §20.4.54), from
    /// the fields: 0x01 frozen, 0x02 off (the bit the spec calls "on"; see
    /// `dwg::tables::layer`), 0x08 locked, 0x10 plots, bits 5–9
    /// the lineweight code. Other bits of the stored word (0x04, frozen in
    /// new viewports) are kept.
    pub fn dwg_flags(&self) -> u16 {
        let kept = self.flags & !(0x01 | 0x02 | 0x08 | 0x10 | 0x03E0);
        kept | (self.frozen as u16)
            | ((self.off as u16) << 1)
            | ((self.locked as u16) << 3)
            | ((self.plot as u16) << 4)
            | (((self.lineweight as u16) & 0x1F) << 5)
    }
}

/// One element of a linetype pattern (DXF groups 49 and 74–75, 340, 46,
/// 50, 44–45, 9).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinetypeElement {
    /// Positive: a dash of that length. Negative: a gap. Zero: a dot.
    pub length: f64,
    /// Complex element flags (DXF 74): 0 for a plain dash; 1 absolute
    /// rotation, 2 text, 4 shape.
    pub complex: u16,
    /// Shape number (DXF 75), for shape elements.
    pub shape: u16,
    /// Text style or shape file (DXF 340), for complex elements.
    pub style: Handle,
    /// Scale of the text or shape (DXF 46).
    pub scale: f64,
    /// Rotation of the text or shape, radians (DXF 50).
    pub rotation: f64,
    /// Offset of the text or shape (DXF 44, 45).
    pub offset: Vec2,
    /// The text of a text element (DXF 9).
    pub text: String,
}

impl LinetypeElement {
    /// A plain dash (positive), gap (negative) or dot (zero).
    pub fn plain(length: f64) -> Self {
        LinetypeElement { length, scale: 1.0, ..Default::default() }
    }
}

/// A linetype table record (LTYPE) with its pattern.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Linetype {
    pub name: String,
    pub description: String,
    /// DXF 70 flags.
    pub flags: u16,
    /// Total pattern length as stored (DXF 40): the sum of the element
    /// lengths' magnitudes.
    pub pattern_length: f64,
    /// Elements in order. Empty for a continuous line.
    pub elements: Vec<LinetypeElement>,
}

impl Linetype {
    /// A plain dash-and-gap linetype.
    pub fn simple(name: &str, description: &str, lengths: &[f64]) -> Self {
        Linetype {
            name: name.into(),
            description: description.into(),
            flags: 0,
            pattern_length: lengths.iter().map(|l| l.abs()).sum(),
            elements: lengths.iter().map(|&l| LinetypeElement::plain(l)).collect(),
        }
    }

    /// The dash lengths (positive dash, negative gap, zero dot).
    pub fn dashes(&self) -> Vec<f64> {
        self.elements.iter().map(|e| e.length).collect()
    }
}

/// A text style table record (STYLE).
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub name: String,
    /// DXF 70 flags: 1 shape file, 4 vertical text.
    pub flags: u16,
    /// Fixed text height, or 0 when the height is given per text.
    pub height: f64,
    pub width_factor: f64,
    /// Oblique angle, radians.
    pub oblique: f64,
    /// DXF 71 generation flags: 2 backwards, 4 upside down.
    pub generation: u16,
    /// The last height used (DXF 42).
    pub last_height: f64,
    /// Font file name, e.g. `simplex.shx` or `arial.ttf`.
    pub font_file: String,
    /// Big-font file name, for Asian scripts.
    pub bigfont_file: String,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            name: String::new(),
            flags: 0,
            height: 0.0,
            width_factor: 1.0,
            oblique: 0.0,
            generation: 0,
            last_height: 0.0,
            font_file: String::new(),
            bigfont_file: String::new(),
        }
    }
}

/// A dimension style table record (DIMSTYLE): the dimension variables
/// the core uses. Each field names its DXF group and variable.
#[derive(Debug, Clone, PartialEq)]
pub struct DimStyle {
    pub name: String,
    /// 70: flags.
    pub flags: u16,
    /// 3 DIMPOST: text around the measurement, `<>` standing for it.
    pub post_text: String,
    /// 40 DIMSCALE: overall scale of sizes (text, arrows, gaps).
    pub scale: f64,
    /// 41 DIMASZ: arrowhead length.
    pub arrow_size: f64,
    /// 42 DIMEXO: gap between the object and an extension line.
    pub ext_offset: f64,
    /// 43 DIMDLI: spacing of stacked (baseline) dimensions.
    pub baseline_spacing: f64,
    /// 44 DIMEXE: how far extension lines run past the dimension line.
    pub ext_extension: f64,
    /// 45 DIMRND: rounding of measurements (0 none).
    pub rounding: f64,
    /// 46 DIMDLE: dimension line past the extension lines (with ticks).
    pub dim_line_extension: f64,
    /// 140 DIMTXT: text height.
    pub text_height: f64,
    /// 141 DIMCEN: centre mark size (0 none, negative with centre lines).
    pub center_mark: f64,
    /// 142 DIMTSZ: oblique tick size instead of arrowheads (0 arrows).
    pub tick_size: f64,
    /// 143 DIMALTF: alternate-unit factor.
    pub alt_factor: f64,
    /// 144 DIMLFAC: measurement scale factor.
    pub linear_factor: f64,
    /// 147 DIMGAP: gap around the text where it breaks the line.
    pub text_gap: f64,
    /// 73 DIMTIH: text inside the extension lines reads horizontally.
    pub text_inside_horizontal: bool,
    /// 74 DIMTOH: text outside the extension lines reads horizontally.
    pub text_outside_horizontal: bool,
    /// 77 DIMTAD: 0 text centred in a break of the dimension line,
    /// 1 above it.
    pub text_above: u16,
    /// 78 DIMZIN: zero suppression flags.
    pub zero_suppression: u16,
    /// 170 DIMALT: show alternate units.
    pub alt_units: bool,
    /// 171 DIMALTD: decimal places of alternate units.
    pub alt_decimals: u16,
    /// DIMCLRD: dimension line and arrow colour.
    pub line_color: Color,
    /// DIMCLRE: extension line colour.
    pub ext_color: Color,
    /// DIMCLRT: text colour.
    pub text_color: Color,
    /// 271 DIMDEC: decimal places.
    pub decimals: u16,
    /// 276 DIMFRAC: fraction format (0 horizontal, 1 diagonal, 2 inline).
    pub fraction_format: u16,
    /// 277 DIMLUNIT: unit format (1 scientific, 2 decimal, 3 engineering,
    /// 4 architectural, 5 fractional).
    pub unit_format: u16,
    /// 278 DIMDSEP: decimal separator character.
    pub decimal_separator: char,
    /// 280 DIMJUST: horizontal text position.
    pub text_justify: u16,
    /// 340 DIMTXSTY: text style.
    pub text_style: Handle,
    /// 371 DIMLWD: dimension line weight, as a DWG code.
    pub line_weight: u8,
    /// 372 DIMLWE: extension line weight, as a DWG code.
    pub ext_weight: u8,
}

impl Default for DimStyle {
    /// Neutral values: sizes 0 (the renderer then uses the standard style
    /// for the drawing's units), colours and weights by block.
    fn default() -> Self {
        DimStyle {
            name: String::new(),
            flags: 0,
            post_text: String::new(),
            scale: 1.0,
            arrow_size: 0.0,
            ext_offset: 0.0,
            baseline_spacing: 0.0,
            ext_extension: 0.0,
            rounding: 0.0,
            dim_line_extension: 0.0,
            text_height: 0.0,
            center_mark: 0.0,
            tick_size: 0.0,
            alt_factor: 25.4,
            linear_factor: 1.0,
            text_gap: 0.0,
            text_inside_horizontal: true,
            text_outside_horizontal: true,
            text_above: 0,
            zero_suppression: 0,
            alt_units: false,
            alt_decimals: 2,
            line_color: Color::ByBlock,
            ext_color: Color::ByBlock,
            text_color: Color::ByBlock,
            decimals: 2,
            fraction_format: 0,
            unit_format: 2,
            decimal_separator: '.',
            text_justify: 0,
            text_style: Handle::NULL,
            line_weight: Lineweight::CODE_BY_BLOCK,
            ext_weight: Lineweight::CODE_BY_BLOCK,
        }
    }
}

/// A symbol-table control object (BLOCK_CONTROL, LAYER_CONTROL, …).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Control {
    pub entries: Vec<Handle>,
    /// BLOCK_CONTROL only.
    pub model_space: Handle,
    pub paper_space: Handle,
}

/// A table record the core keeps only the name of: VIEW, UCS, VPORT,
/// APPID, … and, from DWG files, DIMSTYLE (not decoded yet) and any LTYPE
/// or STYLE record the reader could not decode. New drawings store those
/// as [`Linetype`], [`TextStyle`] and [`DimStyle`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NamedRecord {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectKind {
    Line(Line),
    Circle(Circle),
    Arc(Arc),
    Point(Point),
    Ellipse(Ellipse),
    LwPolyline(LwPolyline),
    Polyline(Polyline),
    Vertex(Vertex),
    SeqEnd,
    Text(Text),
    Attribute(Attribute),
    MText(MText),
    Insert(Insert),
    Solid(Solid),
    Face3d(Face3d),
    Spline(Spline),
    Dimension(Dimension),
    Hatch(Hatch),
    /// XLINE: a construction line, infinite both ways.
    Xline(ConstructionLine),
    /// RAY: a construction line from its base point on.
    Ray(ConstructionLine),
    Leader(Leader),
    Tolerance(Tolerance),
    MLine(MLine),
    /// IMAGE.
    Image(Box<RasterImage>),
    /// WIPEOUT.
    Wipeout(Box<RasterImage>),
    /// A VIEWPORT entity on a layout.
    PaperViewport(Box<PaperViewport>),
    /// BLOCK entity that opens a block definition.
    BlockBegin {
        name: String,
    },
    BlockEnd,
    BlockHeader(BlockHeader),
    Layer(Layer),
    Control(Control),
    NamedRecord(NamedRecord),
    Linetype(Linetype),
    TextStyle(TextStyle),
    DimStyle(DimStyle),
    /// A VPORT table record whole.
    Viewport(Box<Vport>),
    /// A DIMSTYLE table record with every variable (as DWG and DXF files
    /// store them); [`DimStyle`] is the style a new drawing starts with.
    DimStyleRecord(Box<DimStyleRecord>),
    /// A non-graphical object: dictionary, extension record, layout, …
    NonGraphic(NonGraphic),
    /// Recognised type the core does not decode yet.
    Unsupported,
    /// Decoding failed; the message says where.
    Failed(String),
    /// A kind of native record from a newer revision of the `.cad` format.
    /// The number is its kind code; its fields are kept, undecoded, in
    /// [`Object::unknown_tags`] so a save writes them back unchanged. Only
    /// readers create this, and never with a code this version knows.
    NewerKind(i16),
    /// A native record whose kind fields could not be decoded (damage, or
    /// a writer that broke the format); the message says why. Everything
    /// from where the kind tag should be is kept in
    /// [`Object::unknown_tags`], and a save writes it back as it was, so
    /// the next reader sees and reports the same problem. Only readers
    /// create this.
    Damaged(String),
}

impl ObjectKind {
    pub fn is_decoded(&self) -> bool {
        !matches!(self, ObjectKind::Unsupported | ObjectKind::Failed(_) | ObjectKind::NewerKind(_) | ObjectKind::Damaged(_))
    }
}

/// Extended data (DXF "XDATA", DWG "EED"): values an application attaches
/// to an object, under the application's registered name. An object can
/// carry one block per application.
///
/// The value kinds are DXF's group codes 1000–1071, with their meaning
/// from the DXF reference. The model sets no size limits; AutoCAD allows
/// 16 KB of extended data per object, 255-byte strings and 127-byte binary
/// chunks, and a DXF or DWG writer has to respect those.
#[derive(Debug, Clone, PartialEq)]
pub struct XData {
    /// The registered application (group 1001).
    pub app: XDataApp,
    pub items: Vec<XValue>,
}

impl XData {
    /// The application's registered name, when known.
    pub fn app_name(&self) -> Option<&str> {
        match &self.app {
            XDataApp::Name(n) => Some(n),
            XDataApp::Handle(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XDataApp {
    /// The registered application name, as in its APPID table record.
    Name(String),
    /// The APPID table record by handle, kept this way only when its name
    /// is not known (the source file refers to a record that is missing or
    /// could not be read). DWG stores every application this way; readers
    /// turn handles into names when the record exists.
    Handle(Handle),
}

/// One extended-data value. Each variant names its DXF group code.
#[derive(Debug, Clone, PartialEq)]
pub enum XValue {
    /// 1000: a string.
    Str(String),
    /// 1002 `{`: opens a nested list.
    Open,
    /// 1002 `}`: closes a nested list.
    Close,
    /// 1003: a layer, by handle (as DWG stores it; DXF writes the layer's
    /// name). It follows renames.
    Layer(Handle),
    /// 1004: binary data.
    Bytes(Vec<u8>),
    /// 1005: a handle of another object. Translated when objects are
    /// copied into another drawing.
    Handle(Handle),
    /// 1010: three reals. Editing never changes them.
    Point(Vec3),
    /// 1011: a world position. Moved, scaled, rotated, mirrored and
    /// stretched along with the object.
    Position(Vec3),
    /// 1012: a world displacement. Scaled, rotated and mirrored along with
    /// the object, but not moved or stretched.
    Displacement(Vec3),
    /// 1013: a world direction. Rotated and mirrored along with the object,
    /// but not moved, scaled or stretched.
    Direction(Vec3),
    /// 1040: a real.
    Real(f64),
    /// 1041: a distance. Scaled along with the object.
    Distance(f64),
    /// 1042: a scale factor. Scaled along with the object.
    ScaleFactor(f64),
    /// 1070: a 16-bit integer.
    Int16(i16),
    /// 1071: a 32-bit integer.
    Int32(i32),
    /// DWG extended data that could not be decoded: the bytes from the item
    /// that failed to the end of the application's data, exactly as stored,
    /// with the version of the file they came from (string items are
    /// encoded differently before and after R2007).
    Undecoded { version: DwgVersion, data: Vec<u8> },
}

/// One object from the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub handle: Handle,
    pub type_code: u16,
    /// "LINE", "LAYER", or a class DXF name for custom types.
    pub type_name: String,
    pub owner: Handle,
    pub reactors: Vec<Handle>,
    pub xdictionary: Handle,
    /// Present for entities.
    pub entity: Option<EntityCommon>,
    pub kind: ObjectKind,
    /// Where the object starts (file offset for R13–R15, offset in the
    /// AcDb:AcDbObjects section for R2004+).
    pub offset: usize,
    /// Size of the object data in bytes.
    pub size: usize,
    /// Non-fatal decode problem (for example an unreadable handle stream).
    pub note: Option<String>,
    /// The object's original DWG bytes, kept when the core could not decode
    /// it (`Unsupported` or `Failed`), or kept only its name (a
    /// `NamedRecord`), so a save does not lose it. Those
    /// bytes include the object's extended data, which `xdata` holds
    /// decoded; a DWG writer that writes `raw` must rebuild that part if
    /// `xdata` was changed.
    pub raw: Option<RawDwgObject>,
    /// Extended data (XDATA), one block per application, in stored order.
    pub xdata: Vec<XData>,
    /// Fields written by a newer revision of the native format, which this
    /// version does not read: encoded `.cad` tags, kept exactly as found so
    /// that saving the object writes them back after the kind's fields.
    /// Empty for objects that did not come from a newer writer.
    ///
    /// For a `Damaged` object they hold everything from where the kind tag
    /// should be. Code that replaces an object's kind should clear them.
    /// The writer does not check them: they must be whole tags with no
    /// extended-data codes (1000–1071).
    pub unknown_tags: Vec<u8>,
}

impl Object {
    /// Bytes the object holds, roughly: for budgeting the undo history,
    /// not for exact accounting. Counts the parts that grow (lists of
    /// points, text, kept file bytes).
    pub fn approx_bytes(&self) -> usize {
        use std::mem::size_of;
        let kind = match &self.kind {
            ObjectKind::LwPolyline(p) => p.points.len() * size_of::<Vec2>() + p.bulges.len() * 8 + p.widths.len() * 16 + p.vertex_ids.len() * 4,
            ObjectKind::Text(t) => t.value.len(),
            ObjectKind::MText(t) => t.contents.len(),
            ObjectKind::Spline(sp) => sp.knots.len() * 8 + sp.weights.len() * 8 + (sp.control_points.len() + sp.fit_points.len()) * size_of::<Vec3>(),
            ObjectKind::Hatch(h) => h.paths.len() * 256 + h.pattern_lines.len() * 64,
            ObjectKind::Leader(l) => l.vertices.len() * size_of::<Vec3>(),
            ObjectKind::Tolerance(t) => t.text.len(),
            ObjectKind::MLine(m) => {
                m.vertices.iter().map(|v| size_of::<MLineVertex>() + v.params.iter().map(|(a, b)| 48 + (a.len() + b.len()) * 8).sum::<usize>()).sum()
            }
            ObjectKind::Image(r) | ObjectKind::Wipeout(r) => size_of::<RasterImage>() + r.clip.len() * size_of::<Vec2>(),
            ObjectKind::PaperViewport(v) => size_of::<PaperViewport>() + v.frozen_layers.len() * 8,
            ObjectKind::BlockHeader(b) => b.entities.len() * size_of::<Handle>(),
            ObjectKind::Failed(m) | ObjectKind::Damaged(m) => m.len(),
            _ => 0,
        };
        size_of::<Object>()
            + kind
            + self.type_name.len()
            + self.reactors.len() * size_of::<Handle>()
            + self.note.as_ref().map_or(0, String::len)
            + self.raw.as_ref().map_or(0, |r| r.data.len())
            + self.xdata.iter().map(|x| 32 + x.items.len() * 48).sum::<usize>()
            + self.unknown_tags.len()
    }
}

/// An object's undecoded DWG record, exactly as stored in the source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDwgObject {
    /// Version of the file the bytes came from; they are only meaningful
    /// to a reader or writer of that version.
    pub version: DwgVersion,
    /// Size of the handle stream in bits (R2010+), else 0.
    pub handle_stream_bits: u64,
    /// The object data that follows the size prefix (without the CRC).
    pub data: Vec<u8>,
}

impl Object {
    pub fn is_entity(&self) -> bool {
        self.entity.is_some()
    }

    /// The extended data an application attached to this object. Names
    /// compare without regard to ASCII case, as AutoCAD compares them.
    pub fn xdata_for(&self, app: &str) -> Option<&XData> {
        self.xdata.iter().find(|x| x.app_name().is_some_and(|n| n.eq_ignore_ascii_case(app)))
    }
}

/// A custom class declared in the file (object types 500 and up).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClassDef {
    pub number: u16,
    pub proxy_flags: u16,
    pub app_name: String,
    pub cpp_name: String,
    pub dxf_name: String,
    pub was_zombie: bool,
    /// 0x1F2 for entity classes, 0x1F3 for object classes.
    pub item_class_id: u16,
    /// Objects of the class the drawing has made (DXF 91); 0 unknown.
    pub instance_count: u32,
    /// The DWG version and maintenance release the class came with, as
    /// R2004+ files record them; 0 unknown.
    pub dwg_version: u32,
    pub maintenance_version: u32,
}

impl ClassDef {
    pub fn is_entity(&self) -> bool {
        self.item_class_id == 0x1F2
    }
}

/// The header variables the core uses, plus whether the header parse was
/// cross-checked against the object map.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HeaderVars {
    /// True when the parse was validated (control-object handles resolved
    /// to objects of the right type). Unverified values should not be
    /// trusted.
    pub verified: bool,
    pub insunits: Option<u16>,
    pub measurement: Option<u16>,
    pub extmin: Option<Vec3>,
    pub extmax: Option<Vec3>,
    pub limmin: Option<Vec2>,
    pub limmax: Option<Vec2>,
    pub insbase: Option<Vec3>,
    pub ltscale: Option<f64>,
    pub textsize: Option<f64>,
    /// Linear display format (`$LUNITS`): 1 scientific, 2 decimal,
    /// 3 engineering, 4 architectural, 5 fractional.
    pub lunits: Option<u16>,
    /// Linear display precision (`$LUPREC`): decimal places, or for
    /// fractions the power of two of the smallest denominator.
    pub luprec: Option<u16>,
    /// Angle display format (`$AUNITS`): 0 decimal degrees, 1 degrees,
    /// minutes and seconds, 2 grads, 3 radians, 4 surveyor's units.
    pub aunits: Option<u16>,
    /// Angle display precision (`$AUPREC`).
    pub auprec: Option<u16>,
    /// Direction of angle 0, radians (`$ANGBASE`).
    pub angbase: Option<f64>,
    /// 1 when angles run clockwise (`$ANGDIR`).
    pub angdir: Option<u16>,
    /// Linetype scale given to new entities (`$CELTSCALE`).
    pub celtscale: Option<f64>,
    pub handseed: Option<u64>,
    pub clayer: Handle,
    /// Current text style (`$TEXTSTYLE`).
    pub textstyle: Handle,
    /// Linetype given to new entities (`$CELTYPE`): an LTYPE record,
    /// normally the one named ByLayer.
    pub celtype: Handle,
    /// Current dimension style (`$DIMSTYLE`).
    pub dimstyle: Handle,
    pub block_control: Handle,
    pub layer_control: Handle,
    pub style_control: Handle,
    pub linetype_control: Handle,
    pub dimstyle_control: Handle,
    pub model_space: Handle,
    pub paper_space: Handle,
    pub named_objects_dict: Handle,
    /// Every variable the parser read, in file order, as display text.
    pub raw: Vec<(String, String)>,
    /// Why the parse was not verified, if it was not.
    pub note: String,
    /// Drawing properties: title, author and the like.
    pub properties: Properties,
    /// Variables written by a newer revision of the native format, which
    /// this version does not read: encoded `.cad` tags, kept exactly as
    /// found so that saving writes them back.
    pub unknown_tags: Vec<u8>,
}

/// Drawing properties, the fields of the "drawing properties" dialog of
/// CAD programs (DWG keeps them in its summary information). Empty
/// strings and zero times are unset.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Properties {
    pub title: String,
    pub subject: String,
    pub author: String,
    pub keywords: String,
    pub comments: String,
    pub last_saved_by: String,
    pub revision: String,
    /// Base for relative hyperlinks in the drawing.
    pub hyperlink_base: String,
    /// Unix seconds.
    pub created: i64,
    pub modified: i64,
    /// Name and value pairs, in order; names are unique.
    pub custom: Vec<(String, String)>,
}

impl Properties {
    pub fn is_empty(&self) -> bool {
        *self == Properties::default()
    }

    /// The named fields as (label, value), for display and the CLI.
    pub fn fields(&self) -> [(&'static str, &str); 8] {
        [
            ("title", &self.title),
            ("subject", &self.subject),
            ("author", &self.author),
            ("keywords", &self.keywords),
            ("comments", &self.comments),
            ("last-saved-by", &self.last_saved_by),
            ("revision", &self.revision),
            ("hyperlink-base", &self.hyperlink_base),
        ]
    }

    /// Set a named field (one of [`Properties::fields`]) or, for any other
    /// name, a custom property; an empty value removes a custom property.
    pub fn set(&mut self, name: &str, value: &str) {
        let v = value.to_string();
        match name {
            "title" => self.title = v,
            "subject" => self.subject = v,
            "author" => self.author = v,
            "keywords" => self.keywords = v,
            "comments" => self.comments = v,
            "last-saved-by" => self.last_saved_by = v,
            "revision" => self.revision = v,
            "hyperlink-base" => self.hyperlink_base = v,
            _ => {
                let at = self.custom.iter().position(|(k, _)| k == name);
                match (at, value.is_empty()) {
                    (Some(i), true) => {
                        self.custom.remove(i);
                    }
                    (Some(i), false) => self.custom[i].1 = v,
                    (None, false) => self.custom.push((name.to_string(), v)),
                    (None, true) => {}
                }
            }
        }
    }
}

/// Name of an `INSUNITS` value (drawing units), per the DXF reference.
pub fn units_name(insunits: u16) -> &'static str {
    match insunits {
        0 => "unitless",
        1 => "inches",
        2 => "feet",
        3 => "miles",
        4 => "millimeters",
        5 => "centimeters",
        6 => "meters",
        7 => "kilometers",
        8 => "microinches",
        9 => "mils",
        10 => "yards",
        11 => "angstroms",
        12 => "nanometers",
        13 => "microns",
        14 => "decimeters",
        15 => "decameters",
        16 => "hectometers",
        17 => "gigameters",
        18 => "astronomical units",
        19 => "light years",
        20 => "parsecs",
        21 => "US survey feet",
        _ => "unknown",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
}

/// Summary of one stored section (R2004+) or section locator (R13–R15).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SectionInfo {
    pub name: String,
    pub size: u64,
    pub pages: u32,
    pub compressed: bool,
    pub encrypted: bool,
}

/// Lookup tables built once after loading (see [`Drawing::rebuild_index`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DrawingIndex {
    pub model_space: Handle,
    pub paper_space: Handle,
    /// Object handles grouped by owner, in handle order. Model- and
    /// paper-space entities (which files store without an owner handle) are
    /// grouped under the space's block record.
    pub owned: BTreeMap<Handle, Vec<Handle>>,
}

/// A loaded drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawing {
    pub version: DwgVersion,
    pub maintenance_version: u8,
    pub codepage: u16,
    pub header: HeaderVars,
    pub classes: Vec<ClassDef>,
    pub objects: ObjectMap,
    pub sections: Vec<SectionInfo>,
    pub diagnostics: Vec<Diagnostic>,
    pub index: DrawingIndex,
}

impl Drawing {
    /// Record a save the way AutoCAD does when it writes a file: who saved
    /// it (the drawing properties' "last saved by") and when (TDUPDATE and
    /// TDUUPDATE, and TDCREATE and TDUCREATE if the drawing has no creation
    /// time yet). `unix_seconds` is UTC; with no time zone at hand the
    /// local times are given in UTC too.
    pub fn stamp_save(&mut self, saved_by: &str, unix_seconds: i64) {
        let julian = 2_440_588.0 + unix_seconds as f64 / 86_400.0;
        let text = format!("{julian}");
        let set = |raw: &mut Vec<(String, String)>, name: &str, value: &str| match raw.iter_mut().find(|(k, _)| k == name) {
            Some(e) => e.1 = value.to_string(),
            None => raw.push((name.to_string(), value.to_string())),
        };
        let created = self.header.raw.iter().find(|(k, _)| k == "TDCREATE").and_then(|(_, v)| v.parse::<f64>().ok()).is_some_and(|v| v > 0.0);
        if !created {
            set(&mut self.header.raw, "TDCREATE", &text);
            set(&mut self.header.raw, "TDUCREATE", &text);
            self.header.properties.created = unix_seconds;
        }
        set(&mut self.header.raw, "TDUPDATE", &text);
        set(&mut self.header.raw, "TDUUPDATE", &text);
        self.header.properties.last_saved_by = saved_by.to_string();
        self.header.properties.modified = unix_seconds;
    }

    pub fn get(&self, h: Handle) -> Option<&Object> {
        self.objects.get(&h)
    }

    pub fn layer(&self, h: Handle) -> Option<&Layer> {
        match &self.get(h)?.kind {
            ObjectKind::Layer(l) => Some(l),
            _ => None,
        }
    }

    /// A linetype record with its pattern. A DWG linetype the reader could
    /// not decode keeps only its name (see [`NamedRecord`]); use
    /// [`Drawing::record_name`] for any.
    pub fn linetype(&self, h: Handle) -> Option<&Linetype> {
        match &self.get(h)?.kind {
            ObjectKind::Linetype(l) => Some(l),
            _ => None,
        }
    }

    pub fn text_style(&self, h: Handle) -> Option<&TextStyle> {
        match &self.get(h)?.kind {
            ObjectKind::TextStyle(s) => Some(s),
            _ => None,
        }
    }

    pub fn dim_style(&self, h: Handle) -> Option<&DimStyle> {
        match &self.get(h)?.kind {
            ObjectKind::DimStyle(s) => Some(s),
            _ => None,
        }
    }

    /// The name of a table record of any kind (layer, linetype, style,
    /// block, …).
    pub fn record_name(&self, h: Handle) -> Option<&str> {
        Some(match &self.get(h)?.kind {
            ObjectKind::Layer(l) => &l.name,
            ObjectKind::Linetype(l) => &l.name,
            ObjectKind::TextStyle(s) => &s.name,
            ObjectKind::DimStyle(s) => &s.name,
            ObjectKind::BlockHeader(b) => &b.name,
            ObjectKind::NamedRecord(r) => &r.name,
            ObjectKind::Viewport(v) => &v.name,
            ObjectKind::DimStyleRecord(r) => &r.name,
            _ => return None,
        })
    }

    pub fn block_header(&self, h: Handle) -> Option<&BlockHeader> {
        match &self.get(h)?.kind {
            ObjectKind::BlockHeader(b) => Some(b),
            _ => None,
        }
    }

    pub fn block_by_name(&self, name: &str) -> Option<(Handle, &BlockHeader)> {
        self.objects.values().find_map(|o| match &o.kind {
            ObjectKind::BlockHeader(b) if b.name.eq_ignore_ascii_case(name) => Some((o.handle, b)),
            _ => None,
        })
    }

    /// Find a space's block record: verified header first, then the
    /// BLOCK_CONTROL object, then by name.
    fn find_space(&self, from_header: Handle, paper: bool) -> Handle {
        if self.header.verified && self.block_header(from_header).is_some() {
            return from_header;
        }
        for o in self.objects.values() {
            if let ObjectKind::Control(c) = &o.kind {
                let h = if paper { c.paper_space } else { c.model_space };
                if !h.is_null() && self.block_header(h).is_some() {
                    return h;
                }
            }
        }
        let name = if paper { "*Paper_Space" } else { "*Model_Space" };
        self.block_by_name(name).map(|(h, _)| h).unwrap_or(Handle::NULL)
    }

    /// Recompute the lookup tables. Call after changing `objects`.
    pub fn rebuild_index(&mut self) {
        let model_space = self.find_space(self.header.model_space, false);
        let paper_space = self.find_space(self.header.paper_space, true);
        let mut owned: BTreeMap<Handle, Vec<Handle>> = BTreeMap::new();
        for o in self.objects.values() {
            let key = match o.entity.as_ref().map(|e| e.space) {
                Some(Space::Model) => model_space,
                Some(Space::Paper) => paper_space,
                _ => o.owner,
            };
            if !key.is_null() {
                owned.entry(key).or_default().push(o.handle);
            }
        }
        self.index = DrawingIndex { model_space, paper_space, owned };
    }

    /// Handle of the *Model_Space block record.
    pub fn model_space(&self) -> Handle {
        self.index.model_space
    }

    /// Handle of the active *Paper_Space block record.
    pub fn paper_space(&self) -> Handle {
        self.index.paper_space
    }

    /// Objects owned by `owner` (in handle order).
    pub fn owned_by(&self, owner: Handle) -> impl Iterator<Item = &Object> {
        self.index.owned.get(&owner).into_iter().flatten().filter_map(move |h| self.get(*h))
    }

    /// Entities belonging to a block record, in drawing order where the file
    /// records it (R2004+), otherwise in handle order.
    pub fn block_entities(&self, block: Handle) -> Vec<&Object> {
        let Some(bh) = self.block_header(block) else {
            return Vec::new();
        };
        if !bh.entities.is_empty() {
            let listed: Vec<&Object> = bh.entities.iter().filter_map(|h| self.get(*h)).filter(|o| o.is_entity()).collect();
            if !listed.is_empty() {
                return listed;
            }
        }
        self.owned_by(block).filter(|o| o.is_entity()).filter(|o| !matches!(o.kind, ObjectKind::BlockBegin { .. } | ObjectKind::BlockEnd)).collect()
    }

    pub fn model_space_entities(&self) -> Vec<&Object> {
        let ms = self.model_space();
        if !ms.is_null() {
            return self.block_entities(ms);
        }
        // No block record found: fall back to entity modes alone.
        self.objects.values().filter(|o| o.entity.as_ref().map(|e| e.space == Space::Model).unwrap_or(false)).collect()
    }

    /// Vertices of an old-style POLYLINE, in order.
    pub fn polyline_vertices(&self, polyline: &Object) -> Vec<&Vertex> {
        let ObjectKind::Polyline(pl) = &polyline.kind else {
            return Vec::new();
        };
        fn vertex(o: &Object) -> Option<&Vertex> {
            match &o.kind {
                ObjectKind::Vertex(v) => Some(v),
                _ => None,
            }
        }
        let from_list: Vec<&Vertex> = pl.vertices.iter().filter_map(|h| self.get(*h)).filter_map(vertex).collect();
        if !from_list.is_empty() {
            return from_list;
        }
        // R13–R2000: vertices are owned by the polyline; handles ascend in
        // creation order.
        self.owned_by(polyline.handle).filter_map(vertex).collect()
    }

    /// Attributes attached to an INSERT.
    pub fn insert_attributes(&self, insert: &Object) -> Vec<&Object> {
        let ObjectKind::Insert(ins) = &insert.kind else {
            return Vec::new();
        };
        let listed: Vec<&Object> = ins.attribs.iter().filter_map(|h| self.get(*h)).collect();
        if !listed.is_empty() {
            return listed;
        }
        self.owned_by(insert.handle).filter(|o| matches!(o.kind, ObjectKind::Attribute(_))).collect()
    }

    /// Number each layout's viewports in drawing order, the layout's own
    /// view first (DWG stores no numbers; DXF does). Viewports that
    /// already have numbers keep them.
    pub fn number_viewports(&mut self) {
        let blocks: Vec<Handle> = self.objects.values().filter(|o| matches!(o.kind, ObjectKind::BlockHeader(_))).map(|o| o.handle).collect();
        for b in blocks {
            let vps: Vec<Handle> =
                self.block_entities(b).iter().filter(|o| matches!(o.kind, ObjectKind::PaperViewport(_))).map(|o| o.handle).collect();
            for (i, h) in vps.iter().enumerate() {
                if let Some(ObjectKind::PaperViewport(v)) = self.objects.get_mut(h).map(|o| &mut o.kind)
                    && v.id == 0
                {
                    v.id = (i + 1).min(i16::MAX as usize) as i16;
                }
            }
        }
    }

    /// Counts of objects by type name: (decoded, unsupported, failed).
    pub fn type_counts(&self) -> BTreeMap<String, (usize, usize, usize)> {
        let mut m: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
        for o in self.objects.values() {
            let e = m.entry(o.type_name.clone()).or_default();
            match o.kind {
                ObjectKind::Unsupported | ObjectKind::NewerKind(_) => e.1 += 1,
                ObjectKind::Failed(_) | ObjectKind::Damaged(_) => e.2 += 1,
                _ => e.0 += 1,
            }
        }
        m
    }
}
