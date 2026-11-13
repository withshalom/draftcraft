//! Symbol-table objects: control objects, BLOCK_HEADER, LAYER and the
//! common fields of other table records.
//!
//! Written from knowledge of the spec, chapter 20. VERIFY against the spec
//! PDF and real files.

use super::streams::Streams;
use super::version::DwgVersion as V;
use crate::read::error::{Error, Result};
use crate::read::geom::{Vec2, Vec3};
use crate::read::model::{BlockHeader, Control, Handle, Layer, Linetype, LinetypeElement, NamedRecord, ObjectKind, TextStyle};

/// Fields every table record starts with: name, the "64" flag, the xref
/// index and the xref-dependent flag.
fn record_common(s: &mut Streams) -> Result<String> {
    Ok(record_common_flags(s)?.0)
}

/// The common fields with the two flags as DXF 70 bits: 64 (referenced)
/// and 16 (dependent on an xref).
///
/// From R2007 the record stores only the xref index: an AC1032 file saved
/// by AutoCAD LT 2026 has no 64-flag and no xref-dependent bit (every
/// LAYER, APPID and BLOCK_HEADER record then ends exactly where its
/// string stream begins; with the two bits none does). The specification
/// (§20.4.52 and the other table records) still lists both bits. Whether
/// the change came with R2007, R2010 or R2013 is not yet confirmed by
/// files of those versions; `object.rs` reports a record whose data does
/// not end at its strings, which is how a wrong guess would show.
fn record_common_flags(s: &mut Streams) -> Result<(String, u16)> {
    let name = s.tv()?;
    if s.ver.has_string_stream() {
        let _xref_index_plus1 = s.dat.bs()?;
        return Ok((name, 0));
    }
    let flag64 = s.dat.b()?;
    let _xref_index_plus1 = s.dat.bs()?;
    let xref_dependent = s.dat.b()?;
    Ok((name, ((flag64 as u16) << 6) | ((xref_dependent as u16) << 4)))
}

/// Returns the control object and its entry count.
pub(crate) fn control(s: &mut Streams) -> Result<(Control, u32)> {
    let n = s.dat.bl()?;
    Ok((Control::default(), n))
}

pub(crate) struct BlockHeaderCounts {
    pub owned: u32,
    pub inserts: u32,
    pub lists_entities: bool,
}

pub(crate) fn block_header(s: &mut Streams) -> Result<(BlockHeader, BlockHeaderCounts)> {
    let mut b = BlockHeader { name: record_common(s)?, ..Default::default() };
    b.anonymous = s.dat.b()?;
    b.has_attribs = s.dat.b()?;
    b.is_xref = s.dat.b()?;
    b.is_overlaid = s.dat.b()?;
    if s.ver >= V::R2000 {
        let _loaded = s.dat.b()?;
    }
    let lists_entities = !b.is_xref && !b.is_overlaid;
    let owned = if s.ver >= V::R2004 && lists_entities { s.dat.bl()? } else { 0 };
    b.base_point = Vec3::from_array(s.dat.bd3()?);
    b.xref_path = s.tv()?;
    let mut inserts = 0u32;
    if s.ver >= V::R2000 {
        // A run of non-zero bytes, one per insert handle, ended by a zero.
        loop {
            let c = s.dat.rc()?;
            if c == 0 {
                break;
            }
            inserts += 1;
            if inserts > 1_000_000 {
                return Err(Error::format("BLOCK_HEADER insert count runaway"));
            }
        }
        b.description = s.tv()?;
        let preview = s.dat.bl()? as usize;
        s.dat.skip_bits(preview.saturating_mul(8))?;
    }
    if s.ver >= V::R2007 {
        b.insert_units = s.dat.bs()?;
        b.not_explodable = !s.dat.b()?;
        let _scaling = s.dat.rc()?;
    }
    Ok((b, BlockHeaderCounts { owned, inserts, lists_entities }))
}

