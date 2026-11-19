//! How close is a rebuild to the target?
//!
//! Both sides are triangle soups, so the comparison is geometric and does not
//! care how the geometry was produced: sample points across the surfaces, then
//! take the mean distance to the other surface in both directions (a Chamfer
//! distance). Symmetric on purpose - a rebuild that leaves a hole out is
//! penalised, and so is one that invents material the target does not have.
//!
//! The distance from a sample to the other shape is measured to the *surface*,
//! not to the nearest sample point. That distinction is the whole point of this
//! module: two triangles that tile the same quad with a different diagonal have
//! identical surfaces but different corners and centres, so a point-to-sample
//! metric punishes a different triangulation of the same shape. Measuring to the
//! triangle makes an equivalent mesh score exactly zero, which is what makes
//! `max_gap` a real deviation rather than an artefact of how the file was cut
//! into triangles.

use crate::vec3::{add, cross, dot, len, scale, sub, V3};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Score {
    pub chamfer: f64,
    pub forward: f64,
    pub backward: f64,
    pub max_gap: f64,
    pub samples: usize,
    pub size_delta: V3,
}

#[derive(Debug, Clone, Copy)]
struct Tri {
    a: V3,
    b: V3,
    c: V3,
}

/// Parse a triangle soup into triangles, ignoring any trailing partial triangle.
fn triangles(tris: &[f32]) -> Vec<Tri> {
    tris.chunks_exact(9)
        .map(|t| Tri {
            a: [t[0] as f64, t[1] as f64, t[2] as f64],
            b: [t[3] as f64, t[4] as f64, t[5] as f64],
            c: [t[6] as f64, t[7] as f64, t[8] as f64],
        })
        .collect()
}

fn centroid(t: &Tri) -> V3 {
    [
        (t.a[0] + t.b[0] + t.c[0]) / 3.0,
        (t.a[1] + t.b[1] + t.c[1]) / 3.0,
        (t.a[2] + t.b[2] + t.c[2]) / 3.0,
    ]
}

fn area(t: &Tri) -> f64 {
    0.5 * len(cross(sub(t.b, t.a), sub(t.c, t.a)))
}

/// Sample points across a soup, one weight each: four points per triangle, and
/// the weight carries the triangle's area so a surface made of a few big
/// triangles is not outvoted by one made of many small ones.
fn samples(tris: &[f32]) -> Vec<(V3, f64)> {
    let mut out = Vec::with_capacity(tris.len() / 9 * 4);
    for t in triangles(tris) {
        let w = area(&t) / 4.0;
        for p in [t.a, t.b, t.c, centroid(&t)] {
            out.push((p, w));
        }
    }
    out
}

/// The point on a triangle closest to `q`. Standard barycentric solution - the
/// best point is either inside the face or on one of its three edges or vertices.
fn closest_on_triangle(q: V3, t: &Tri) -> V3 {
    let ab = sub(t.b, t.a);
    let ac = sub(t.c, t.a);
    let ap = sub(q, t.a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return t.a;
    }

    let bp = sub(q, t.b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return t.b;
    }

    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return add(t.a, scale(ab, v));
    }

    let cp = sub(q, t.c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return t.c;
    }

    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return add(t.a, scale(ac, w));
    }

    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return add(t.b, scale(sub(t.c, t.b), w));
    }

    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    add(t.a, add(scale(ab, v), scale(ac, w)))
}

