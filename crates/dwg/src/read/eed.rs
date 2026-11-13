//! Extended entity data (EED): the per-application data DXF calls XDATA.
//!
//! Spec: ODA Open Design Specification for .dwg files v5.4.1, chapter 28
//! "Extended Entity Data (Extended Object Data)", checked against the spec
//! text. Its place in every object, right after the object's own handle,
//! is from §20.1 and §20.4.1 (also checked). Layout, repeated until a
//! length of zero:
//!
//! ```text
//! BS   length of this application's data, in bytes
//! H    the application's APPID table record
//! …    that many bytes of items: a one-byte code (DXF group − 1000),
//!      then the value, with no bit coding
//! ```
//!
//! Three points the spec leaves open, and how this module handles them:
//!
//! - **Byte order of the 8-byte handles in items 3 and 5.** The spec says
//!   to read them "as usual for handles" (most significant byte first, as
//!   in §2.13), but also that all data is little-endian unless stated
//!   otherwise (§2). Items are read most significant first, then
//!   [`resolve`] checks which order makes the drawing's 1003 and 1005
//!   values point at existing layers and objects, and keeps that one, as
//!   the object map reader does with its two readings.
//! - **8-bit strings (R13–R2004)** carry a code page, a "2-byte short" with
//!   no byte order given. Real R14, 2000 and 2004 files store it most
//!   significant byte first (00 1E for code page 30), so it is read that
//!   way, and least significant first is accepted too. Only ANSI_1252 is
//!   decoded (code page 30, or 1252), exactly, one character per byte;
//!   strings in any other code page are kept as bytes.
//! - **Terminators.** The spec describes none, so strings are kept exactly,
//!   including any NUL characters. Real files will show whether AutoCAD
//!   counts a terminating NUL.

use super::version::DwgVersion;
use crate::bits::BitReader;
use crate::read::error::{Error, Result};
use crate::read::geom::Vec3;
use crate::read::model::{Diagnostic, Handle, Object, ObjectKind, Severity, XData, XDataApp, XValue};
use crate::read::text;
use std::collections::{BTreeMap, BTreeSet};

/// DWG type codes this module looks for: APPID (67, §20.4.66) and LAYER
/// (51, §20.4.54).
const APPID: u16 = 0x43;
const LAYER: u16 = 0x33;

/// Read the extended data at the reader's position and leave the reader
/// just past the terminating zero length. Applications are returned by
/// handle; [`resolve`] names them once every object is loaded. `own`
/// is the object's handle, for references stored relative to it.
pub(crate) fn read(dat: &mut BitReader, ver: DwgVersion, own: u64, text: Option<text::TextDecoding>) -> Result<Vec<XData>> {
    let mut out = Vec::new();
    loop {
        let size = dat.bs()? as usize;
        if size == 0 {
            return Ok(out);
        }
        let app = dat.handle()?.absolute(own);
        let data = dat.raw_bytes(size)?;
        out.push(XData { app: XDataApp::Handle(Handle(app)), items: items_with(&data, ver, text) });
    }
}

/// Decode one application's items. An item that cannot be decoded ends
/// the list with an [`XValue::Undecoded`] holding the rest of the bytes, so
/// nothing is lost.
#[cfg(test)]
pub(crate) fn items(data: &[u8], ver: DwgVersion) -> Vec<XValue> {
    items_with(data, ver, None)
}

/// [`items`], with the strings decoded by `text` when given (every code page, every escape)
/// rather than kept byte for byte in Windows-1252.
pub(crate) fn items_with(data: &[u8], ver: DwgVersion, text: Option<text::TextDecoding>) -> Vec<XValue> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p < data.len() {
        let start = p;
        match item(data, &mut p, ver, text) {
            Ok(v) => out.push(v),
            Err(_) => {
                out.push(XValue::Undecoded { version: ver, data: data[start..].to_vec() });
                break;
            }
        }
    }
    out
}

fn take<'a>(data: &'a [u8], p: &mut usize, n: usize) -> Result<&'a [u8]> {
    let end = p
        .checked_add(n)
        .filter(|&e| e <= data.len())
        .ok_or_else(|| Error::truncated(format!("extended data item needs {n} bytes at byte {p} of {}", data.len())))?;
    let s = &data[*p..end];
    *p = end;
    Ok(s)
}

