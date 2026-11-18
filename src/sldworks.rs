//! SolidWorks documents: what they are, and what is inside them.
//!
//! A `.SLDPRT` (part), `.SLDASM` (assembly) or `.SLDDRW` (drawing) is an OLE2
//! compound file. Its contents are named streams, and one of them - the
//! FeatureManager data - is exactly what this project wants to learn from.
//!
//! What this module does, honestly:
//!
//! * reads the container and lists every stream it holds, with sizes;
//! * names the streams that are recognised, so a real file explains itself;
//! * extracts the embedded preview image, which is the part's picture;
//! * classifies the document as part, assembly or drawing.
//!
//! What it does **not** do is decode the feature tree. That encoding is
//! proprietary and undocumented, and guessing at it would produce confidently
//! wrong build histories - the one thing worse than none. The catalogue below
//! is how we find out exactly what a real file contains; the feature tree
//! parser is written against that evidence, not against a hunch.
//!
//! Classification is by extension, which is what SolidWorks itself enforces:
//! the application refuses to save a part under the assembly extension. When
//! the extension is missing or unknown this says so instead of guessing.

use serde::{Deserialize, Serialize};

use crate::ole::{EntryKind, OleFile};

/// Stream names that have been seen in SolidWorks documents, and what they are
/// believed to hold. Anything not listed is still reported, as `unrecognised`,
/// so nothing is hidden.
const RECOGNISED: &[(&str, &str)] = &[
    ("Preview", "embedded preview image"),
    ("PreviewPNG", "embedded preview image (PNG)"),
    ("ThirdPv", "feature and parameter tree data (proprietary encoding)"),
    ("Contents", "the document's geometry and topology payload"),
    ("SwDocContentMgr", "document content manager"),
    ("SummaryInformation", "OLE summary properties: title, author, dates"),
    (
        "DocumentSummaryInformation",
        "OLE document properties: company, part number",
    ),
    ("DisplayLists", "cached tessellation for display"),
];

/// Large streams are not worth reading to look for a thumbnail.
const MAX_SNIFF_BYTES: u64 = 8 * 1024 * 1024;

/// What kind of SolidWorks document a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    Part,
    Assembly,
    Drawing,
    Unknown,
}

impl DocKind {
    pub fn from_extension(ext: &str) -> DocKind {
        match ext.to_ascii_lowercase().as_str() {
            "sldprt" => DocKind::Part,
            "sldasm" => DocKind::Assembly,
            "slddrw" => DocKind::Drawing,
            _ => DocKind::Unknown,
        }
    }

    /// Does this extension belong to SolidWorks at all?
    pub fn is_solidworks(ext: &str) -> bool {
        !matches!(DocKind::from_extension(ext), DocKind::Unknown)
    }