pub(crate) fn layer(s: &mut Streams) -> Result<Layer> {
    let mut l = Layer { name: record_common(s)?, plot: true, ..Default::default() };
    if s.ver <= V::R14 {
        let frozen = s.dat.b()?;
        let on = s.dat.b()?;
        let frozen_new = s.dat.b()?;
        let locked = s.dat.b()?;
        l.frozen = frozen;
        l.locked = locked;
        l.flags = (frozen as u16) | ((!on as u16) << 1) | ((frozen_new as u16) << 2) | ((locked as u16) << 3);
    } else {
        let flags = s.dat.bs()?;
        l.flags = flags;
        l.frozen = flags & 0x01 != 0;
        l.locked = flags & 0x08 != 0;
        l.plot = flags & 0x10 != 0;
        l.lineweight = ((flags & 0x03E0) >> 5) as u8;
    }
    let (color, index) = s.cmc()?;
    // Up to R14 a negative colour index marks a layer that is switched
    // off. From R2000 the flag word's 0x02 bit says it: the spec (§20.4.54)
    // calls the bit "on", but AutoCAD 2027's files leave it clear on every
    // layer their DXF shows on (corpus MI-201, 2000 to 2018), so a set bit
    // is a layer that is off. From R2004 the colour's index field is always
    // 0 (the colour is in the method word), so its sign says nothing.
    l.off = if s.ver >= V::R2000 { l.flags & 0x02 != 0 } else { index < 0 };
    l.color = color;
    Ok(l)
}

/// VIEW, UCS, VPORT, DIMSTYLE, VX records: only the name is decoded for
/// now.
pub(crate) fn named_record(s: &mut Streams) -> Result<NamedRecord> {
    Ok(NamedRecord { name: record_common(s)? })
}

/// APPID (§20.4.66): the name, then an undocumented RC (DXF 71), which an
/// AC1032 file confirms is there.
pub(crate) fn appid(s: &mut Streams) -> Result<NamedRecord> {
    let name = record_common(s)?;
    let _unknown71 = s.dat.rc()?;
    Ok(NamedRecord { name })
}

/// Most elements AutoCAD allows in one linetype pattern.
const MAX_DASHES: usize = 12;

/// LTYPE (ODA spec §20.4.58, read from the specification text): name,
/// description and pattern. Not yet checked against real files, so the
/// record must pass checks the specification makes possible (alignment
/// 'A', at most 12 elements, element lengths adding up to the pattern
/// length); one that does not is kept by name only, as before.
pub(crate) fn linetype(s: &mut Streams) -> Result<ObjectKind> {
    let (name, flags) = record_common_flags(s)?;
    Ok(match linetype_body(s, &name, flags) {
        Ok(lt) => ObjectKind::Linetype(lt),
        Err(_) => ObjectKind::NamedRecord(NamedRecord { name }),
    })
}