fn fixed<const N: usize>(data: &[u8], p: &mut usize) -> Result<[u8; N]> {
    let mut a = [0u8; N];
    a.copy_from_slice(take(data, p, N)?);
    Ok(a)
}

fn real(data: &[u8], p: &mut usize) -> Result<f64> {
    Ok(f64::from_le_bytes(fixed::<8>(data, p)?))
}

/// Items 3 and 5: eight bytes, read most significant first (see the
/// module notes; [`resolve`] may reverse them for the whole drawing).
fn handle8(data: &[u8], p: &mut usize) -> Result<Handle> {
    Ok(Handle(u64::from_be_bytes(fixed::<8>(data, p)?)))
}

/// DWG code page numbers for Windows-1252: 30 in the DWG numbering (as the
/// file header's DWGCODEPAGE uses), 1252 in Windows' own.
fn is_cp1252(codepage: u16) -> bool {
    matches!(codepage, 30 | 1252)
}

fn string(data: &[u8], p: &mut usize, ver: DwgVersion, decoding: Option<text::TextDecoding>) -> Result<String> {
    if ver >= DwgVersion::R2007 {
        // RS length in characters, then UTF-16LE.
        let n = u16::from_le_bytes(fixed::<2>(data, p)?) as usize;
        let bytes = take(data, p, n * 2)?;
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        let s = String::from_utf16(&units).map_err(|_| Error::format("group 1000: invalid UTF-16"))?;
        Ok(decoding.map_or(s.clone(), |d| (d.unicode)(s)))
    } else {
        // RC length, RS code page, then that many 8-bit characters.
        let n = fixed::<1>(data, p)?[0] as usize;
        let raw = fixed::<2>(data, p)?;
        let (be, le) = (u16::from_be_bytes(raw), u16::from_le_bytes(raw));
        let codepage = if !is_cp1252(be) && is_cp1252(le) { le } else { be };
        let bytes = take(data, p, n)?;
        if let Some(d) = decoding {
            return Ok((d.ansi)(bytes, codepage));
        }
        if !is_cp1252(codepage) {
            return Err(Error::new(crate::read::error::ErrorKind::Unsupported, format!("group 1000: code page {codepage} is not decoded")));
        }
        Ok(text::decode_cp1252_exact(bytes))
    }
}

fn item(data: &[u8], p: &mut usize, ver: DwgVersion, text: Option<text::TextDecoding>) -> Result<XValue> {
    let code = fixed::<1>(data, p)?[0];
    Ok(match code {
        0 => XValue::Str(string(data, p, ver, text)?),
        2 => match fixed::<1>(data, p)?[0] {
            0 => XValue::Open,
            1 => XValue::Close,
            b => return Err(Error::format(format!("group 1002: value {b}"))),
        },
        3 => XValue::Layer(handle8(data, p)?),
        4 => {
            let n = fixed::<1>(data, p)?[0] as usize;
            XValue::Bytes(take(data, p, n)?.to_vec())
        }
        5 => XValue::Handle(handle8(data, p)?),
        10..=13 => {
            let v = Vec3::new(real(data, p)?, real(data, p)?, real(data, p)?);
            match code {
                10 => XValue::Point(v),
                11 => XValue::Position(v),
                12 => XValue::Displacement(v),
                _ => XValue::Direction(v),
            }
        }
        40..=42 => {
            let x = real(data, p)?;
            match code {
                40 => XValue::Real(x),
                41 => XValue::Distance(x),
                _ => XValue::ScaleFactor(x),
            }
        }
        70 => XValue::Int16(i16::from_le_bytes(fixed::<2>(data, p)?)),
        71 => XValue::Int32(i32::from_le_bytes(fixed::<4>(data, p)?)),
        // 1 (group 1001) is not valid inside the data, per the spec.
        c => return Err(Error::format(format!("extended data item code {c} (group {})", 1000 + c as u32))),
    })
}