    pub fn label(self) -> &'static str {
        match self {
            DocKind::Part => "SolidWorks part",
            DocKind::Assembly => "SolidWorks assembly",
            DocKind::Drawing => "SolidWorks drawing",
            DocKind::Unknown => "unknown document",
        }
    }

    /// The word a folder tree would use, for grouping.
    pub fn short(self) -> &'static str {
        match self {
            DocKind::Part => "part",
            DocKind::Assembly => "assembly",
            DocKind::Drawing => "drawing",
            DocKind::Unknown => "other",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamInfo {
    pub id: usize,
    pub name: String,
    pub path: String,
    pub size: u64,
    /// What the stream is believed to hold - `unrecognised` when we do not know.
    pub role: String,
    pub recognised: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewInfo {
    pub stream: String,
    pub content_type: String,
    pub bytes: u64,
}

/// Everything we can say about a document without decoding its internals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalogue {
    pub kind: DocKind,
    pub kind_label: String,
    /// e.g. `OLE2 compound file, version 3, 512-byte sectors`.
    pub container: String,
    pub major_version: u16,
    pub streams: Vec<StreamInfo>,
    pub stream_bytes: u64,
    pub preview: Option<PreviewInfo>,
    /// Streams whose names are not recognised on their own - the raw material
    /// for working out the feature tree format.
    pub unrecognised: Vec<String>,
    pub feature_tree_stream: Option<String>,
    pub notes: Vec<String>,
}

impl Catalogue {
    /// One line for the artifact list, so the studio reads at a glance.
    pub fn headline(&self) -> String {
        let mut s = format!("{} · {} stream(s)", self.kind_label, self.streams.len());
        if let Some(p) = &self.preview {
            s.push_str(&format!(" · preview {} KB", p.bytes / 1024));
        }
        if let Some(t) = &self.feature_tree_stream {
            s.push_str(&format!(" · feature tree in `{t}`"));
        }
        s
    }
}

/// The preview image itself, kept separately from the catalogue. Its format is
/// in `Catalogue::preview`, and is re-checked from the bytes when it is served.
#[derive(Debug, Clone)]
pub struct Preview {
    pub bytes: Vec<u8>,
}

pub struct Inspected {
    pub catalogue: Catalogue,
    pub preview: Option<Preview>,
}

/// Recognise an image by its leading bytes.
pub fn sniff_image(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    const SIGNS: &[(&[u8], &str, &str)] = &[
        (&[0x89, 0x50, 0x4E, 0x47], "png", "image/png"),
        (&[0xFF, 0xD8, 0xFF], "jpg", "image/jpeg"),
        (b"GIF8", "gif", "image/gif"),
        (b"BM", "bmp", "image/bmp"),
    ];
    for (magic, ext, ct) in SIGNS {
        if bytes.len() >= magic.len() && &bytes[..magic.len()] == *magic {
            return Some((ext, ct));
        }
    }
    None
}

/// Read a SolidWorks document: its catalogue, and its preview image if it has one.
///
/// `ext` is the file extension without the dot; it decides the document kind.
/// A file whose extension says SolidWorks but whose bytes are not a compound
/// file is an error, not a silent guess.
pub fn inspect(bytes: &[u8], ext: &str) -> Result<Inspected, String> {
    let kind = DocKind::from_extension(ext);
    let ole = OleFile::parse(bytes.to_vec())?;

    let mut streams: Vec<StreamInfo> = Vec::new();
    let mut unrecognised: Vec<String> = Vec::new();
    let mut feature_tree_stream: Option<String> = None;
    let mut stream_bytes: u64 = 0;
    let mut best_preview: Option<(String, String, Vec<u8>, bool)> = None;

    for entry in ole.entries() {
        if entry.kind != EntryKind::Stream {
            continue;
        }
        // OLE's own property streams start with 0x05; match on the readable name.
        let bare = entry.name.trim_start_matches('\u{5}').to_string();
        let role = RECOGNISED
            .iter()
            .find(|(n, _)| *n == bare)
            .map(|(_, r)| *r);
        if role.is_none() {
            unrecognised.push(entry.name.clone());
        }
        if bare == "ThirdPv" || bare.starts_with("ThirdPv") {
            feature_tree_stream = Some(entry.name.clone());
        }

        streams.push(StreamInfo {
            id: entry.id,
            name: entry.name.clone(),
            path: entry.path.clone(),
            size: entry.size,
            role: role.unwrap_or("unrecognised").to_string(),
            recognised: role.is_some(),
        });
        stream_bytes = stream_bytes.saturating_add(entry.size);

        // Look for a thumbnail, but never read a huge stream to do it.
        if entry.size == 0 || entry.size > MAX_SNIFF_BYTES {
            continue;
        }
        let Ok(data) = ole.read(entry) else { continue };
        if let Some((_e, content_type)) = sniff_image(&data) {
            let preferred = entry.name.to_ascii_lowercase().contains("preview");
            let better = match &best_preview {
                None => true,
                Some((_, _, prev, was_preferred)) => {
                    (preferred && !*was_preferred) || (preferred == *was_preferred && data.len() > prev.len())
                }
            };
            if better {
                best_preview = Some((
                    entry.name.clone(),
                    content_type.to_string(),
                    data,
                    preferred,
                ));
            }
        }
    }

    streams.sort_by(|a, b| b.size.cmp(&a.size));

    let preview_info = best_preview.as_ref().map(|(name, ct, data, _)| PreviewInfo {
        stream: name.clone(),
        content_type: ct.clone(),
        bytes: data.len() as u64,
    });

    let mut notes: Vec<String> = Vec::new();
    if feature_tree_stream.is_some() {
        notes.push(
            "the feature tree stream is listed but not decoded: its encoding is proprietary and decoding it needs a real sample to work against".into(),
        );
    } else {
        notes.push("no stream named ThirdPv was found; the feature tree may be absent or named differently in this version".into());
    }
    if preview_info.is_none() {
        notes.push("no embedded preview image found".into());
    }
    if kind == DocKind::Unknown {
        notes.push(
            "the extension is not one of .SLDPRT/.SLDASM/.SLDDRW, so the document kind is unknown".into(),
        );
    }

    let catalogue = Catalogue {
        kind,
        kind_label: kind.label().to_string(),
        container: format!(
            "OLE2 compound file, version {}, {}-byte sectors",
            ole.major_version,
            ole.sector_size()
        ),
        major_version: ole.major_version,
        streams,
        stream_bytes,
        preview: preview_info,
        unrecognised,
        feature_tree_stream,
        notes,
    };

    let preview = best_preview.map(|(_, _ct, bytes, _)| Preview { bytes });

    Ok(Inspected { catalogue, preview })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ole::fixture::build;

    /// A believable SolidWorks part: a small preview image, a feature-tree
    /// stream, a large geometry payload, and the two OLE property streams.
    fn sample_part() -> Vec<u8> {
        let png = {
            let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
            v.extend(std::iter::repeat_n(0x11u8, 900));
            v
        };
        let tree = b"\x02\x00\x01\x00Extrude1\x00Sketch1\x00Fillet3\x00".to_vec();
        let contents: Vec<u8> = (0..20_000u32).map(|i| (i % 97) as u8).collect();
        let summary = b"\xfe\xff\x00\x00title\x00bracket\x00".to_vec();
        build(&[
            ("Preview", &png),
            ("ThirdPv", &tree),
            ("Contents", &contents),
            ("\u{5}SummaryInformation", &summary),
            ("SwDocContentMgr", &b"x".repeat(40)),
        ])
    }

    #[test]
    fn classifies_by_extension() {
        assert_eq!(DocKind::from_extension("sldprt"), DocKind::Part);
        assert_eq!(DocKind::from_extension("SLDASM"), DocKind::Assembly);
        assert_eq!(DocKind::from_extension("SldDrw"), DocKind::Drawing);
        assert_eq!(DocKind::from_extension("stp"), DocKind::Unknown);
        assert!(DocKind::is_solidworks("sldprt"));
        assert!(!DocKind::is_solidworks("stl"));
        assert_eq!(DocKind::Part.short(), "part");
    }

    #[test]
    fn catalogues_a_part_and_finds_its_preview() {
        let r = inspect(&sample_part(), "sldprt").unwrap();
        let c = &r.catalogue;
        assert_eq!(c.kind, DocKind::Part);
        assert_eq!(c.kind_label, "SolidWorks part");
        assert_eq!(c.streams.len(), 5);
        assert_eq!(c.major_version, 3);

        // Every stream is reported, with the known ones named.
        let by_name = |n: &str| c.streams.iter().find(|s| s.name == n).unwrap();
        assert!(by_name("ThirdPv").recognised);
        assert!(by_name("ThirdPv").role.contains("feature"));
        assert_eq!(by_name("Contents").size, 20_000);
        assert!(!by_name("SwDocContentMgr").role.is_empty());

        // The preview is extracted, and it is the PNG we put in.
        let info = c.preview.as_ref().expect("preview");
        assert_eq!(info.content_type, "image/png");
        assert_eq!(info.stream, "Preview");
        let p = r.preview.unwrap();
        assert_eq!(p.bytes.len(), 908);
        assert_eq!(&p.bytes[..4], &[0x89, 0x50, 0x4E, 0x47]);

        // The feature-tree stream is located...
        assert_eq!(c.feature_tree_stream.as_deref(), Some("ThirdPv"));
        // ...and we say plainly that it is not decoded.
        assert!(c.notes.iter().any(|n| n.contains("not decoded")));
    }

    #[test]
    fn reports_unrecognised_streams_rather_than_hiding_them() {
        let file = build(&[("SomeNewStream", b"payload"), ("Contents", &vec![1u8; 5000])]);
        let c = inspect(&file, "sldprt").unwrap().catalogue;
        assert!(c.unrecognised.contains(&"SomeNewStream".to_string()));
        assert!(c.streams.iter().any(|s| s.name == "SomeNewStream" && !s.recognised));
    }

    #[test]
    fn a_document_without_a_preview_says_so() {
        let file = build(&[("Contents", &vec![3u8; 5000])]);
        let c = inspect(&file, "sldasm").unwrap().catalogue;
        assert_eq!(c.kind, DocKind::Assembly);
        assert!(c.preview.is_none());
        assert!(c.notes.iter().any(|n| n.contains("no embedded preview")));
        assert!(c.notes.iter().any(|n| n.contains("no stream named ThirdPv")));
    }

    #[test]
    fn a_solidworks_extension_on_a_non_compound_file_is_an_error() {
        assert!(inspect(b"not really a part", "sldprt").is_err());
    }

    #[test]
    fn headline_reads_at_a_glance() {
        let c = inspect(&sample_part(), "sldprt").unwrap().catalogue;
        let h = c.headline();
        assert!(h.contains("SolidWorks part"), "{h}");
        assert!(h.contains("5 stream"), "{h}");
        assert!(h.contains("preview"), "{h}");
    }
}
