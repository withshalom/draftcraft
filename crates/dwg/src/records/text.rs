//! Text helpers: 8-bit code page decoding, AutoCAD escape sequences and
//! MTEXT inline-format stripping.

/// Windows-1252 characters for bytes 0x80–0x9F (the rest of the upper half
/// matches Latin-1). Unassigned positions map to the C1 control of the same
/// value.
const CP1252_HIGH: [u32; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D, 0x017D, 0x008F, 0x0090, 0x2018,
    0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
];

/// Name for the DWGCODEPAGE value stored in the file header, where known.
pub fn codepage_name(cp: u16) -> &'static str {
    code_page_name(cp).unwrap_or("unmapped (decoded as ANSI_1252)")
}

/// The DXF name (`$DWGCODEPAGE`) of the code page number a DWG file stores for its 8-bit text
/// (1 to 44), or `None` for a number with no name. Where two numbers name one code page
/// (22 and 38, Japanese), each keeps the name AutoCAD gives it.
pub fn code_page_name(cp: u16) -> Option<&'static str> {
    Some(match cp {
        1 => "ASCII",
        2 => "ISO8859-1",
        3 => "ISO8859-2",
        4 => "ISO8859-3",
        5 => "ISO8859-4",
        6 => "ISO8859-5",
        7 => "ISO8859-6",
        8 => "ISO8859-7",
        9 => "ISO8859-8",
        10 => "ISO8859-9",
        11 => "DOS437",
        12 => "DOS850",
        13 => "DOS852",
        14 => "DOS855",
        15 => "DOS857",
        16 => "DOS860",
        17 => "DOS861",
        18 => "DOS863",
        19 => "DOS864",
        20 => "DOS865",
        21 => "DOS869",
        22 => "DOS932",
        23 => "MACINTOSH",
        24 => "BIG5",
        25 => "KSC5601",
        26 => "JOHAB",
        27 => "DOS866",
        28 => "ANSI_1250",
        29 => "ANSI_1251",
        30 => "ANSI_1252",
        31 => "GB2312",
        32 => "ANSI_1253",
        33 => "ANSI_1254",
        34 => "ANSI_1255",
        35 => "ANSI_1256",
        36 => "ANSI_1257",
        37 => "ANSI_874",
        38 => "ANSI_932",
        39 => "ANSI_936",
        40 => "ANSI_949",
        41 => "ANSI_950",
        42 => "ANSI_1361",
        43 => "UTF8",
        44 => "ANSI_1258",
        _ => return None,
    })
}

/// How the reader turns a file's text into strings: 8-bit text (up to AutoCAD 2004), given its
/// bytes and the code page number it is in, and UTF-16 text (from 2007), given as decoded. The
/// codec's own ([`TextDecoding::default`]) reads 8-bit text as Windows-1252 with `\U+XXXX`
/// escapes expanded and keeps UTF-16 text as stored; a caller that knows the code pages and the
/// rest of the escapes sets its own on the file, so the text of both formats reads one way.
#[derive(Clone, Copy, Debug)]
pub struct TextDecoding {
    pub ansi: fn(&[u8], u16) -> String,
    pub unicode: fn(String) -> String,
}

impl Default for TextDecoding {
    fn default() -> Self {
        TextDecoding { ansi: decode_ansi, unicode: |s| s }
    }
}

/// Decode an 8-bit DWG string (R13–R2004). Every code page is decoded as
/// Windows-1252 for now; `\U+XXXX` escapes, which AutoCAD uses for
/// characters outside the code page, are expanded.
pub fn decode_ansi(bytes: &[u8], _codepage: u16) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        if b == 0 {
            break;
        }
        let c = match b {
            0x80..=0x9F => CP1252_HIGH[(b - 0x80) as usize],
            _ => b as u32,
        };
        s.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
    }
    expand_unicode_escapes(&s)
}

/// Decode Windows-1252 bytes one character per byte, keeping NULs and
/// leaving `\U+XXXX` escapes as they are. Every byte maps to a distinct
/// character, so the bytes can be recovered exactly.
pub fn decode_cp1252_exact(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            let c = match b {
                0x80..=0x9F => CP1252_HIGH[(b - 0x80) as usize],
                _ => b as u32,
            };
            char::from_u32(c).unwrap_or('\u{FFFD}')
        })
        .collect()
}

