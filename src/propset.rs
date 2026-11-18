//! OLE property-set streams: the one place a SolidWorks document keeps
//! parameters in a *documented* format.
//!
//! A document stores two of these, as streams named (in OLE's odd way, with a
//! leading 0x05 byte):
//!
//! * `SummaryInformation` - title, subject, author, keywords, comments,
//!   template, last-saved-by, revision, edit time, created and saved dates,
//!   and the application that wrote the file;
//! * `DocumentSummaryInformation` - category, manager, company, and, in a
//!   second section, the document's **custom properties**: the user-defined
//!   name/value pairs a real shop actually fills in - Material, Mass, Part
//!   Number, Revision, Finish, and so on.
//!
//! This format is Microsoft's, stable and documented, so it is read directly
//! instead of through a dependency. Every read is bounds-checked: these are
//! untrusted files, and a truncated one must error, not panic.
//!
//! What this does **not** do is invent meaning. A property is reported with the
//! name the file gives it and the value it holds; nothing is guessed.

use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// One property: a name and its value, already rendered as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Property {
    pub name: String,
    pub value: String,
}

/// One section of a property-set stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertySet {
    /// `summary`, `document` or `custom`.
    pub group: String,
    pub properties: Vec<Property>,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    let s = b.get(o..o + 8)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Some(u64::from_le_bytes(a))
}

/// Subtracting in favour of a literal `0` rather than a panic.
fn sub(a: usize, b: usize) -> usize {
    a.saturating_sub(b)
}

/// Standard name for a property id in one of the two OLE streams.
fn standard_name(group: &str, id: u32) -> Option<&'static str> {
    const SUMMARY: &[(u32, &str)] = &[
        (2, "Title"),
        (3, "Subject"),
        (4, "Author"),
        (5, "Keywords"),
        (6, "Comments"),
        (7, "Template"),
        (8, "Last saved by"),
        (9, "Revision"),
        (10, "Total edit time"),
        (11, "Last printed"),
        (12, "Created"),
        (13, "Last saved"),
        (14, "Page count"),
        (15, "Word count"),
        (16, "Character count"),
        (18, "Created with"),
        (19, "Security"),
    ];
    const DOCUMENT: &[(u32, &str)] = &[
        (2, "Category"),
        (3, "Presentation target"),
        (4, "Byte count"),
        (5, "Line count"),
        (6, "Paragraph count"),
        (7, "Slide count"),
        (8, "Note count"),
        (9, "Hidden slide count"),
        (10, "Multimedia clip count"),
        (11, "Scale crop"),
        (14, "Manager"),
        (15, "Company"),
        (16, "Links up to date"),
        (17, "Character count with spaces"),
        (18, "Shared document"),
        (19, "Hyperlink base"),
        (21, "Hyperlinks changed"),
        (23, "Application version"),
        (26, "Content type"),
        (27, "Content status"),
        (28, "Language"),
        (29, "Document version"),
    ];
    let table = match group {
        "summary" => SUMMARY,
        _ => DOCUMENT,
    };
    table.iter().find(|(i, _)| *i == id).map(|(_, n)| *n)
}

/// Decode bytes that are either UTF-16LE (codepage 1200) or single-byte text.
fn decode(bytes: &[u8], codepage: u16) -> String {
    if codepage == 1200 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|u| *u != 0)
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        let slice = &bytes[..end];
        match std::str::from_utf8(slice) {
            Ok(s) => s.to_string(),
            // Not valid UTF-8: treat as Latin-1, which never fails.
            Err(_) => slice.iter().map(|b| *b as char).collect(),
        }
    }
}

/// A `FILETIME` (100-nanosecond ticks since 1601) as an ISO-8601 string.
fn filetime(ticks: u64) -> String {
    const EPOCH_DIFF: i64 = 11_644_473_600; // 1601-01-01 to 1970-01-01, in seconds
    let secs = (ticks / 10_000_000) as i64 - EPOCH_DIFF;
    match Utc.timestamp_opt(secs, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        None => format!("{ticks} (ticks)"),
    }
}

