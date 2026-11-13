//! AutoCAD 2007 container (AC1021): a file header, a page map and a section map stored as
//! Reed-Solomon codewords, and data pages that are compressed with the format's own scheme and
//! then stored as codewords too.
//!
//! Spec chapter 5. The reader takes the data bytes of each codeword and leaves the parity alone:
//! a file whose bytes are damaged decodes to damaged sections, which the object reader reports
//! object by object, as in every other version.

use super::container::Container;
use super::version::DwgVersion;
use crate::read::error::{Error, Result};
use crate::read::model::{Diagnostic, SectionInfo, Severity};
use std::collections::HashMap;

/// Where the pages start: every page offset of the file is counted from here.
const PAGES_AT: u64 = 0x480;
/// The file header's codewords: three of 255 bytes, interleaved, at 0x80.
const HEADER_AT: usize = 0x80;
const HEADER_CODEWORDS: usize = 3;
/// The header's fields once decompressed: 34 words of 64 bits.
const HEADER_FIELDS: usize = 0x110;
/// A codeword is 255 bytes: 239 of data in the header and the two maps, 251 in a data page.
const CODEWORD: usize = 255;
const SYSTEM_DATA: usize = 239;
const PAGE_DATA: usize = 251;
/// What a page map or a section map may decompress to.
const MAX_SYSTEM_PAGE: u64 = 64 << 20;
/// Refuse to allocate absurd section sizes from corrupt maps.
const MAX_SECTION: u64 = 2 * 1024 * 1024 * 1024;
/// What all of a file's sections together may declare, as a multiple of the file's size (the
/// 2004 container's rule: `r2004::MAX_EXPANSION`).
const MAX_EXPANSION: u64 = 64;
const MIN_BUDGET: u64 = 64 << 20;

fn u64_at(d: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(o..o.checked_add(8)?)?.try_into().ok()?))
}

/// The data bytes of `blocks` interleaved codewords: byte `j` of codeword `i` is at
/// `i + j * blocks`. `data` bytes are taken from each, `len` in all.
fn deinterleave(encoded: &[u8], blocks: usize, data: usize, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    for i in 0..blocks {
        for j in 0..data {
            if out.len() == len {
                return out;
            }
            out.push(encoded.get(i + j * blocks).copied().unwrap_or(0));
        }
    }
    out.resize(len, 0);
    out
}

/// The order a run of literal bytes is stored in (spec §5.2): a run is cut into pieces of at most
/// 32 bytes, and each piece is stored as these parts of it, `(start, length)`, one after the
/// other. A part of 2 or 3 bytes is stored back to front.
const LITERAL_ORDER: [&[(u8, u8)]; 33] = [
    &[],
    &[(0, 1)],
    &[(0, 2)],
    &[(0, 3)],
    &[(0, 4)],
    &[(4, 1), (0, 4)],
    &[(5, 1), (1, 4), (0, 1)],
    &[(5, 2), (1, 4), (0, 1)],
    &[(0, 8)],
    &[(8, 1), (0, 8)],
    &[(9, 1), (1, 8), (0, 1)],
    &[(9, 2), (1, 8), (0, 1)],
    &[(8, 4), (0, 8)],
    &[(12, 1), (8, 4), (0, 8)],
    &[(13, 1), (9, 4), (1, 8), (0, 1)],
    &[(13, 2), (9, 4), (1, 8), (0, 1)],
    &[(8, 8), (0, 8)],
    &[(9, 8), (8, 1), (0, 8)],
    &[(17, 1), (9, 8), (1, 8), (0, 1)],
    &[(16, 3), (8, 8), (0, 8)],
    &[(16, 4), (8, 8), (0, 8)],
    &[(20, 1), (16, 4), (8, 8), (0, 8)],
    &[(20, 2), (16, 4), (8, 8), (0, 8)],
    &[(20, 3), (16, 4), (8, 8), (0, 8)],
    &[(16, 8), (8, 8), (0, 8)],
    &[(17, 8), (16, 1), (8, 8), (0, 8)],
    &[(25, 1), (17, 8), (16, 1), (8, 8), (0, 8)],
    &[(25, 2), (17, 8), (16, 1), (8, 8), (0, 8)],
    &[(24, 4), (16, 8), (8, 8), (0, 8)],
    &[(28, 1), (24, 4), (16, 8), (8, 8), (0, 8)],
    &[(28, 2), (24, 4), (16, 8), (8, 8), (0, 8)],
    &[(30, 1), (26, 4), (18, 8), (10, 8), (2, 8), (0, 2)],
    &[(24, 8), (16, 8), (8, 8), (0, 8)],
];

