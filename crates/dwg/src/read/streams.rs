//! The three streams an object (or the R2007+ header) is split into: main
//! data, strings (R2007+) and handles.
//!
//! Written from knowledge of the spec (chapter 20, "common object format",
//! R2007+ string stream). VERIFY against the spec PDF and real files.

use super::version::DwgVersion;
use crate::bits::{BitReader, HandleRef};
use crate::read::error::{Error, Result};
use crate::read::model::Color;

pub(crate) struct Streams<'a> {
    pub ver: DwgVersion,
    pub codepage: u16,
    /// How text becomes strings (the file's, or the codec's own).
    pub text: crate::records::text::TextDecoding,
    pub dat: BitReader<'a>,
    /// R2007+ string stream; `None` when the object stores no strings.
    pub strs: Option<BitReader<'a>>,
    /// Separate handle stream; `None` means handles are inline in `dat`
    /// (the R13–R2004 header section).
    pub hdl: Option<BitReader<'a>>,
}

impl<'a> Streams<'a> {
    /// TV: 8-bit text in the main stream up to R2004, UTF-16 text in the
    /// string stream from R2007.
    pub fn tv(&mut self) -> Result<String> {
        if self.ver.has_string_stream() {
            match self.strs.as_mut() {
                Some(s) => Ok((self.text.unicode)(s.text16()?)),
                None => Ok(String::new()),
            }
        } else {
            let raw = self.dat.text8()?;
            Ok((self.text.ansi)(&raw, self.codepage))
        }
    }

    /// Next handle reference from the handle stream (or inline).
    pub fn h(&mut self) -> Result<HandleRef> {
        match self.hdl.as_mut() {
            Some(h) => h.handle(),
            None => self.dat.handle(),
        }
    }

    /// Handle resolved against the owning object's handle.
    pub fn h_abs(&mut self, own: u64) -> Result<u64> {
        Ok(self.h()?.absolute(own))
    }

    /// Bits left in the handle stream (or main stream when inline).
    pub fn handle_bits_left(&self) -> usize {
        match &self.hdl {
            Some(h) => h.remaining(),
            None => self.dat.remaining(),
        }
    }

    /// CMC (§2.11): BS index up to R2000; from R2004 a BS index, a BL
    /// method+RGB word and a flag byte announcing optional names.
    pub fn cmc(&mut self) -> Result<(Color, i16)> {
        let (color, index, _, _) = self.cmc_named()?;
        Ok((color, index))
    }

    /// A CMC color with the names a color book color carries from R2004 on: the color's name
    /// in its book (`RAL 1006`, flag 1) and the book's (`RAL CLASSIC`, flag 2).
    pub fn cmc_named(&mut self) -> Result<(Color, i16, Option<String>, Option<String>)> {
        let index = self.dat.bs()? as i16;
        if self.ver < DwgVersion::R2004 {
            return Ok((Color::from_aci(index), index, None, None));
        }
        let word = self.dat.bl()?;
        let flags = self.dat.rc()?;
        let name = if flags & 1 != 0 { Some(self.tv()?) } else { None };
        let book = if flags & 2 != 0 { Some(self.tv()?) } else { None };
        Ok((Color::from_method_word(word, index), index, name, book))
    }

    /// ENC entity color (§2.11, R2004+). Returns the color, the raw
    /// transparency word if stored, and whether a color-book handle follows
    /// in the handle stream.
    pub fn enc(&mut self) -> Result<(Color, Option<u32>, bool)> {
        let raw = self.dat.bs()?;
        if self.ver < DwgVersion::R2004 {
            return Ok((Color::from_aci(raw as i16), None, false));
        }
        let index = (raw & 0x01FF) as i16;
        let mut color = Color::from_aci(index);
        // A color book color (0x4000, with 0x8000) is a DBCOLOR object the handle stream names:
        // its true color is the object's, and none is stored here (AutoCAD's own files go
        // straight on to the linetype scale).
        let book = raw & 0x4000 != 0;
        if raw & 0x8000 != 0 && !book {
            let word = self.dat.bl()?;
            color = Color::from_method_word(word, index);
        }
        let transparency = if raw & 0x2000 != 0 { Some(self.dat.bl()?) } else { None };
        Ok((color, transparency, book))
    }

    /// Guard against absurd counts read from corrupt data: `n` items, each
    /// needing at least `min_bits` in the main stream.
    pub fn check_count(&self, n: u32, min_bits: usize, what: &str) -> Result<usize> {
        let n = n as usize;
        if n.saturating_mul(min_bits) > self.dat.remaining() {
            return Err(Error::format(format!("{what} count {n} exceeds remaining data ({} bits)", self.dat.remaining())));
        }
        Ok(n)
    }

