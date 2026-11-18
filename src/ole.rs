//! Compound File Binary Format ("OLE2 structured storage") reader.
//!
//! A SolidWorks document - part, assembly or drawing - is one of these: a small
//! filesystem inside a single file, with a directory of named streams. The
//! format is Microsoft's, stable and documented, so it is read directly here
//! instead of through a dependency. Nothing has to be installed to open a
//! customer's part file, which matters for a tool that runs on the user's own
//! machine.
//!
//! This module understands the *container* only. What is inside a SolidWorks
//! stream is `src/sldworks.rs`.
//!
//! Every read is bounds-checked: these are untrusted files, and a truncated or
//! hostile one must produce an error rather than a panic.

use serde::Serialize;
use std::collections::HashSet;

const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Sector markers, as they appear in a FAT.
const FREESECT: u32 = 0xFFFF_FFFF;
const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
/// Only written, never needed while reading: the reader does not care *why* a
/// sector is not part of a chain.
const FATSECT: u32 = 0xFFFF_FFFD;
const NOSTREAM: u32 = 0xFFFF_FFFF;

pub fn looks_like_ole(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && bytes[..8] == SIGNATURE
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Root,
    Storage,
    Stream,
    Free,
}

/// One item in the container's directory.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: usize,
    pub name: String,
    /// Slash-separated, from the root: `/Preview`, `/Contents`.
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
}

#[derive(Debug, Clone)]
struct RawEntry {
    name: String,
    kind: EntryKind,
    start: u32,
    size: u64,
    left: u32,
    right: u32,
    child: u32,
}

pub struct OleFile {
    bytes: Vec<u8>,
    sector_size: usize,
    mini_sector_size: usize,
    cutoff: u32,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    raw: Vec<RawEntry>,
    entries: Vec<Entry>,
    root: usize,
    mini_stream_start: u32,
    mini_stream_size: u64,
    pub major_version: u16,
}

/// Offset of a sector, checked against the real file length.
fn sector_offset(sector: u32, sector_size: usize, len: usize) -> Result<usize, String> {
    if sector >= 0xFFFF_FFF0 {
        return Err(format!("sector {sector:#010x} is a marker, not a sector"));
    }
    let off = (sector as usize + 1)
        .checked_mul(sector_size)
        .ok_or_else(|| "sector index overflows".to_string())?;
    if off + sector_size > len {
        return Err(format!(
            "sector {sector} runs past the end of the file (need {sector_size} bytes at {off}, file is {len})"
        ));
    }
    Ok(off)
}

/// Follow a FAT chain, concatenating whole sectors.
fn read_chain(
    bytes: &[u8],
    sector_size: usize,
    fat: &[u32],
    start: u32,
    max_len: Option<usize>,
) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    let mut sector = start;
    let mut steps = 0usize;
    // A malformed chain can loop; the FAT is finite, so cap the walk.
    let limit = fat.len() + 16;
    while sector != ENDOFCHAIN && sector != FREESECT {
        if steps > limit {
            return Err("FAT chain does not terminate".into());
        }
        steps += 1;
        let off = sector_offset(sector, sector_size, bytes.len())?;
        out.extend_from_slice(&bytes[off..off + sector_size]);
        if let Some(m) = max_len {
            if out.len() >= m {
                break;
            }
        }
        let next = fat.get(sector as usize).copied().unwrap_or(ENDOFCHAIN);
        if next == sector {
            break;
        }
        sector = next;
    }
    Ok(out)
}

fn decode_name(field: &[u8], name_len: usize) -> String {
    let take = name_len.saturating_sub(2).min(field.len());
    let mut units = Vec::with_capacity(take / 2);
    for c in field[..take].chunks_exact(2) {
        units.push(u16::from_le_bytes([c[0], c[1]]));
    }
    String::from_utf16_lossy(&units)
}