/// One compressed stream being read.
struct Packed<'a> {
    src: &'a [u8],
    pos: usize,
}

impl Packed<'_> {
    fn byte(&mut self) -> Result<usize> {
        let b = *self.src.get(self.pos).ok_or_else(|| Error::truncated("compressed data ends in the middle of an instruction"))?;
        self.pos += 1;
        Ok(usize::from(b))
    }

    /// The length of a literal run that the opcode `op` starts.
    fn literal_length(&mut self, op: usize) -> Result<usize> {
        let mut len = op + 8;
        if len == 0x17 {
            let mut n = self.byte()?;
            len += n;
            if n == 0xFF {
                loop {
                    n = self.byte()? | (self.byte()? << 8);
                    len = len.saturating_add(n);
                    if n != 0xFFFF {
                        break;
                    }
                }
            }
        }
        Ok(len)
    }

    /// A copy of earlier output, by the opcode `op`: its length, how far back it starts, and the
    /// opcode that follows it.
    fn copy(&mut self, op: usize) -> Result<(usize, usize, usize)> {
        Ok(match op >> 4 {
            0 => {
                let low = self.byte()?;
                let next = self.byte()?;
                ((op & 0x0F) + 0x13 + ((next >> 3) & 0x10), ((next & 0x78) << 5) + 1 + low, next)
            }
            1 => {
                let low = self.byte()?;
                let next = self.byte()?;
                ((op & 0x0F) + 3, ((next & 0xF8) << 5) + 1 + low, next)
            }
            2 => {
                let offset = self.byte()? | (self.byte()? << 8);
                let len = op & 7;
                if op & 8 == 0 {
                    let next = self.byte()?;
                    (len + (next & 0xF8), offset, next)
                } else {
                    let mid = self.byte()?;
                    let next = self.byte()?;
                    (len + (mid << 3) + ((next & 0xF8) << 8) + 0x100, offset + 1, next)
                }
            }
            n => {
                let next = self.byte()?;
                (n, ((next & 0xF8) << 1) + (op & 0x0F) + 1, next)
            }
        })
    }
}

/// Decompress `src` (spec §5.2) to `len` bytes.
pub(crate) fn decompress(src: &[u8], len: usize) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(len);
    let mut p = Packed { src, pos: 0 };
    if src.is_empty() {
        out.resize(len, 0);
        return Ok(out);
    }
    let mut op = p.byte()?;
    let mut literal = 0usize;
    if op & 0xF0 == 0x20 {
        p.pos += 2;
        literal = p.byte()? & 7;
    }
    while p.pos < src.len() && out.len() < len {
        if literal == 0 {
            literal = p.literal_length(op)?;
        }
        // The literal run, a piece of at most 32 bytes at a time.
        while literal > 0 && out.len() < len {
            let n = literal.min(32);
            let piece = src.get(p.pos..p.pos + n).ok_or_else(|| Error::truncated("compressed data ends in the middle of a literal run"))?;
            for &(start, count) in LITERAL_ORDER.get(n).copied().unwrap_or(&[]) {
                let part = piece.get(usize::from(start)..usize::from(start) + usize::from(count)).unwrap_or(&[]);
                if count == 2 || count == 3 {
                    out.extend(part.iter().rev());
                } else {
                    out.extend_from_slice(part);
                }
            }
            p.pos += n;
            literal -= n;
        }
        if p.pos >= src.len() || out.len() >= len {
            break;
        }
        // Copies of earlier output, until one is followed by a literal run.
        op = p.byte()?;
        loop {
            let (count, back, next) = p.copy(op)?;
            op = next;
            if back > out.len() {
                return Err(Error::format(format!("compressed data copies from {back} bytes back, with {} bytes written", out.len())));
            }
            let from = out.len() - back;
            for i in 0..count.min(len - out.len()) {
                let b = out.get(from + i).copied().unwrap_or(0);
                out.push(b);
            }
            literal = op & 7;
            if literal != 0 || p.pos >= src.len() || out.len() >= len {
                break;
            }
            op = p.byte()?;
            match op >> 4 {
                0 => break,
                15 => op &= 15,
                _ => {}
            }
        }
    }
    out.resize(len, 0);
    Ok(out)
}