/// Read one typed value at `at`, returning it as text.
///
/// Vector types are reported by their element count rather than expanded: a
/// thumbnail is binary, and this is not the place to carry it.
fn read_value(data: &[u8], at: usize, codepage: u16) -> Option<String> {
    const VT_VECTOR: u32 = 0x1000;
    let raw = u32_at(data, at)?;
    let vt = raw & 0x0FFF;
    let is_vector = raw & VT_VECTOR != 0;
    let v = at + 4;

    if is_vector {
        let count = u32_at(data, v)?;
        return Some(format!("<{count} values>"));
    }

    let text = match vt {
        0 => String::new(), // VT_EMPTY
        1 => String::new(), // VT_NULL
        2 => u16_at(data, v).map(|n| (n as i16).to_string())?, // VT_I2
        3 | 22 => u32_at(data, v).map(|n| (n as i32).to_string())?, // VT_I4/VT_INT
        4 => u32_at(data, v).map(|n| f32::from_bits(n).to_string())?, // VT_R4
        5 => u64_at(data, v).map(|n| f64::from_bits(n).to_string())?, // VT_R8
        11 => {
            let n = u16_at(data, v)?;
            (n != 0).to_string()
        } // VT_BOOL
        19 => u32_at(data, v).map(|n| n.to_string())?, // VT_UI4
        30 => {
            // VT_LPSTR: length includes the trailing null.
            let len = u32_at(data, v)? as usize;
            let bytes = data.get(v + 4..v + 4 + len)?;
            decode(bytes, codepage)
        }
        31 | 8 => {
            // VT_LPWSTR / VT_BSTR: length is in characters.
            let len = u32_at(data, v)? as usize;
            let bytes = data.get(v + 4..v + 4 + len * 2)?;
            decode(bytes, 1200)
        }
        64 => u64_at(data, v).map(filetime)?, // VT_FILETIME
        65 => {
            let len = u32_at(data, v)?;
            format!("<binary, {len} bytes>")
        } // VT_BLOB
        72 => {
            let g = data.get(v..v + 16)?;
            format!(
                "{{{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
                g[3], g[2], g[1], g[0], g[5], g[4], g[7], g[6], g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]
            )
        } // VT_CLSID
        _ => format!("<type {vt}>")
    };
    Some(text)
}

/// Parse the custom-properties dictionary (property id 0): id -> name.
fn read_dictionary(data: &[u8], at: usize, codepage: u16) -> Vec<(u32, String)> {
    let Some(count) = u32_at(data, at) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut p = at + 4;
    for _ in 0..count.min(4096) {
        let Some(id) = u32_at(data, p) else { break };
        let Some(len) = u32_at(data, p + 4) else { break };
        let len = len as usize;
        let start = p + 8;
        // The name is `len` characters; when the codepage is 1200 that is
        // two bytes each, otherwise one.
        let bytes = len * if codepage == 1200 { 2 } else { 1 };
        let Some(chunk) = data.get(start..start + bytes) else { break };
        out.push((id, decode(chunk, codepage)));
        p = start + bytes;
        // Strings are padded to a 4-byte boundary.
        p += sub(4, bytes % 4) % 4;
    }
    out
}