fn linetype_body(s: &mut Streams, name: &str, flags: u16) -> Result<Linetype> {
    let description = s.tv()?;
    let pattern_length = s.dat.bd()?;
    let alignment = s.dat.rc()?;
    let n = s.dat.rc()? as usize;
    if alignment != b'A' || n > MAX_DASHES || !pattern_length.is_finite() || pattern_length < 0.0 {
        return Err(Error::format("LTYPE: unexpected pattern header"));
    }
    let mut elements = Vec::with_capacity(n);
    for _ in 0..n {
        let length = s.dat.bd()?;
        let shape = s.dat.bs()?;
        let x = s.dat.rd()?;
        let y = s.dat.rd()?;
        let scale = s.dat.bd()?;
        let rotation = s.dat.bd()?;
        let complex = s.dat.bs()?;
        elements.push(LinetypeElement { length, complex, shape, style: Handle::NULL, scale, rotation, offset: Vec2::new(x, y), text: String::new() });
    }
    let sane = elements
        .iter()
        .all(|e| e.length.is_finite() && e.length.abs() < 1e9 && e.offset.is_finite() && e.scale.is_finite() && e.rotation.is_finite());
    let sum: f64 = elements.iter().map(|e| e.length.abs()).sum();
    if !sane || (sum - pattern_length).abs() > 1e-6 * (1.0 + pattern_length) {
        return Err(Error::format("LTYPE: elements do not add up to the pattern"));
    }
    // Text elements (DXF 74 bit 2) find their string in a text area by
    // their 75 value: 0-terminated strings, 256 bytes up to R2004, and 512
    // bytes of UTF-16 from R2007, present only when some element is text.
    // Shape elements (bit 4) keep their shape number in 75. The spec's note
    // on the shape flag gives these two bits the other way round, against
    // its own R2007 rule and the DXF reference for group 74; the DXF
    // meaning is used. VERIFY with real files. A missing area only loses
    // the text (the handles are in their own stream).
    let is_text = |e: &LinetypeElement| e.complex & 2 != 0;
    if s.ver <= V::R2004 {
        if let Ok(area) = s.dat.raw_bytes(256) {
            for e in elements.iter_mut().filter(|e| is_text(e)) {
                let start = (e.shape as usize).min(area.len());
                let end = area[start..].iter().position(|&b| b == 0).map_or(area.len(), |p| start + p);
                e.text = (s.text.ansi)(&area[start..end], s.codepage);
                e.shape = 0;
            }
        }
    } else if elements.iter().any(is_text) {
        // R2007+: 512 bytes "if the 0x02 bit is set on any shape flag"
        // (the spec's words), read as UTF-16. VERIFY with real files.
        if let Ok(area) = s.dat.raw_bytes(512) {
            for e in elements.iter_mut().filter(|e| is_text(e)) {
                let units: Vec<u16> = area
                    .get(e.shape as usize..)
                    .unwrap_or(&[])
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .take_while(|&u| u != 0)
                    .collect();
                e.text = String::from_utf16_lossy(&units);
                e.shape = 0;
            }
        }
    }
    Ok(Linetype { name: name.to_string(), description, flags, pattern_length, elements })
}

/// STYLE (ODA spec §20.4.56, read from the specification text, and checked
/// against its example): name, flags, sizes and font files. A record whose
/// values are out of range is kept by name only.
pub(crate) fn text_style(s: &mut Streams) -> Result<ObjectKind> {
    let (name, flags) = record_common_flags(s)?;
    Ok(match text_style_body(s, &name, flags) {
        Ok(t) => ObjectKind::TextStyle(t),
        Err(_) => ObjectKind::NamedRecord(NamedRecord { name }),
    })
}