impl OleFile {
    pub fn parse(bytes: Vec<u8>) -> Result<OleFile, String> {
        if !looks_like_ole(&bytes) {
            return Err("not an OLE2 compound file (signature mismatch)".into());
        }
        let sector_shift = u16_at(&bytes, 0x1E).ok_or("header is truncated")?;
        let mini_shift = u16_at(&bytes, 0x20).ok_or("header is truncated")?;
        if !(7..=20).contains(&sector_shift) {
            return Err(format!("implausible sector size shift {sector_shift}"));
        }
        if !(2..=12).contains(&mini_shift) {
            return Err(format!("implausible mini sector shift {mini_shift}"));
        }
        let sector_size = 1usize << sector_shift;
        let mini_sector_size = 1usize << mini_shift;
        let minor_version = u16_at(&bytes, 0x18).unwrap_or(0);
        let major_version = u16_at(&bytes, 0x1A).unwrap_or(3);
        tracing::trace!("compound file version {major_version}.{minor_version}");
        let cutoff = u32_at(&bytes, 0x38).unwrap_or(4096);
        let first_dir = u32_at(&bytes, 0x30).ok_or("header is truncated")?;
        let first_mini_fat = u32_at(&bytes, 0x3C).unwrap_or(ENDOFCHAIN);
        let first_difat = u32_at(&bytes, 0x44).unwrap_or(ENDOFCHAIN);

        // The DIFAT says which sectors hold the FAT. 109 entries sit in the
        // header; any more are chained through dedicated DIFAT sectors.
        let mut fat_sectors: Vec<u32> = Vec::new();
        for i in 0..109 {
            match u32_at(&bytes, 0x4C + i * 4) {
                Some(s) if s != FREESECT && s != ENDOFCHAIN => fat_sectors.push(s),
                _ => break,
            }
        }
        let mut next = first_difat;
        let mut guard = 0;
        while next != ENDOFCHAIN && next != FREESECT && guard < 65_536 {
            let off = match sector_offset(next, sector_size, bytes.len()) {
                Ok(o) => o,
                Err(_) => break,
            };
            let per = sector_size / 4 - 1;
            for i in 0..per {
                if let Some(s) = u32_at(&bytes, off + i * 4) {
                    if s != FREESECT {
                        fat_sectors.push(s);
                    }
                }
            }
            next = u32_at(&bytes, off + per * 4).unwrap_or(ENDOFCHAIN);
            guard += 1;
        }

        let mut fat: Vec<u32> = Vec::new();
        for s in &fat_sectors {
            if let Ok(off) = sector_offset(*s, sector_size, bytes.len()) {
                for i in 0..sector_size / 4 {
                    fat.push(u32_at(&bytes, off + i * 4).unwrap_or(FREESECT));
                }
            }
        }
        if fat.is_empty() {
            return Err("the file has no readable FAT".into());
        }

        let dir = read_chain(&bytes, sector_size, &fat, first_dir, None)?;
        let mut raw: Vec<RawEntry> = Vec::new();
        for chunk in dir.chunks_exact(128) {
            let name_len = u16_at(chunk, 0x40).unwrap_or(0) as usize;
            let kind = match chunk[0x42] {
                1 => EntryKind::Storage,
                2 => EntryKind::Stream,
                5 => EntryKind::Root,
                _ => EntryKind::Free,
            };
            // In version 3 the size field is 32 bits; the rest is padding.
            let size = if major_version <= 3 {
                u32_at(chunk, 0x78).unwrap_or(0) as u64
            } else {
                u64_at(chunk, 0x78).unwrap_or(0)
            };
            raw.push(RawEntry {
                name: decode_name(&chunk[0..64], name_len),
                kind,
                start: u32_at(chunk, 0x74).unwrap_or(ENDOFCHAIN),
                size,
                left: u32_at(chunk, 0x44).unwrap_or(NOSTREAM),
                right: u32_at(chunk, 0x48).unwrap_or(NOSTREAM),
                child: u32_at(chunk, 0x4C).unwrap_or(NOSTREAM),
            });
        }
        if raw.is_empty() {
            return Err("the directory is empty".into());
        }

        let root = raw
            .iter()
            .position(|e| e.kind == EntryKind::Root)
            .ok_or("the container has no root entry")?;
        let mini_stream_start = raw[root].start;
        let mini_stream_size = raw[root].size;

        let mut mini_fat: Vec<u32> = Vec::new();
        if first_mini_fat != ENDOFCHAIN && first_mini_fat != FREESECT {
            let bytes_of_mini_fat =
                read_chain(&bytes, sector_size, &fat, first_mini_fat, None).unwrap_or_default();
            for c in bytes_of_mini_fat.chunks_exact(4) {
                mini_fat.push(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
            }
        }

        let mut file = OleFile {
            bytes,
            sector_size,
            mini_sector_size,
            cutoff,
            fat,
            mini_fat,
            raw,
            entries: Vec::new(),
            root,
            mini_stream_start,
            mini_stream_size,
            major_version,
        };
        file.entries = file.build_entries();
        Ok(file)
    }

    /// Walk the directory red-black tree to give every item a path. Anything the
    /// tree fails to reach is still listed, so a corrupt file cannot hide a
    /// stream from the catalogue.
    fn build_entries(&self) -> Vec<Entry> {
        let mut paths: Vec<Option<String>> = vec![None; self.raw.len()];
        let mut seen: HashSet<u32> = HashSet::new();
        let child = self.raw[self.root].child;
        self.walk(child, "", &mut paths, &mut seen);
        paths[self.root] = Some("/".to_string());

        let mut out = Vec::new();
        for (id, e) in self.raw.iter().enumerate() {
            if e.kind == EntryKind::Free {
                continue;
            }
            let path = paths[id]
                .clone()
                .unwrap_or_else(|| format!("/{}", e.name));
            out.push(Entry {
                id,
                name: e.name.clone(),
                path,
                kind: e.kind,
                size: e.size,
            });
        }
        out
    }

    fn walk(&self, id: u32, prefix: &str, paths: &mut [Option<String>], seen: &mut HashSet<u32>) {
        if id == NOSTREAM || !seen.insert(id) {
            return;
        }
        let Some(e) = self.raw.get(id as usize) else {
            return;
        };
        if e.kind == EntryKind::Free {
            return;
        }
        let path = format!("{prefix}/{}", e.name);
        paths[id as usize] = Some(path.clone());
        // Left and right are siblings at this level; the child is one level down.
        self.walk(e.left, prefix, paths, seen);
        self.walk(e.right, prefix, paths, seen);
        let below = if e.kind == EntryKind::Storage {
            path
        } else {
            prefix.to_string()
        };
        self.walk(e.child, &below, paths, seen);
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Bytes per sector, for anything that wants to describe the container.
    pub fn sector_size(&self) -> usize {
        self.sector_size
    }

    /// Read one stream in full. Streams below the cutoff live in the mini
    /// stream, which is itself a stream - followed through the mini FAT.
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>, String> {
        let raw = self
            .raw
            .get(entry.id)
            .ok_or_else(|| "no such stream".to_string())?;
        match raw.kind {
            EntryKind::Free => Ok(Vec::new()),
            EntryKind::Root | EntryKind::Storage => {
                let mut out = read_chain(
                    &self.bytes,
                    self.sector_size,
                    &self.fat,
                    raw.start,
                    Some(raw.size as usize),
                )?;
                out.truncate(raw.size as usize);
                Ok(out)
            }
            EntryKind::Stream => {
                if raw.size == 0 {
                    return Ok(Vec::new());
                }
                if raw.size < self.cutoff as u64 {
                    let mini = read_chain(
                        &self.bytes,
                        self.sector_size,
                        &self.fat,
                        self.mini_stream_start,
                        Some(self.mini_stream_size as usize),
                    )?;
                    let mut out: Vec<u8> = Vec::with_capacity(raw.size as usize);
                    let mut sector = raw.start;
                    let mut steps = 0usize;
                    while sector != ENDOFCHAIN
                        && sector != FREESECT
                        && out.len() < raw.size as usize
                    {
                        if steps > self.mini_fat.len() + 16 {
                            return Err("mini FAT chain does not terminate".into());
                        }
                        steps += 1;
                        let off = sector as usize * self.mini_sector_size;
                        if off + self.mini_sector_size > mini.len() {
                            break;
                        }
                        out.extend_from_slice(&mini[off..off + self.mini_sector_size]);
                        let next = self
                            .mini_fat
                            .get(sector as usize)
                            .copied()
                            .unwrap_or(ENDOFCHAIN);
                        if next == sector {
                            break;
                        }
                        sector = next;
                    }
                    out.truncate(raw.size as usize);
                    Ok(out)
                } else {
                    let mut out = read_chain(
                        &self.bytes,
                        self.sector_size,
                        &self.fat,
                        raw.start,
                        Some(raw.size as usize),
                    )?;
                    out.truncate(raw.size as usize);
                    Ok(out)
                }
            }
        }
    }
}

/// A writer for the same format, so the reader can be tested without a
/// SolidWorks licence: it lays out a real compound file with the given streams.
/// It is also what the studio's sample documents are built from.
pub mod fixture {
    use super::*;

    const SECTOR: usize = 512;
    const MINI: usize = 64;
    const CUTOFF: usize = 4096;

    fn set_name(e: &mut [u8], name: &str) {
        let mut bytes: Vec<u8> = Vec::new();
        for u in name.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        let n = bytes.len().min(64);
        e[..n].copy_from_slice(&bytes[..n]);
        e[0x40..0x42].copy_from_slice(&(n as u16).to_le_bytes());
    }

    fn put_u32(buf: &mut [u8], at: usize, v: u32) {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// Mark `count` sectors starting at `start` as one chain.
    fn link(fat: &mut [u32], start: usize, count: usize) {
        for i in 0..count {
            fat[start + i] = if i + 1 == count {
                ENDOFCHAIN
            } else {
                (start + i + 1) as u32
            };
        }
    }

    pub fn build(streams: &[(&str, &[u8])]) -> Vec<u8> {
        let n = streams.len();
        let n_entries = n + 1;
        let n_dir = (n_entries * 128).div_ceil(SECTOR);

        // Split the streams: small ones go in the mini stream, large ones get
        // their own sectors.
        let mut mini_blob: Vec<u8> = Vec::new();
        let mut mini_fat: Vec<u32> = Vec::new();
        let mut is_mini: Vec<bool> = Vec::new();
        let mut mini_starts: Vec<u32> = Vec::new();
        let mut regular_sectors: Vec<usize> = Vec::new();

        for (_, data) in streams {
            let small = data.len() < CUTOFF;
            is_mini.push(small);
            if small {
                let start = (mini_blob.len() / MINI) as u32;
                mini_starts.push(start);
                let mut chain: Vec<u32> = Vec::new();
                for chunk in data.chunks(MINI) {
                    let s = (mini_blob.len() / MINI) as u32;
                    chain.push(s);
                    let mut c = chunk.to_vec();
                    c.resize(MINI, 0);
                    mini_blob.extend_from_slice(&c);
                }
                while mini_fat.len() < mini_blob.len() / MINI {
                    mini_fat.push(FREESECT);
                }
                for w in chain.windows(2) {
                    mini_fat[w[0] as usize] = w[1];
                }
                if let Some(&last) = chain.last() {
                    mini_fat[last as usize] = ENDOFCHAIN;
                }
                regular_sectors.push(0);
            } else {
                mini_starts.push(ENDOFCHAIN);
                regular_sectors.push(data.len().div_ceil(SECTOR));
            }
        }

        let n_minifat = if mini_fat.is_empty() {
            0
        } else {
            mini_fat.len().div_ceil(SECTOR / 4)
        };
        let n_mini = mini_blob.len().div_ceil(SECTOR);
        let mut mini_padded = mini_blob.clone();
        mini_padded.resize(n_mini * SECTOR, 0);
        let n_regular: usize = regular_sectors.iter().sum();

        // How many sectors hold the FAT depends on how many sectors there are.
        let mut n_fat = 1usize;
        for _ in 0..32 {
            let total = n_fat + n_dir + n_minifat + n_mini + n_regular;
            let want = total.div_ceil(SECTOR / 4);
            if want == n_fat {
                break;
            }
            n_fat = want;
        }
        assert!(n_fat <= 109, "too many sectors for a header DIFAT");

        let dir_start = n_fat;
        let minifat_start = dir_start + n_dir;
        let mini_start = minifat_start + n_minifat;
        let regular_start = mini_start + n_mini;

        let mut fat: Vec<u32> = vec![FREESECT; n_fat * (SECTOR / 4)];
        for slot in fat.iter_mut().take(n_fat) {
            *slot = FATSECT;
        }
        link(&mut fat, dir_start, n_dir);
        if n_minifat > 0 {
            link(&mut fat, minifat_start, n_minifat);
        }
        if n_mini > 0 {
            link(&mut fat, mini_start, n_mini);
        }
        let mut regular_starts: Vec<u32> = Vec::new();
        let mut cursor = regular_start;
        for sectors in &regular_sectors {
            if *sectors == 0 {
                regular_starts.push(ENDOFCHAIN);
                continue;
            }
            regular_starts.push(cursor as u32);
            link(&mut fat, cursor, *sectors);
            cursor += sectors;
        }

        // Directory: root first, then one entry per stream, siblings chained
        // to the right. No red-black balancing, which readers do not need.
        let mut dir = vec![0u8; n_dir * SECTOR];
        {
            let e = &mut dir[0..128];
            set_name(e, "Root Entry");
            e[0x42] = 5;
            e[0x43] = 1;
            put_u32(e, 0x44, NOSTREAM);
            put_u32(e, 0x48, NOSTREAM);
            put_u32(e, 0x4C, if n > 0 { 1 } else { NOSTREAM });
            put_u32(e, 0x74, if n_mini > 0 { mini_start as u32 } else { ENDOFCHAIN });
            put_u32(e, 0x78, mini_padded.len() as u32);
        }
        for (i, (name, data)) in streams.iter().enumerate() {
            let id = i + 1;
            let e = &mut dir[id * 128..(id + 1) * 128];
            set_name(e, name);
            e[0x42] = 2;
            e[0x43] = 1;
            put_u32(e, 0x44, NOSTREAM);
            let right = if id + 1 < n_entries {
                (id + 1) as u32
            } else {
                NOSTREAM
            };
            put_u32(e, 0x48, right);
            put_u32(e, 0x4C, NOSTREAM);
            let start = if is_mini[i] { mini_starts[i] } else { regular_starts[i] };
            put_u32(e, 0x74, start);
            put_u32(e, 0x78, data.len() as u32);
        }

        let mut minifat_bytes = vec![0u8; n_minifat * SECTOR];
        for i in 0..n_minifat * (SECTOR / 4) {
            let v = mini_fat.get(i).copied().unwrap_or(FREESECT);
            put_u32(&mut minifat_bytes, i * 4, v);
        }

        let mut fat_bytes = vec![0u8; n_fat * SECTOR];
        for (i, v) in fat.iter().enumerate() {
            put_u32(&mut fat_bytes, i * 4, *v);
        }

        let mut out = vec![0u8; SECTOR];
        out[0..8].copy_from_slice(&SIGNATURE);
        out[0x18..0x1A].copy_from_slice(&0x003Eu16.to_le_bytes());
        out[0x1A..0x1C].copy_from_slice(&3u16.to_le_bytes());
        out[0x1C..0x1E].copy_from_slice(&0xFFFEu16.to_le_bytes());
        out[0x1E..0x20].copy_from_slice(&9u16.to_le_bytes());
        out[0x20..0x22].copy_from_slice(&6u16.to_le_bytes());
        put_u32(&mut out, 0x28, 0);
        put_u32(&mut out, 0x2C, n_fat as u32);
        put_u32(&mut out, 0x30, dir_start as u32);
        put_u32(&mut out, 0x38, CUTOFF as u32);
        put_u32(
            &mut out,
            0x3C,
            if n_minifat > 0 { minifat_start as u32 } else { ENDOFCHAIN },
        );
        put_u32(&mut out, 0x40, n_minifat as u32);
        put_u32(&mut out, 0x44, ENDOFCHAIN);
        put_u32(&mut out, 0x48, 0);
        for i in 0..109 {
            let v = if i < n_fat { i as u32 } else { FREESECT };
            put_u32(&mut out, 0x4C + i * 4, v);
        }

        out.extend_from_slice(&fat_bytes);
        out.extend_from_slice(&dir);
        out.extend_from_slice(&minifat_bytes);
        out.extend_from_slice(&mini_padded);
        for (i, (_, data)) in streams.iter().enumerate() {
            if !is_mini[i] {
                let mut d = data.to_vec();
                d.resize(regular_sectors[i] * SECTOR, 0);
                out.extend_from_slice(&d);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::build;
    use super::*;

    /// Look a stream up by name and read it, the way a caller would.
    fn read_named(ole: &OleFile, name: &str) -> Option<Vec<u8>> {
        let e = ole.entries().iter().find(|e| e.name == name)?;
        ole.read(e).ok()
    }

    fn png() -> Vec<u8> {
        let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend(std::iter::repeat_n(0x42u8, 120));
        v
    }

    fn big() -> Vec<u8> {
        (0..5000u32).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn reads_streams_both_side_of_the_cutoff() {
        let text = b"feature tree payload, for testing the container".to_vec();
        let file = build(&[
            ("Preview", &png()),
            ("ThirdPv", &text),
            ("Contents", &big()),
        ]);
        assert!(looks_like_ole(&file));
        let ole = OleFile::parse(file).unwrap();

        let third = ole.entries().iter().find(|e| e.name == "ThirdPv").unwrap();
        assert_eq!(third.size, text.len() as u64);
        assert_eq!(read_named(&ole, "ThirdPv").unwrap(), text);
        assert_eq!(read_named(&ole, "Preview").unwrap(), png());
        assert_eq!(read_named(&ole, "Contents").unwrap(), big());
    }

    #[test]
    fn reports_paths_from_the_directory_tree() {
        let file = build(&[("Preview", &png()), ("Contents", &big())]);
        let ole = OleFile::parse(file).unwrap();
        let paths: Vec<&str> = ole.entries().iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"/Preview"), "{paths:?}");
        assert!(paths.contains(&"/Contents"), "{paths:?}");
        assert!(paths.contains(&"/"), "{paths:?}");
        assert_eq!(ole.major_version, 3);
    }

    #[test]
    fn reads_an_empty_stream() {
        let file = build(&[("Empty", &[]), ("Contents", &big())]);
        let ole = OleFile::parse(file).unwrap();
        assert!(read_named(&ole, "Empty").unwrap().is_empty());
    }

    #[test]
    fn rejects_files_that_are_not_compound_documents() {
        assert!(OleFile::parse(b"this is an STL, not a document".to_vec()).is_err());
        assert!(OleFile::parse(vec![0u8; 600]).is_err());
    }

    #[test]
    fn truncated_files_error_rather_than_panic() {
        let full = build(&[("Contents", &vec![7u8; 6000]), ("Preview", &png())]);
        for cut in [1usize, 100, 511, 520, 1024, 2048, 4000] {
            let part = full[..cut.min(full.len())].to_vec();
            if let Ok(ole) = OleFile::parse(part) {
                let _ = read_named(&ole, "Contents");
                let _ = read_named(&ole, "Preview");
            }
        }
    }
}