/// Finish the extended data once every object is loaded: settle the byte
/// order of item 3 and 5 handles, name each application from its APPID
/// record, and report what could not be resolved.
pub(crate) fn resolve(objects: &mut BTreeMap<Handle, Object>, diagnostics: &mut Vec<Diagnostic>) {
    let names: BTreeMap<Handle, String> = objects
        .iter()
        .filter_map(|(h, o)| match &o.kind {
            ObjectKind::NamedRecord(r) if o.type_code == APPID => Some((*h, r.name.clone())),
            _ => None,
        })
        .collect();
    let layers: BTreeSet<Handle> = objects.iter().filter(|(_, o)| o.type_code == LAYER).map(|(h, _)| *h).collect();

    // Which byte order makes the handles point at something: layers for
    // 1003, any object for 1005.
    let (mut refs, mut as_read, mut reversed) = (0usize, 0usize, 0usize);
    for o in objects.values() {
        for it in o.xdata.iter().flat_map(|x| &x.items) {
            let (h, is_layer) = match it {
                XValue::Layer(h) => (*h, true),
                XValue::Handle(h) => (*h, false),
                _ => continue,
            };
            if h.is_null() {
                continue;
            }
            let hit = |k: Handle| {
                if is_layer { layers.contains(&k) } else { objects.contains_key(&k) }
            };
            refs += 1;
            as_read += hit(h) as usize;
            reversed += hit(Handle(h.0.swap_bytes())) as usize;
        }
    }
    let swap = reversed > as_read;

    let (mut carriers, mut blocks, mut unnamed, mut undecoded) = (0usize, 0usize, 0usize, 0usize);
    let (mut layer_refs, mut stray_layer_refs) = (0usize, 0usize);
    for o in objects.values_mut() {
        if !o.xdata.is_empty() {
            carriers += 1;
        }
        for x in &mut o.xdata {
            blocks += 1;
            if let XDataApp::Handle(h) = x.app {
                match names.get(&h) {
                    Some(n) => x.app = XDataApp::Name(n.clone()),
                    None => unnamed += 1,
                }
            }
            for it in &mut x.items {
                match it {
                    XValue::Layer(h) | XValue::Handle(h) if swap => *h = Handle(h.0.swap_bytes()),
                    _ => {}
                }
                match it {
                    XValue::Layer(h) => {
                        layer_refs += 1;
                        if !layers.contains(h) {
                            stray_layer_refs += 1;
                        }
                    }
                    XValue::Undecoded { .. } => undecoded += 1,
                    _ => {}
                }
            }
        }
    }

    let mut note = |severity, message: String| diagnostics.push(Diagnostic { severity, message });
    if blocks > 0 {
        note(Severity::Info, format!("extended data: {blocks} application blocks on {carriers} objects"));
    }
    if refs > 0 {
        let (order, hits) = if swap { ("least", reversed) } else { ("most", as_read) };
        note(
            if hits == 0 { Severity::Warning } else { Severity::Info },
            format!(
                "extended data: handles (1003, 1005) read {order} significant byte first; \
                 {hits} of {refs} point at objects in the drawing"
            ),
        );
    }
    if unnamed > 0 {
        note(
            Severity::Warning,
            format!(
                "extended data: {unnamed} blocks refer to an application record that is \
                 missing or unreadable; kept by handle"
            ),
        );
    }
    if undecoded > 0 {
        note(Severity::Warning, format!("extended data: {undecoded} blocks only partly decoded; the rest is kept as bytes"));
    }
    if stray_layer_refs > 0 {
        note(
            Severity::Warning,
            format!(
                "extended data: {stray_layer_refs} of {layer_refs} layer references (1003) do not \
                 point at a LAYER"
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::model::NamedRecord;

    /// Writes DWG bit codes most significant bit first, for building test
    /// input (the reader side is `BitReader`).
    #[derive(Default)]
    struct BitWriter {
        buf: Vec<u8>,
        bits: usize,
    }

    impl BitWriter {
        fn bit(&mut self, b: bool) {
            if self.bits.is_multiple_of(8) {
                self.buf.push(0);
            }
            if b {
                self.buf[self.bits / 8] |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
        fn bits(&mut self, n: u32, v: u64) {
            for i in (0..n).rev() {
                self.bit((v >> i) & 1 == 1);
            }
        }
        fn rc(&mut self, v: u8) {
            self.bits(8, v as u64);
        }
        /// BS as in §2.2.
        fn bs(&mut self, v: u16) {
            match v {
                0 => self.bits(2, 2),
                256 => self.bits(2, 3),
                1..=255 => {
                    self.bits(2, 1);
                    self.rc(v as u8);
                }
                _ => {
                    self.bits(2, 0);
                    self.rc(v as u8);
                    self.rc((v >> 8) as u8);
                }
            }
        }
        /// H as in §2.13: code, byte count, bytes most significant first.
        fn handle(&mut self, code: u8, v: u64) {
            let bytes: Vec<u8> = v.to_be_bytes().into_iter().skip_while(|b| *b == 0).collect();
            self.rc((code << 4) | bytes.len() as u8);
            for b in bytes {
                self.rc(b);
            }
        }
        fn block(&mut self, app: u64, data: &[u8]) {
            self.bs(data.len() as u16);
            self.handle(5, app);
            for b in data {
                self.rc(*b);
            }
        }
    }

    fn v3(x: f64, y: f64, z: f64) -> Vec<u8> {
        [x, y, z].iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// One item of every code, R13–R2004 string form.
    fn every_item_r2004() -> (Vec<u8>, Vec<XValue>) {
        let mut d = Vec::new();
        d.extend([0, 4, 30, 0]);
        d.extend(b"Test");
        d.extend([2, 0]);
        d.extend([3, 0, 0, 0, 0, 0, 0, 0, 0x10]);
        d.extend([4, 3, 0xDE, 0xAD, 0xBF]);
        d.extend([5, 0, 0, 0, 0, 0, 0, 0x01, 0x2C]);
        for (code, v) in [(10u8, 1.0), (11, 2.0), (12, 3.0), (13, 4.0)] {
            d.push(code);
            d.extend(v3(v, -v, 0.5));
        }
        for (code, v) in [(40u8, 1.5), (41, 2.5), (42, 0.25)] {
            d.push(code);
            d.extend(f64::to_le_bytes(v));
        }
        d.push(70);
        d.extend((-2i16).to_le_bytes());
        d.push(71);
        d.extend(123_456_789i32.to_le_bytes());
        d.extend([2, 1]);
        let want = vec![
            XValue::Str("Test".into()),
            XValue::Open,
            XValue::Layer(Handle(0x10)),
            XValue::Bytes(vec![0xDE, 0xAD, 0xBF]),
            XValue::Handle(Handle(0x12C)),
            XValue::Point(Vec3::new(1.0, -1.0, 0.5)),
            XValue::Position(Vec3::new(2.0, -2.0, 0.5)),
            XValue::Displacement(Vec3::new(3.0, -3.0, 0.5)),
            XValue::Direction(Vec3::new(4.0, -4.0, 0.5)),
            XValue::Real(1.5),
            XValue::Distance(2.5),
            XValue::ScaleFactor(0.25),
            XValue::Int16(-2),
            XValue::Int32(123_456_789),
            XValue::Close,
        ];
        (d, want)
    }

    #[test]
    fn decodes_every_item_code() {
        let (d, want) = every_item_r2004();
        assert_eq!(items(&d, DwgVersion::R2004), want);
    }

    #[test]
    fn strings_are_kept_exactly() {
        // R2007+: a character count, then UTF-16LE; a NUL is kept.
        let d = [0u8, 4, 0, 0xD8, 0, b'-', 0, b'x', 0, 0, 0];
        assert_eq!(items(&d, DwgVersion::R2018), vec![XValue::Str("Ø-x\0".into())]);
        // R13–R2004 in ANSI_1252 (code page 30, or 1252): one character per
        // byte, NULs and escapes left as they are.
        // The code page is stored most significant byte first in real files
        // (00 1E); the other order is accepted too.
        for cp in [[0u8, 30], [30, 0]] {
            let d = [0u8, 3, cp[0], cp[1], b'4', b'5', 0xB0];
            assert_eq!(items(&d, DwgVersion::R2000), vec![XValue::Str("45°".into())]);
        }
        // Code page 1252 (0x04E4): "a", NUL, then a literal "\U+0041b".
        let mut d = vec![0u8, 10, 0xE4, 0x04];
        d.extend(b"a\0\\U+0041b");
        assert_eq!(items(&d, DwgVersion::R2004), vec![XValue::Str("a\0\\U+0041b".into())]);
        // Any other code page is kept as bytes (Shift-JIS here).
        let d = vec![0u8, 2, 42, 0, 0x93, 0xFA, 70, 1, 0];
        assert_eq!(items(&d, DwgVersion::R2004), vec![XValue::Undecoded { version: DwgVersion::R2004, data: d.clone() }]);
    }

    #[test]
    fn undecodable_items_keep_their_bytes() {
        let v = DwgVersion::R2004;
        // Code 1 (group 1001) is invalid inside the data.
        let d = [70u8, 7, 0, 1, 9, 9];
        assert_eq!(items(&d, v), vec![XValue::Int16(7), XValue::Undecoded { version: v, data: vec![1, 9, 9] }]);
        // A string longer than the data, a bad brace, an unknown code.
        for d in [vec![0u8, 10, 30, 0, b'a'], vec![2u8, 7], vec![99u8, 1, 2], vec![5u8, 0, 0]] {
            assert_eq!(items(&d, v), vec![XValue::Undecoded { version: v, data: d.clone() }]);
        }
        // An unpaired UTF-16 surrogate is kept as bytes, not replaced.
        let d = vec![0u8, 1, 0, 0x00, 0xD8];
        assert_eq!(items(&d, DwgVersion::R2010), vec![XValue::Undecoded { version: DwgVersion::R2010, data: d.clone() }]);
    }

    #[test]
    fn reads_blocks_until_the_zero_length() {
        let (d, want) = every_item_r2004();
        let mut w = BitWriter::default();
        w.bits(3, 0b101); // EED rarely starts on a byte boundary
        w.block(0x12, &d);
        w.block(0x1A3, &[70, 1, 0]);
        w.bs(0);
        w.bits(5, 0b10011); // what follows in the object
        let mut r = BitReader::new(&w.buf);
        r.skip_bits(3).unwrap();
        let x = read(&mut r, DwgVersion::R2004, 0x40, None).unwrap();
        assert_eq!(
            x,
            vec![
                XData { app: XDataApp::Handle(Handle(0x12)), items: want },
                XData { app: XDataApp::Handle(Handle(0x1A3)), items: vec![XValue::Int16(1)] },
            ]
        );
        assert_eq!(r.bits(5).unwrap(), 0b10011);
        // No extended data: just the zero length.
        let mut w = BitWriter::default();
        w.bs(0);
        let mut r = BitReader::new(&w.buf);
        assert!(read(&mut r, DwgVersion::R2018, 1, None).unwrap().is_empty());
        assert_eq!(r.position(), 2);
    }

    #[test]
    fn truncated_block_is_an_error() {
        let mut w = BitWriter::default();
        w.bs(40);
        w.handle(5, 0x12);
        w.rc(70);
        let mut r = BitReader::new(&w.buf);
        assert!(read(&mut r, DwgVersion::R2004, 1, None).is_err());
    }

    fn object(h: u64, code: u16, name: &str, kind: ObjectKind, xdata: Vec<XData>) -> Object {
        Object {
            handle: Handle(h),
            type_code: code,
            type_name: name.into(),
            owner: Handle::NULL,
            reactors: vec![],
            xdictionary: Handle::NULL,
            entity: None,
            kind,
            offset: 0,
            size: 0,
            note: None,
            raw: None,
            xdata,
            unknown_tags: vec![],
        }
    }

    #[test]
    fn applications_are_named_from_appid_records() {
        let appid = |h, n: &str| object(h, APPID, "APPID", ObjectKind::NamedRecord(NamedRecord { name: n.into() }), vec![]);
        let mut objects: BTreeMap<Handle, Object> = [
            appid(0x12, "ACAD"),
            appid(0x13, "AcadAnnotative"),
            object(0x10, LAYER, "LAYER", ObjectKind::Unsupported, vec![]),
            object(
                0x100,
                0x13,
                "LINE",
                ObjectKind::Unsupported,
                vec![
                    XData { app: XDataApp::Handle(Handle(0x13)), items: vec![XValue::Layer(Handle(0x10))] },
                    XData { app: XDataApp::Handle(Handle(0x99)), items: vec![XValue::Layer(Handle(0x11))] },
                ],
            ),
        ]
        .into_iter()
        .map(|o| (o.handle, o))
        .collect();
        let mut diags = Vec::new();
        resolve(&mut objects, &mut diags);
        let line = &objects[&Handle(0x100)];
        assert_eq!(line.xdata[0].app, XDataApp::Name("AcadAnnotative".into()));
        assert_eq!(line.xdata[1].app, XDataApp::Handle(Handle(0x99)));
        assert!(line.xdata_for("ACADANNOTATIVE").is_some());
        assert!(line.xdata_for("ACAD").is_none());
        let text: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(text.iter().any(|m| m.contains("2 application blocks on 1 objects")));
        assert!(text.iter().any(|m| m.contains("1 blocks refer to an application")));
        assert!(text.iter().any(|m| m.contains("1 of 2 layer references")));
    }

    #[test]
    fn handle_byte_order_is_settled_by_what_the_handles_point_at() {
        let objects_with = |items: Vec<XValue>| -> BTreeMap<Handle, Object> {
            [
                object(0x10, LAYER, "LAYER", ObjectKind::Unsupported, vec![]),
                object(0x100, 0x13, "LINE", ObjectKind::Unsupported, vec![XData { app: XDataApp::Handle(Handle(0x12)), items }]),
            ]
            .into_iter()
            .map(|o| (o.handle, o))
            .collect()
        };
        // Bytes written most significant first: kept as read.
        let d = [&[3u8, 0, 0, 0, 0, 0, 0, 0, 0x10][..], &[5u8, 0, 0, 0, 0, 0, 0, 0x01, 0x00][..]].concat();
        let mut objects = objects_with(items(&d, DwgVersion::R2004));
        let mut diags = Vec::new();
        resolve(&mut objects, &mut diags);
        let want = vec![XValue::Layer(Handle(0x10)), XValue::Handle(Handle(0x100))];
        assert_eq!(objects[&Handle(0x100)].xdata[0].items, want);
        assert!(diags.iter().any(|x| x.message.contains("most significant byte first; 2 of 2")));
        // The same values written least significant first: reversed.
        let d = [&[3u8, 0x10, 0, 0, 0, 0, 0, 0, 0][..], &[5u8, 0x00, 0x01, 0, 0, 0, 0, 0, 0][..]].concat();
        let mut objects = objects_with(items(&d, DwgVersion::R2004));
        let mut diags = Vec::new();
        resolve(&mut objects, &mut diags);
        assert_eq!(objects[&Handle(0x100)].xdata[0].items, want);
        assert!(diags.iter().any(|x| x.message.contains("least significant byte first; 2 of 2")));
        // Nothing to go by: kept as read, with a warning.
        let d = [5u8, 0, 0, 0, 0, 0, 0, 0x0F, 0xFF];
        let mut objects = objects_with(items(&d, DwgVersion::R2004));
        let mut diags = Vec::new();
        resolve(&mut objects, &mut diags);
        assert_eq!(objects[&Handle(0x100)].xdata[0].items, vec![XValue::Handle(Handle(0xFFF))]);
        assert!(diags.iter().any(|x| x.severity == Severity::Warning && x.message.contains("0 of 1")));
    }

    #[test]
    fn random_input_never_panics() {
        let mut s = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..20_000 {
            let n = (next() % 64) as usize;
            let d: Vec<u8> = (0..n).map(|_| next() as u8).collect();
            for v in [DwgVersion::R2000, DwgVersion::R2018] {
                let _ = items(&d, v);
                let mut r = BitReader::new(&d);
                let _ = read(&mut r, v, 1, None);
            }
        }
    }
}