fn bbox_of(t: &Tri) -> (V3, V3) {
    let mut lo = t.a;
    let mut hi = t.a;
    for p in [t.b, t.c] {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

/// Triangles in a uniform grid, so a query only examines nearby faces.
struct Surface {
    cell: f64,
    origin: V3,
    tris: Vec<Tri>,
    buckets: std::collections::HashMap<(i64, i64, i64), Vec<usize>>,
    /// How far the shell search is allowed to grow before falling back to an
    /// exhaustive scan, which is bounded and exact.
    span: i64,
}

impl Surface {
    fn new(tris: Vec<Tri>) -> Surface {
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        for t in &tris {
            let (blo, bhi) = bbox_of(t);
            for k in 0..3 {
                lo[k] = lo[k].min(blo[k]);
                hi[k] = hi[k].max(bhi[k]);
            }
        }
        let cell = (len(sub(hi, lo)) / 32.0).max(1e-6);
        let span = 64;

        let mut buckets: std::collections::HashMap<(i64, i64, i64), Vec<usize>> =
            std::collections::HashMap::new();
        let key = |p: V3| -> (i64, i64, i64) {
            (
                ((p[0] - lo[0]) / cell).floor() as i64,
                ((p[1] - lo[1]) / cell).floor() as i64,
                ((p[2] - lo[2]) / cell).floor() as i64,
            )
        };

        // A triangle goes in every cell its bounding box touches, so no query can
        // miss it. Big faces span many cells; that is the price of certainty.
        for (i, t) in tris.iter().enumerate() {
            let (blo, bhi) = bbox_of(t);
            let (klo, khi) = (key(blo), key(bhi));
            for x in klo.0..=khi.0 {
                for y in klo.1..=khi.1 {
                    for z in klo.2..=khi.2 {
                        buckets.entry((x, y, z)).or_default().push(i);
                    }
                }
            }
        }

        Surface {
            cell,
            origin: lo,
            tris,
            buckets,
            span,
        }
    }

    fn key(&self, p: V3) -> (i64, i64, i64) {
        (
            ((p[0] - self.origin[0]) / self.cell).floor() as i64,
            ((p[1] - self.origin[1]) / self.cell).floor() as i64,
            ((p[2] - self.origin[2]) / self.cell).floor() as i64,
        )
    }

    /// Distance from a point to the surface. Expands shells until no closer face
    /// can still be hiding outside the shell already searched, then falls back to
    /// an exhaustive scan rather than trusting an unbounded shell count.
    fn distance(&self, q: V3) -> Option<f64> {
        if self.tris.is_empty() {
            return None;
        }
        let base = self.key(q);
        let mut best = f64::MAX;
        let mut ring = 0i64;
        loop {
            // Anything outside the searched shells is at least this far away.
            let shell_min = (ring as f64 - 1.0).max(0.0) * self.cell;
            if best < f64::MAX && shell_min > best {
                return Some(best);
            }
            if ring > self.span {
                // Either the query is far outside the grid or the grid is sparse.
                // Scanning everything is bounded, and it is the true minimum.
                let mut exact = f64::MAX;
                for t in &self.tris {
                    let d = len(sub(closest_on_triangle(q, t), q));
                    if d < exact {
                        exact = d;
                    }
                }
                return Some(exact);
            }
            for dx in -ring..=ring {
                for dy in -ring..=ring {
                    for dz in -ring..=ring {
                        if ring > 0 && dx.abs() != ring && dy.abs() != ring && dz.abs() != ring {
                            continue;
                        }
                        if let Some(b) = self.buckets.get(&(base.0 + dx, base.1 + dy, base.2 + dz))
                        {
                            for &i in b {
                                let d = len(sub(closest_on_triangle(q, &self.tris[i]), q));
                                if d < best {
                                    best = d;
                                }
                            }
                        }
                    }
                }
            }
            ring += 1;
        }
    }
}

pub fn compare(rebuilt: &[f32], target: &[f32]) -> Option<Score> {
    let a = samples(rebuilt);
    let b = samples(target);
    if a.is_empty() || b.is_empty() {
        return None;
    }

    let surface_b = Surface::new(triangles(target));
    let surface_a = Surface::new(triangles(rebuilt));

    // Area-weighted means, so the estimate is of the surface, not of the
    // triangles someone happened to cut it into.
    let mut forward = 0.0;
    let mut forward_weight = 0.0;
    let mut max_gap = 0.0f64;
    for (p, w) in &a {
        let d = surface_b.distance(*p)?;
        forward += d * w;
        forward_weight += w;
        max_gap = max_gap.max(d);
    }
    forward /= forward_weight;

    let mut backward = 0.0;
    let mut backward_weight = 0.0;
    for (p, w) in &b {
        let d = surface_a.distance(*p)?;
        backward += d * w;
        backward_weight += w;
    }
    backward /= backward_weight;

    let pts_a: Vec<V3> = a.iter().map(|(p, _)| *p).collect();
    let pts_b: Vec<V3> = b.iter().map(|(p, _)| *p).collect();
    let sa = extents(&pts_a);
    let sb = extents(&pts_b);

    Some(Score {
        chamfer: 0.5 * (forward + backward),
        forward,
        backward,
        max_gap,
        samples: a.len() + b.len(),
        size_delta: [sa[0] - sb[0], sa[1] - sb[1], sa[2] - sb[2]],
    })
}

fn extents(pts: &[V3]) -> V3 {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in pts {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri(a: V3, b: V3, c: V3) -> Vec<f32> {
        let mut v = Vec::new();
        crate::vec3::push_tri(&mut v, a, b, c);
        v
    }

    /// One 10x10 square, cut along the diagonal that runs the other way.
    fn square(cut_downwards: bool) -> Vec<f32> {
        let p = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [0.0, 10.0, 0.0],
        ];
        let mut v = Vec::new();
        if cut_downwards {
            crate::vec3::push_tri(&mut v, p[0], p[1], p[2]);
            crate::vec3::push_tri(&mut v, p[0], p[2], p[3]);
        } else {
            crate::vec3::push_tri(&mut v, p[0], p[1], p[3]);
            crate::vec3::push_tri(&mut v, p[1], p[2], p[3]);
        }
        v
    }

    #[test]
    fn identical_shapes_score_zero() {
        let soup = tri([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let s = compare(&soup, &soup).unwrap();
        // A point that lands inside a face is located by the barycentric solution,
        // so this is zero to within rounding rather than bit-exactly zero.
        assert!(s.chamfer < 1e-12, "{s:?}");
        assert!(s.max_gap < 1e-12, "{s:?}");
    }

    /// The bug this module was fixed for. Both meshes are the same square; only
    /// the diagonal differs, so every corner and centre moves while the surface
    /// does not. A point-to-sample metric reported a millimetre-scale `max_gap`
    /// here - a phantom outlier in what is used as the training signal.
    #[test]
    fn an_equivalent_triangulation_of_the_same_face_scores_zero() {
        let s = compare(&square(true), &square(false)).unwrap();
        assert!(s.chamfer < 1e-12, "{s:?}");
        assert!(s.max_gap < 1e-12, "{s:?}");
        assert_eq!(s.size_delta, [0.0, 0.0, 0.0], "{s:?}");
    }

    /// A metric that cannot see a uniform offset is useless, so check it measures
    /// one exactly - through `max_gap` too, which must now be the real deviation.
    #[test]
    fn a_uniform_offset_is_measured_exactly() {
        let lift = |z: f64| {
            let mut v = Vec::new();
            crate::vec3::push_tri(&mut v, [0.0, 0.0, z], [10.0, 0.0, z], [0.0, 10.0, z]);
            v
        };
        let s = compare(&lift(0.0), &lift(1.0)).unwrap();
        assert!((s.chamfer - 1.0).abs() < 1e-9, "{s:?}");
        assert!((s.max_gap - 1.0).abs() < 1e-9, "{s:?}");
    }

    #[test]
    fn displaced_shapes_score_roughly_the_offset() {
        let a = tri([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let b = tri([2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 1.0, 0.0]);
        let s = compare(&a, &b).unwrap();
        assert!(s.chamfer > 1.0 && s.chamfer < 3.0, "chamfer {}", s.chamfer);
    }

    /// A point far outside the mesh must still get a real distance rather than
    /// running out of shells.
    #[test]
    fn a_distant_point_gets_a_real_distance() {
        let a = tri([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let far = tri([200.0, 0.0, 0.0], [201.0, 0.0, 0.0], [200.0, 1.0, 0.0]);
        let s = compare(&a, &far).unwrap();
        assert!(s.chamfer > 100.0, "chamfer {}", s.chamfer);
        assert!(
            s.max_gap >= s.chamfer,
            "max_gap {} chamfer {}",
            s.max_gap,
            s.chamfer
        );
    }

    #[test]
    fn empty_input_has_no_score() {
        assert!(compare(&[], &[1.0, 2.0, 3.0]).is_none());
    }
}