/// Replace `\U+XXXX` (four hex digits) with the character it names.
pub fn expand_unicode_escapes(s: &str) -> String {
    if !s.contains("\\U+") && !s.contains("\\u+") {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    // The four hex digits of a `\U+XXXX` escape at `i`.
    let unit = |i: usize| -> Option<u32> {
        let head = chars.get(i..i + 3)?;
        if head[0] != '\\' || !matches!(head[1], 'U' | 'u') || head[2] != '+' {
            return None;
        }
        let hex: String = chars.get(i + 3..i + 7)?.iter().collect();
        u32::from_str_radix(&hex, 16).ok()
    };
    while i < chars.len() {
        if let Some(v) = unit(i) {
            if let Some(c) = char::from_u32(v) {
                out.push(c);
                i += 7;
                continue;
            }
            // A character past U+FFFF is two escapes, a UTF-16 surrogate pair.
            if (0xD800..0xDC00).contains(&v)
                && let Some(low) = unit(i + 7).filter(|u| (0xDC00..0xE000).contains(u))
                && let Some(c) = char::from_u32(0x10000 + ((v - 0xD800) << 10) + (low - 0xDC00))
            {
                out.push(c);
                i += 14;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Expand the `%%` control codes used in single-line TEXT: %%c (diameter),
/// %%d (degree), %%p (plus/minus), %%% (percent), %%nnn (character code).
/// Underline/overline toggles (%%u, %%o) are dropped.
pub fn expand_percent_codes(s: &str) -> String {
    if !s.contains("%%") {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%'
            && chars.get(i + 1) == Some(&'%')
            && let Some(&c) = chars.get(i + 2)
        {
            match c.to_ascii_lowercase() {
                'c' => {
                    out.push('\u{2300}');
                    i += 3;
                    continue;
                }
                'd' => {
                    out.push('\u{00B0}');
                    i += 3;
                    continue;
                }
                'p' => {
                    out.push('\u{00B1}');
                    i += 3;
                    continue;
                }
                '%' => {
                    out.push('%');
                    i += 3;
                    continue;
                }
                'u' | 'o' | 'k' => {
                    i += 3;
                    continue;
                }
                d if d.is_ascii_digit() => {
                    let mut j = i + 2;
                    let mut v: u32 = 0;
                    while j < chars.len() && j < i + 5 && chars[j].is_ascii_digit() {
                        v = v * 10 + chars[j].to_digit(10).unwrap_or(0);
                        j += 1;
                    }
                    if let Some(ch) = char::from_u32(v) {
                        out.push(ch);
                    }
                    i = j;
                    continue;
                }
                _ => {}
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Reduce MTEXT contents to plain text: paragraphs become newlines and
/// formatting codes are removed. Stacked fractions render as "a/b".
pub fn strip_mtext(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '{' | '}' => {
                i += 1;
            }
            '\\' if i + 1 < chars.len() => {
                let code = chars[i + 1];
                match code {
                    'P' | 'X' => {
                        out.push('\n');
                        i += 2;
                    }
                    'N' => {
                        out.push('\n');
                        i += 2;
                    }
                    '~' => {
                        out.push('\u{00A0}');
                        i += 2;
                    }
                    '\\' | '{' | '}' => {
                        out.push(code);
                        i += 2;
                    }
                    'L' | 'l' | 'O' | 'o' | 'K' | 'k' => {
                        i += 2;
                    }
                    'U' | 'u' if chars.get(i + 2) == Some(&'+') => {
                        // Leave for the unicode pass.
                        out.push('\\');
                        i += 1;
                    }
                    'S' => {
                        // \Snum^den; or \Snum/den; or \Snum#den;
                        let mut j = i + 2;
                        let mut frac = String::new();
                        while j < chars.len() && chars[j] != ';' {
                            let ch = chars[j];
                            frac.push(if ch == '^' || ch == '#' { '/' } else { ch });
                            j += 1;
                        }
                        out.push_str(frac.trim());
                        i = (j + 1).min(chars.len());
                    }
                    'A' | 'C' | 'c' | 'F' | 'f' | 'H' | 'Q' | 'T' | 'W' | 'p' => {
                        // Parameterised code terminated by ';'.
                        let mut j = i + 2;
                        while j < chars.len() && chars[j] != ';' {
                            j += 1;
                        }
                        i = (j + 1).min(chars.len());
                    }
                    _ => {
                        i += 2;
                    }
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    expand_percent_codes(&expand_unicode_escapes(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_and_escapes() {
        assert_eq!(decode_ansi(b"R\x80 5\xB0", 30), "R€ 5°");
        assert_eq!(decode_ansi(b"A\\U+00D8B", 30), "AØB");
        assert_eq!(decode_ansi(b"abc\0junk", 30), "abc");
    }

    #[test]
    fn percent_codes() {
        assert_eq!(expand_percent_codes("%%c25 %%p0.1 45%%d 100%%%"), "⌀25 ±0.1 45° 100%");
        assert_eq!(expand_percent_codes("%%uUNDER%%u"), "UNDER");
    }

    #[test]
    fn mtext() {
        assert_eq!(strip_mtext("{\\fArial|b1|i0;Plate}\\PQTY 2"), "Plate\nQTY 2");
        assert_eq!(strip_mtext("\\H2.5x;\\S1/2;\" hole"), "1/2\" hole");
        assert_eq!(strip_mtext("a\\\\b\\{c\\}"), "a\\b{c}");
    }
}