/// Parse one section: the codepage, the dictionary, and every value.
fn read_section(data: &[u8], off: usize, group: &str) -> Vec<Property> {
    let Some(num) = u32_at(data, off + 4) else {
        return Vec::new();
    };
    let num = num.min(4096) as usize;

    // Two passes: the dictionary and codepage can come after the values they
    // apply to, so collect the offsets first.
    let mut entries: Vec<(u32, usize)> = Vec::with_capacity(num);
    for i in 0..num {
        let p = off + 8 + i * 8;
        let (Some(id), Some(voff)) = (u32_at(data, p), u32_at(data, p + 4)) else {
            break;
        };
        entries.push((id, off + voff as usize));
    }

    let custom = group == "custom";
    let mut codepage = if custom { 1252 } else { 1252 };
    let mut dictionary: Vec<(u32, String)> = Vec::new();

    // Dictionary and codepage first, so names and encodings are known.
    for (id, at) in &entries {
        if custom && *id == 0 {
            dictionary = read_dictionary(data, *at, 1252);
        } else if *id == 1 {
            // The codepage is always VT_I2.
            if let Some(v) = u16_at(data, *at + 4) {
                codepage = v;
            }
        }
    }

    // The dictionary for a custom section may itself be UTF-16; re-read it now
    // that the codepage is known, when that changes how it should be decoded.
    if custom && codepage == 1200 {
        if let Some((_, at)) = entries.iter().find(|(id, _)| *id == 0) {
            let d = read_dictionary(data, *at, 1200);
            if !d.is_empty() {
                dictionary = d;
            }
        }
    }

    let mut out = Vec::new();
    for (id, at) in entries {
        // 0 is the dictionary, 1 the codepage: neither is a display property.
        if id == 0 || id == 1 {
            continue;
        }
        let name = if custom {
            dictionary
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, n)| n.clone())
        } else {
            standard_name(group, id).map(|s| s.to_string())
        };
        let Some(name) = name else { continue };
        let Some(value) = read_value(data, at, codepage) else { continue };
        if value.is_empty() {
            continue;
        }
        out.push(Property { name, value });
    }
    out
}

/// Read every section of a property-set stream.
///
/// `groups` names the sections in order: a `SummaryInformation` stream has one
/// (`summary`); a `DocumentSummaryInformation` stream has two (`document`, then
/// `custom`). Fails only when the stream is not a property set at all.
pub fn parse(data: &[u8], groups: &[&str]) -> Result<Vec<PropertySet>, String> {
    if u16_at(data, 0) != Some(0xFFFE) {
        return Err("not a property-set stream (byte order signature missing)".into());
    }
    let count = u32_at(data, 24).unwrap_or(0).min(16) as usize;
    if count == 0 {
        return Err("property-set stream declares no sections".into());
    }

    let mut out = Vec::new();
    for i in 0..count {
        let off = match u32_at(data, 44 + i * 20) {
            Some(o) => o as usize,
            None => break,
        };
        let group = groups.get(i).copied().unwrap_or("properties");
        let properties = read_section(data, off, group);
        out.push(PropertySet {
            group: group.to_string(),
            properties,
        });
    }
    Ok(out)
}

/// Which sections a stream name holds, or `None` when it is not a property set.
pub fn groups_for(stream_name: &str) -> Option<&'static [&'static str]> {
    let bare = stream_name.trim_start_matches('\u{5}');
    match bare {
        "SummaryInformation" => Some(&["summary"]),
        "DocumentSummaryInformation" => Some(&["document", "custom"]),
        _ => None,
    }
}