/// A page stored as interleaved codewords at `offset` from the pages' start: `compressed` bytes
/// that decompress to `uncompressed`, in codewords of `data` data bytes, `repeat` times over.
fn page(file: &[u8], offset: u64, compressed: u64, uncompressed: u64, repeat: u64, data: usize, limit: u64) -> Result<Vec<u8>> {
    if compressed == 0 || compressed > limit || uncompressed > limit {
        return Err(Error::format(format!("page of {compressed} bytes for {uncompressed} is implausible")));
    }
    let total =
        ((compressed + 7) & !7).checked_mul(repeat.max(1)).filter(|t| *t <= limit).ok_or_else(|| Error::format("page size is implausible"))?;
    let total = usize::try_from(total).map_err(|_| Error::format("page size is implausible"))?;
    let blocks = total.div_ceil(data);
    let at = usize::try_from(PAGES_AT.saturating_add(offset)).map_err(|_| Error::format("page address overflow"))?;
    if at >= file.len() {
        return Err(Error::truncated(format!("page at {at:#x} is past the end of the file")));
    }
    let end = at.saturating_add(blocks.saturating_mul(CODEWORD)).min(file.len());
    let encoded = file.get(at..end).unwrap_or(&[]);
    let mut bytes = deinterleave(encoded, blocks, data, total);
    let compressed = usize::try_from(compressed).map_err(|_| Error::format("page size is implausible"))?;
    let uncompressed = usize::try_from(uncompressed).map_err(|_| Error::format("page size is implausible"))?;
    if compressed == uncompressed {
        bytes.truncate(uncompressed);
        return Ok(bytes);
    }
    decompress(bytes.get(..compressed).unwrap_or(&bytes), uncompressed)
}

struct PageRef {
    /// Where the page's data starts in its section.
    start: u64,
    id: u64,
    uncompressed: u64,
    compressed: u64,
}

struct SectionDesc {
    name: String,
    size: u64,
    encrypted: u64,
    /// 4: compressed pages in codewords; 1: pages stored as they are.
    encoding: u64,
    pages: Vec<PageRef>,
}

/// Reassemble one section from its pages, taking its size from what is left of `budget`.
fn assemble(file: &[u8], d: &SectionDesc, pages: &HashMap<u64, u64>, budget: &mut u64) -> Result<Vec<u8>> {
    if d.encrypted != 0 {
        return Err(Error::new(crate::read::error::ErrorKind::Unsupported, "section is encrypted (password-protected drawing)"));
    }
    if d.size > MAX_SECTION {
        return Err(Error::format(format!("section size {} is implausible", d.size)));
    }
    if d.size > *budget {
        return Err(Error::format(format!(
            "section size {} is more than a file of {} bytes holds (its sections may hold {} more)",
            d.size,
            file.len(),
            *budget
        )));
    }
    *budget -= d.size;
    let mut buf = vec![0u8; usize::try_from(d.size).map_err(|_| Error::format("section size is implausible"))?];
    for pr in &d.pages {
        let offset = *pages.get(&pr.id).ok_or_else(|| Error::format(format!("page {} not in page map", pr.id)))?;
        let data = if d.encoding == 1 {
            let at = usize::try_from(PAGES_AT.saturating_add(offset)).map_err(|_| Error::format("page address overflow"))?;
            let n = usize::try_from(pr.uncompressed).map_err(|_| Error::format("page size is implausible"))?;
            file.get(at..at.saturating_add(n)).ok_or_else(|| Error::truncated(format!("page {} past end of file", pr.id)))?.to_vec()
        } else {
            page(file, offset, pr.compressed, pr.uncompressed, 1, PAGE_DATA, d.size.max(1 << 20)).map_err(|e| e.context(format!("page {}", pr.id)))?
        };
        let Ok(start) = usize::try_from(pr.start) else { continue };
        if let Some(room) = buf.get_mut(start..) {
            let n = data.len().min(room.len());
            if let (Some(to), Some(from)) = (room.get_mut(..n), data.get(..n)) {
                to.copy_from_slice(from);
            }
        }
    }
    Ok(buf)
}

