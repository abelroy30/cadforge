//! Minimal STL reader: enough to show a real part in the viewport.
//!
//! Handles both ASCII and binary STL. A sibling module will later parse STEP
//! feature history; this one only needs geometry.

pub struct Mesh {
    /// Flat triangle soup: three vertices per triangle, xyz each.
    pub positions: Vec<f32>,
    pub triangles: usize,
    pub bbox: [f32; 6],
}

/// Upper bound on triangles we hand to the browser in one response.
const MAX_TRIANGLES: usize = 250_000;

pub fn parse(bytes: &[u8]) -> Result<Mesh, String> {
    let verts = if looks_binary(bytes) {
        parse_binary(bytes).ok_or_else(|| "binary STL header did not match its length".to_string())?
    } else {
        parse_ascii(bytes).ok_or_else(|| "no triangles found in ASCII STL".to_string())?
    };
    if verts.is_empty() {
        return Err("STL contained no triangles".into());
    }
    Ok(finish(verts))
}

fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.len() < 84 {
        return false;
    }
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    bytes.len() == 84 + count * 50
}

fn parse_binary(bytes: &[u8]) -> Option<Vec<f32>> {
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    let mut out = Vec::with_capacity(count.min(MAX_TRIANGLES) * 9);
    for i in 0..count {
        if out.len() / 9 >= MAX_TRIANGLES {
            break;
        }
        let base = 84 + i * 50 + 12; // skip the facet normal
        let mut v = [0f32; 9];
        for k in 0..9 {
            let o = base + k * 4;
            v[k] = f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        }
        out.extend_from_slice(&v);
    }
    Some(out)
}

fn parse_ascii(bytes: &[u8]) -> Option<Vec<f32>> {
    let text = String::from_utf8_lossy(bytes);
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("vertex") {
            if out.len() / 9 >= MAX_TRIANGLES {
                break;
            }
            let mut nums = rest.split_whitespace().filter_map(|t| t.parse::<f32>().ok());
            if let (Some(x), Some(y), Some(z)) = (nums.next(), nums.next(), nums.next()) {
                out.extend_from_slice(&[x, y, z]);
            }
        }
    }
    // Drop a trailing partial triangle so the buffer stays a multiple of 9.
    out.truncate(out.len() / 9 * 9);
    Some(out)
}

fn finish(positions: Vec<f32>) -> Mesh {
    let mut bbox = [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN];
    for p in positions.chunks_exact(3) {
        for axis in 0..3 {
            bbox[axis] = bbox[axis].min(p[axis]);
            bbox[axis + 3] = bbox[axis + 3].max(p[axis]);
        }
    }
    Mesh {
        triangles: positions.len() / 9,
        positions,
        bbox,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ascii_triangle() {
        let src = b"solid t\nfacet normal 0 0 1\n outer loop\n  vertex 0 0 0\n  vertex 1 0 0\n  vertex 0 1 0\n endloop\nendfacet\nendsolid t\n";
        let m = parse(src).unwrap();
        assert_eq!(m.triangles, 1);
        assert_eq!(m.positions.len(), 9);
        assert_eq!(m.bbox, [0.0, 0.0, 0.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn rejects_empty() {
        assert!(parse(b"solid empty\nendsolid empty\n").is_err());
    }
}