/// A writer for the same format, so the reader can be exercised - and so the
/// studio's sample documents carry real parameters - without a SolidWorks
/// licence. Every value written is a string.
pub mod fixture {
    fn lpwstr(s: &str) -> Vec<u8> {
        let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        let mut v = 31u32.to_le_bytes().to_vec();
        v.extend_from_slice(&(units.len() as u32).to_le_bytes());
        for u in units {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v
    }

    /// One section: a table of (id, offset) pairs, then the values.
    fn section(props: &[(u32, &str)]) -> Vec<u8> {
        let table_len = 8 + props.len() * 8;
        let mut body: Vec<u8> = Vec::new();
        let mut table: Vec<u8> = Vec::new();
        for (id, text) in props {
            let at = table_len + body.len();
            table.extend_from_slice(&id.to_le_bytes());
            table.extend_from_slice(&(at as u32).to_le_bytes());
            body.extend_from_slice(&lpwstr(text));
        }
        let mut sec = Vec::new();
        sec.extend_from_slice(&((table_len + body.len()) as u32).to_le_bytes());
        sec.extend_from_slice(&(props.len() as u32).to_le_bytes());
        sec.extend_from_slice(&table);
        sec.extend_from_slice(&body);
        sec
    }

    /// A whole stream holding one section per entry, in order.
    pub fn build(sections: &[&[(u32, &str)]]) -> Vec<u8> {
        let n = sections.len();
        let mut data = vec![0u8; 28 + n * 20];
        data[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
        data[24..28].copy_from_slice(&(n as u32).to_le_bytes());
        for (i, props) in sections.iter().enumerate() {
            let off = data.len();
            data[44 + i * 20..48 + i * 20].copy_from_slice(&(off as u32).to_le_bytes());
            data.extend_from_slice(&section(props));
        }
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a one-section property-set stream from (id, value-bytes) pairs.
    /// The value bytes must already be a full typed value.
    fn stream(section: &[(u32, Vec<u8>)]) -> Vec<u8> {
        // Header
        let mut data = vec![0u8; 48];
        data[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
        data[24..28].copy_from_slice(&1u32.to_le_bytes()); // one section
        data[44..48].copy_from_slice(&48u32.to_le_bytes()); // section offset

        // Section header
        let mut body: Vec<u8> = Vec::new();
        let n = section.len() as u32;
        let table_len = 8 + section.len() * 8;
        let mut table: Vec<u8> = Vec::new();
        for (id, v) in section {
            let at = table_len + body.len();
            table.extend_from_slice(&id.to_le_bytes());
            table.extend_from_slice(&(at as u32).to_le_bytes());
            body.extend_from_slice(v);
        }
        let size = (table_len + body.len()) as u32;
        let mut sec: Vec<u8> = Vec::new();
        sec.extend_from_slice(&size.to_le_bytes());
        sec.extend_from_slice(&n.to_le_bytes());
        sec.extend_from_slice(&table);
        sec.extend_from_slice(&body);
        data.extend_from_slice(&sec);
        data
    }

    fn lpwstr(s: &str) -> Vec<u8> {
        let mut v = 31u32.to_le_bytes().to_vec();
        let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        v.extend_from_slice(&(units.len() as u32).to_le_bytes());
        for u in units {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v
    }

    fn i4(n: i32) -> Vec<u8> {
        let mut v = 3u32.to_le_bytes().to_vec();
        v.extend_from_slice(&n.to_le_bytes());
        v
    }

    #[test]
    fn reads_summary_properties_by_name() {
        let s = stream(&[(2, lpwstr("bracket")), (4, lpwstr("A. Machinist"))]);
        let sets = parse(&s, &["summary"]).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].group, "summary");
        let get = |n: &str| sets[0].properties.iter().find(|p| p.name == n).map(|p| p.value.clone());
        assert_eq!(get("Title").as_deref(), Some("bracket"));
        assert_eq!(get("Author").as_deref(), Some("A. Machinist"));
    }

    #[test]
    fn renders_numbers_and_dates() {
        let s = stream(&[(10, i4(3600))]);
        let sets = parse(&s, &["summary"]).unwrap();
        assert_eq!(sets[0].properties[0].name, "Total edit time");
        assert_eq!(sets[0].properties[0].value, "3600");

        // 1601-01-01 + 1 day, in 100ns ticks.
        let mut ft = 64u32.to_le_bytes().to_vec();
        ft.extend_from_slice(&(86_400u64 * 10_000_000).to_le_bytes());
        let s = stream(&[(12, ft)]);
        let sets = parse(&s, &["summary"]).unwrap();
        assert!(sets[0].properties[0].value.starts_with("1601-01-02"), "{}", sets[0].properties[0].value);
    }

    #[test]
    fn reports_a_stream_that_is_not_a_property_set() {
        assert!(parse(b"nonsense bytes here", &["summary"]).is_err());
        assert!(parse(&[0u8; 4], &["summary"]).is_err());
    }

    #[test]
    fn the_writer_and_the_reader_agree() {
        let s = fixture::build(&[&[(2, "bracket"), (4, "A. Machinist")]]);
        let sets = parse(&s, &["summary"]).unwrap();
        assert_eq!(sets[0].properties.len(), 2);
        assert_eq!(sets[0].properties[0].name, "Title");
        assert_eq!(sets[0].properties[0].value, "bracket");
        assert_eq!(sets[0].properties[1].name, "Author");
    }

    #[test]
    fn names_the_sections_each_stream_holds() {
        assert_eq!(groups_for("\u{5}SummaryInformation"), Some(&["summary"][..]));
        assert_eq!(
            groups_for("DocumentSummaryInformation"),
            Some(&["document", "custom"][..])
        );
        assert_eq!(groups_for("Contents"), None);
    }
}