pub(crate) fn read(file: &[u8], ver: DwgVersion) -> Result<Container> {
    let short = || Error::truncated("file is shorter than the AutoCAD 2007 file header");
    let maintenance = *file.get(0x0B).ok_or_else(short)?;
    let codepage = u16::from_le_bytes([*file.get(0x13).ok_or_else(short)?, *file.get(0x14).ok_or_else(short)?]);
    let mut notes = Vec::new();

    // The file header: three codewords, then a compressed block of 34 words.
    let encoded = file.get(HEADER_AT..HEADER_AT + HEADER_CODEWORDS * CODEWORD).ok_or_else(short)?;
    let block = deinterleave(encoded, HEADER_CODEWORDS, SYSTEM_DATA, HEADER_CODEWORDS * SYSTEM_DATA);
    let stored = i32::from_le_bytes(block.get(24..28).and_then(|b| b.try_into().ok()).unwrap_or([0; 4]));
    let body = block.get(32..).unwrap_or(&[]);
    let hdr = if stored < 0 {
        // Not compressed: the fields as they are.
        let mut raw = body.get(..(stored.unsigned_abs() as usize).min(body.len())).unwrap_or(&[]).to_vec();
        raw.resize(HEADER_FIELDS, 0);
        raw
    } else {
        decompress(body.get(..(stored as usize).min(body.len())).unwrap_or(&[]), HEADER_FIELDS)
            .map_err(|e| e.context(format!("{} file header", ver.magic())))?
    };
    let field = |i: usize| u64_at(&hdr, i * 8).unwrap_or(0);
    let (page_map_repeat, page_map_offset) = (field(3), field(7));
    let (page_map_compressed, page_map_uncompressed) = (field(10), field(11));
    let (section_map_compressed, section_map_id, section_map_uncompressed, section_map_repeat) = (field(22), field(24), field(25), field(27));

    // Page map: each page's size and id, in the file's order.
    let pm = page(file, page_map_offset, page_map_compressed, page_map_uncompressed, page_map_repeat, SYSTEM_DATA, MAX_SYSTEM_PAGE)
        .map_err(|e| e.context("page map"))?;
    let mut pages: HashMap<u64, u64> = HashMap::new();
    let mut offset = 0u64;
    let mut p = 0usize;
    while let (Some(size), Some(id)) = (u64_at(&pm, p), u64_at(&pm, p + 8)) {
        if size == 0 && id == 0 {
            break;
        }
        // A negative id is a free page: its place still counts.
        pages.insert((id as i64).unsigned_abs(), offset);
        offset = offset.saturating_add(size);
        p += 16;
    }

    // Section map.
    let sm_offset = *pages.get(&section_map_id).ok_or_else(|| Error::format(format!("section map page {section_map_id} not in page map")))?;
    let sm = page(file, sm_offset, section_map_compressed, section_map_uncompressed, section_map_repeat, SYSTEM_DATA, MAX_SYSTEM_PAGE)
        .map_err(|e| e.context("section map"))?;
    let mut descs: Vec<SectionDesc> = Vec::new();
    let mut q = 0usize;
    while descs.len() < 4096 {
        let Some(size) = u64_at(&sm, q) else { break };
        let (Some(encrypted), Some(name_len), Some(encoding), Some(page_count)) =
            (u64_at(&sm, q + 16), u64_at(&sm, q + 32), u64_at(&sm, q + 48), u64_at(&sm, q + 56))
        else {
            break;
        };
        q += 64;
        let name_len = usize::try_from(name_len).unwrap_or(usize::MAX);
        let Some(name_bytes) = q.checked_add(name_len).and_then(|end| sm.get(q..end)) else { break };
        let units: Vec<u16> = name_bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        let name = String::from_utf16_lossy(&units).trim_end_matches('\0').to_string();
        q += name_len;
        let mut page_refs = Vec::new();
        for _ in 0..page_count.min(1 << 20) {
            let (Some(start), Some(id), Some(uncompressed), Some(compressed)) =
                (u64_at(&sm, q), u64_at(&sm, q + 16), u64_at(&sm, q + 24), u64_at(&sm, q + 32))
            else {
                break;
            };
            if sm.len() < q + 56 {
                break;
            }
            q += 56;
            page_refs.push(PageRef { start, id: (id as i64).unsigned_abs(), uncompressed, compressed });
        }
        if name_len > 0 {
            descs.push(SectionDesc { name, size, encrypted, encoding, pages: page_refs });
        }
    }

    let sections: Vec<SectionInfo> = descs
        .iter()
        .map(|d| SectionInfo {
            name: d.name.clone(),
            size: d.size,
            pages: d.pages.len() as u32,
            compressed: d.encoding == 4,
            encrypted: d.encrypted != 0,
        })
        .collect();

    let mut budget = (file.len() as u64).saturating_mul(MAX_EXPANSION).max(MIN_BUDGET);
    let mut load = |name: &str| -> Result<Option<Vec<u8>>> {
        match descs.iter().find(|d| d.name == name) {
            None => Ok(None),
            Some(d) => assemble(file, d, &pages, &mut budget).map(Some).map_err(|e| e.context(name)),
        }
    };
    let mut soft = |name: &str, notes: &mut Vec<Diagnostic>| {
        load(name).unwrap_or_else(|e| {
            notes.push(Diagnostic { severity: Severity::Warning, message: e.to_string() });
            None
        })
    };
    let header = soft("AcDb:Header", &mut notes);
    let classes = soft("AcDb:Classes", &mut notes);
    let handles = load("AcDb:Handles")?.ok_or_else(|| Error::format("no AcDb:Handles section"))?;
    let objects = load("AcDb:AcDbObjects")?.ok_or_else(|| Error::format("no AcDb:AcDbObjects section"))?;
    let mut others = Vec::new();
    for d in &descs {
        if matches!(d.name.as_str(), "AcDb:Header" | "AcDb:Classes" | "AcDb:Handles" | "AcDb:AcDbObjects") {
            continue;
        }
        match assemble(file, d, &pages, &mut budget) {
            Ok(bytes) => others.push((d.name.clone(), bytes)),
            Err(e) => notes.push(Diagnostic { severity: Severity::Info, message: format!("{}: {e}", d.name) }),
        }
    }
    Ok(Container { maintenance, codepage, header, classes, handles, objects, sections, others, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codewords_give_their_data_bytes_in_order() {
        // Two codewords interleaved: 1 2 3 and 4 5 6, then parity.
        assert_eq!(deinterleave(&[1, 4, 2, 5, 3, 6, 99, 99], 2, 3, 6), [1, 2, 3, 4, 5, 6]);
        // A short input reads as zeros, never past its end.
        assert_eq!(deinterleave(&[1, 4], 2, 3, 6), [1, 0, 0, 4, 0, 0]);
    }

    /// Every literal piece's order names each of its bytes once.
    #[test]
    fn a_literal_piece_holds_each_byte_once() {
        for (n, parts) in LITERAL_ORDER.iter().enumerate() {
            let mut seen = vec![0u8; n];
            for &(start, count) in *parts {
                for i in start..start + count {
                    seen[usize::from(i)] += 1;
                }
            }
            assert!(seen.iter().all(|c| *c == 1), "{n}: {seen:?}");
        }
    }

    #[test]
    fn literals_and_copies_decompress() {
        // Opcode 0: a literal run of 8 bytes (stored in order), then a copy.
        // Copy opcode 0x35, next 0x00: length 3, from (0 << 1) + 5 + 1 = 6 back; no literal follows.
        let src = [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 0x35, 0x00];
        assert_eq!(decompress(&src, 11).unwrap(), [1, 2, 3, 4, 5, 6, 7, 8, 3, 4, 5]);
    }

    #[test]
    fn damaged_streams_are_errors_not_panics() {
        // A copy from before the start of the output.
        let src = [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 0x3F, 0xF8];
        assert!(decompress(&src, 64).is_err());
        // A literal run longer than the data.
        assert!(decompress(&[0u8, 1, 2], 64).is_err());
        // Every prefix of a stream, and every single byte, ends without a panic.
        let src = [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 0x35, 0x00, 0x17, 0xFF, 0xFF, 0xFF];
        for n in 0..src.len() {
            let _ = decompress(&src[..n], 4096);
        }
        for b in 0..=255u8 {
            let _ = decompress(&[b], 64);
            let _ = decompress(&[b, b, b, b], 64);
        }
    }

    #[test]
    fn a_file_that_ends_early_is_an_error() {
        assert!(read(b"AC1021", DwgVersion::R2007).is_err());
        let mut file = b"AC1021".to_vec();
        file.resize(0x480, 0);
        assert!(read(&file, DwgVersion::R2007).is_err());
    }
}