    /// Same guard for handle lists.
    pub fn check_handle_count(&self, n: u32, what: &str) -> Result<usize> {
        let n = n as usize;
        if n.saturating_mul(8) > self.handle_bits_left() {
            return Err(Error::format(format!("{what} handle count {n} exceeds remaining handle data")));
        }
        Ok(n)
    }
}

/// Locate the R2007+ string stream that ends just before bit `endbit` of
/// `data`. The bit at `endbit - 1` says whether strings are present; the 16
/// bits before it hold the stream size in bits, with 0x8000 flagging a
/// further 16-bit high part before that. Returns the string reader and the
/// bit where the strings begin.
pub(crate) fn string_stream(data: &[u8], endbit: usize) -> Result<Option<(BitReader<'_>, usize)>> {
    if endbit == 0 || endbit > data.len() * 8 {
        return Err(Error::format(format!("string stream end bit {endbit} out of range")));
    }
    let mut r = BitReader::with_bounds(data, 0, endbit);
    r.set_position(endbit - 1)?;
    if !r.b()? {
        return Ok(None);
    }
    let mut p = endbit.checked_sub(17).ok_or_else(|| Error::format("string stream flag without size"))?;
    r.set_position(p)?;
    let mut size = r.rs()? as usize;
    if size & 0x8000 != 0 {
        p = p.checked_sub(16).ok_or_else(|| Error::format("string stream size underflow"))?;
        r.set_position(p)?;
        let hi = r.rs()? as usize;
        size = (size & 0x7FFF) | (hi << 15);
    }
    let start = p.checked_sub(size).ok_or_else(|| Error::format(format!("string stream of {size} bits does not fit")))?;
    Ok(Some((BitReader::with_bounds(data, start, p), start)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bits::BitWriter;

    fn streams(ver: DwgVersion, data: &[u8]) -> Streams<'_> {
        Streams { ver, codepage: 30, text: Default::default(), dat: BitReader::new(data), strs: None, hdl: None }
    }

    /// AutoCAD's own files: a colour book colour (0xC02A, colour 42 of a book) stores no true
    /// colour after its flags; the linetype scale follows at once. Reading one used to take the
    /// next 32 bits as the colour and lose the entity.
    #[test]
    fn a_color_book_color_stores_no_true_color() {
        for ver in [DwgVersion::R2004, DwgVersion::R2010, DwgVersion::R2018] {
            let mut w = BitWriter::new();
            w.bs(0xC02A);
            w.bd(1.0);
            w.bb(0);
            let bytes = w.into_bytes();
            let mut s = streams(ver, &bytes);
            let (color, transparency, book) = s.enc().expect("enc");
            assert_eq!(color, Color::from_aci(42), "{ver:?}");
            assert_eq!(transparency, None);
            assert!(book, "{ver:?}: the DBCOLOR handle follows in the handle stream");
            assert_eq!(s.dat.bd().expect("linetype scale"), 1.0, "{ver:?}");
            assert_eq!(s.dat.bb().expect("linetype flags"), 0);
        }
    }

    /// A true colour (0x8000 alone) does store its colour, and transparency (0x2000) follows it.
    #[test]
    fn a_true_color_stores_its_word() {
        let mut w = BitWriter::new();
        w.bs(0xA000);
        w.bl(0xC2E2_9000);
        w.bl(0x0200_0033);
        w.bd(1.0);
        let bytes = w.into_bytes();
        let mut s = streams(DwgVersion::R2004, &bytes);
        let (color, transparency, book) = s.enc().expect("enc");
        assert_eq!(color, Color::from_method_word(0xC2E2_9000, 0));
        assert_eq!(transparency, Some(0x0200_0033));
        assert!(!book);
        assert_eq!(s.dat.bd().expect("linetype scale"), 1.0);
    }

    /// A DBCOLOR object, as AutoCAD saves it: index 0, the true colour, flags 3, the colour's
    /// name, the book's.
    #[test]
    fn cmc_names_of_a_color_book_color() {
        let mut w = BitWriter::new();
        w.bs(0);
        w.bl(0xC2E2_9000);
        w.rc(3);
        for text in ["RAL 1006", "RAL CLASSIC"] {
            w.bs(u16::try_from(text.len()).expect("short"));
            w.raw_bytes(text.as_bytes());
        }
        let bytes = w.into_bytes();
        let mut s = streams(DwgVersion::R2004, &bytes);
        let (color, index, name, book) = s.cmc_named().expect("cmc");
        assert_eq!((color, index), (Color::from_method_word(0xC2E2_9000, 0), 0));
        assert_eq!(name.as_deref(), Some("RAL 1006"));
        assert_eq!(book.as_deref(), Some("RAL CLASSIC"));
    }
}