fn text_style_body(s: &mut Streams, name: &str, flags: u16) -> Result<TextStyle> {
    let vertical = s.dat.b()?;
    let shape_file = s.dat.b()?;
    let height = s.dat.bd()?;
    let width_factor = s.dat.bd()?;
    let oblique = s.dat.bd()?;
    let generation = s.dat.rc()?;
    let last_height = s.dat.bd()?;
    let font_file = s.tv()?;
    let bigfont_file = s.tv()?;
    let ok = height.is_finite()
        && height >= 0.0
        && width_factor.is_finite()
        && width_factor > 0.0
        && width_factor <= 100.0
        && oblique.is_finite()
        && oblique.abs() < std::f64::consts::FRAC_PI_2
        && last_height.is_finite()
        && last_height >= 0.0;
    if !ok {
        return Err(Error::format("STYLE: values out of range"));
    }
    Ok(TextStyle {
        name: name.to_string(),
        flags: flags | (shape_file as u16) | ((vertical as u16) << 2),
        height,
        width_factor,
        oblique,
        generation: generation as u16,
        last_height,
        font_file,
        bigfont_file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bits::BitReader;

    /// Writes DWG bit codes, first bit most significant, for building
    /// records in tests.
    #[derive(Default)]
    struct Bits {
        out: Vec<u8>,
        n: usize,
    }

    impl Bits {
        fn bit(&mut self, v: bool) {
            if self.n.is_multiple_of(8) {
                self.out.push(0);
            }
            if v {
                let last = self.out.len() - 1;
                self.out[last] |= 0x80 >> (self.n % 8);
            }
            self.n += 1;
        }
        fn bits(&mut self, v: u64, count: u32) {
            for i in (0..count).rev() {
                self.bit((v >> i) & 1 == 1);
            }
        }
        fn rc(&mut self, v: u8) {
            self.bits(v as u64, 8);
        }
        fn rd(&mut self, v: f64) {
            for b in v.to_le_bytes() {
                self.rc(b);
            }
        }
        fn bs(&mut self, v: u16) {
            match v {
                0 => self.bits(2, 2),
                1..=255 => {
                    self.bits(1, 2);
                    self.rc(v as u8);
                }
                _ => {
                    self.bits(0, 2);
                    self.rc(v as u8);
                    self.rc((v >> 8) as u8);
                }
            }
        }
        fn bd(&mut self, v: f64) {
            if v == 1.0 {
                self.bits(1, 2);
            } else if v == 0.0 {
                self.bits(2, 2);
            } else {
                self.bits(0, 2);
                self.rd(v);
            }
        }
        fn tv(&mut self, s: &str) {
            self.bs(s.len() as u16);
            for b in s.bytes() {
                self.rc(b);
            }
        }
        /// Name, 64-flag, xref index + 1, xref-dependent.
        fn record(&mut self, name: &str) {
            self.tv(name);
            self.bit(false);
            self.bs(0);
            self.bit(false);
        }
    }

    fn ltype_bytes(alignment: u8, dashes: &[f64], text: Option<&str>) -> Vec<u8> {
        let mut w = Bits::default();
        w.record("GAS");
        w.tv("Gas line");
        w.bd(dashes.iter().map(|d| d.abs()).sum());
        w.rc(alignment);
        w.rc(dashes.len() as u8);
        for (i, &d) in dashes.iter().enumerate() {
            w.bd(d);
            let is_text = text.is_some() && i == 1;
            w.bs(if is_text { 1 } else { 0 }); // shape code: offset 1 in the area
            w.rd(if is_text { -0.1 } else { 0.0 });
            w.rd(0.0);
            w.bd(1.0);
            w.bd(0.0);
            w.bs(if is_text { 2 } else { 0 });
        }
        let mut area = [0u8; 256];
        if let Some(t) = text {
            area[1..1 + t.len()].copy_from_slice(t.as_bytes());
        }
        for b in area {
            w.rc(b);
        }
        w.out
    }

    fn read(bytes: &[u8]) -> ObjectKind {
        let mut s = Streams { ver: V::R2000, codepage: 30, dat: BitReader::new(bytes), strs: None, hdl: None, text: Default::default() };
        linetype(&mut s).unwrap()
    }

    #[test]
    fn linetype_patterns_and_text_elements_are_read() {
        match read(&ltype_bytes(b'A', &[12.0, -1.5, 3.0, -1.5], None)) {
            ObjectKind::Linetype(l) => {
                assert_eq!(l.name, "GAS");
                assert_eq!(l.description, "Gas line");
                assert_eq!(l.dashes(), vec![12.0, -1.5, 3.0, -1.5]);
                assert_eq!(l.pattern_length, 18.0);
            }
            other => panic!("{other:?}"),
        }
        match read(&ltype_bytes(b'A', &[12.7, -5.08, 0.0], Some("GAS"))) {
            ObjectKind::Linetype(l) => {
                assert_eq!(l.elements[1].text, "GAS");
                assert_eq!(l.elements[1].complex, 2);
                assert_eq!(l.elements[1].shape, 0);
                assert_eq!(l.elements[1].offset, Vec2::new(-0.1, 0.0));
            }
            other => panic!("{other:?}"),
        }
    }

    /// An LTYPE record of `ver` named GAS, its elements given as (length,
    /// DXF 74 flags, 75 code), followed by `area`.
    fn ltype_in(ver: V, elements: &[(f64, u16, u16)], area: &[u8], referenced: bool) -> ObjectKind {
        let mut w = Bits::default();
        let strings_apart = ver.has_string_stream();
        if !strings_apart {
            w.tv("GAS");
        }
        if strings_apart {
            // R2007+: the xref index only (see `record_common_flags`).
            w.bs(0);
        } else {
            w.bit(referenced);
            w.bs(0);
            w.bit(referenced);
        }
        if !strings_apart {
            w.tv("");
        }
        w.bd(elements.iter().map(|e| e.0.abs()).sum());
        w.rc(b'A');
        w.rc(elements.len() as u8);
        for &(length, flags, code) in elements {
            w.bd(length);
            w.bs(code);
            w.rd(0.0);
            w.rd(0.0);
            w.bd(1.0);
            w.bd(0.0);
            w.bs(flags);
        }
        for &b in area {
            w.rc(b);
        }
        let mut s = Streams { ver, codepage: 30, dat: BitReader::new(&w.out), strs: None, hdl: None, text: Default::default() };
        linetype(&mut s).unwrap()
    }

    #[test]
    fn text_and_shape_elements_by_their_flag_bits() {
        let lt = |k: ObjectKind| match k {
            ObjectKind::Linetype(l) => l,
            other => panic!("{other:?}"),
        };
        // R2000: the 256-byte area is always there.
        let mut area = vec![0u8; 256];
        area[1..4].copy_from_slice(b"GAS");
        let text = lt(ltype_in(V::R2000, &[(12.7, 0, 0), (-5.08, 2, 1)], &area, false));
        assert_eq!((text.elements[1].text.as_str(), text.elements[1].shape), ("GAS", 0));
        let shape = lt(ltype_in(V::R2000, &[(12.7, 0, 0), (-5.08, 4, 130)], &area, false));
        let e = &shape.elements[1];
        assert_eq!((e.text.as_str(), e.shape, e.complex), ("", 130, 4));
        // R2010: 512 bytes of UTF-16, there only when an element is text.
        let mut area = vec![0u8; 512];
        for (i, u) in "GAS".encode_utf16().enumerate() {
            area[2 + 2 * i..4 + 2 * i].copy_from_slice(&u.to_le_bytes());
        }
        let text = lt(ltype_in(V::R2010, &[(12.7, 0, 0), (-5.08, 2, 2)], &area, false));
        assert_eq!(text.elements[1].text, "GAS");
        let shape = lt(ltype_in(V::R2010, &[(12.7, 0, 0), (-5.08, 4, 130)], &[], false));
        assert_eq!(shape.elements[1].shape, 130);
        // The referenced and xref-dependent bits come through as DXF 70.
        let xref = lt(ltype_in(V::R2000, &[(1.0, 0, 0), (-1.0, 0, 0)], &[0; 256], true));
        assert_eq!(xref.flags, 64 | 16);
    }

    #[test]
    fn linetypes_that_fail_the_checks_keep_their_name() {
        // Alignment not 'A': a layout this reader does not trust.
        assert_eq!(read(&ltype_bytes(b'X', &[1.0, -1.0], None)), ObjectKind::NamedRecord(NamedRecord { name: "GAS".into() }));
        // Lengths that do not add up to the pattern length.
        let mut w = Bits::default();
        w.record("ODD");
        w.tv("");
        w.bd(99.0);
        w.rc(b'A');
        w.rc(1);
        w.bd(2.0);
        w.bs(0);
        w.rd(0.0);
        w.rd(0.0);
        w.bd(1.0);
        w.bd(0.0);
        w.bs(0);
        assert_eq!(read(&w.out), ObjectKind::NamedRecord(NamedRecord { name: "ODD".into() }));
        // Cut short after the name: still the name.
        let full = ltype_bytes(b'A', &[1.0, -1.0], None);
        assert!(matches!(read(&full[..6]), ObjectKind::NamedRecord(_)));
    }
}
