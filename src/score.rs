//! How close is a rebuild to the target?
//!
//! Both sides are triangle soups, so the comparison is geometric and does not
//! care how the geometry was produced: sample points on the surfaces, then take
//! the mean nearest-neighbour distance in both directions (a Chamfer distance).
//! Symmetric on purpose - a rebuild that leaves a hole out is penalised, and so
//! is one that invents material the target does not have.

use crate::vec3::{len, sub, V3};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Score {
    pub chamfer: f64,
    pub forward: f64,
    pub backward: f64,
    pub max_gap: f64,
    pub samples: usize,
    pub size_delta: V3,
}

/// Four sample points per triangle: its three corners and its centroid.
fn samples(tris: &[f32]) -> Vec<V3> {
    let mut out = Vec::with_capacity(tris.len() / 9 * 4);
    for t in tris.chunks_exact(9) {
        let a = [t[0] as f64, t[1] as f64, t[2] as f64];
        let b = [t[3] as f64, t[4] as f64, t[5] as f64];
        let c = [t[6] as f64, t[7] as f64, t[8] as f64];
        let m = [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ];
        out.extend_from_slice(&[a, b, c, m]);
    }
    out
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

struct Grid {
    cell: f64,
    origin: V3,
    buckets: std::collections::HashMap<(i64, i64, i64), Vec<V3>>,
}

impl Grid {
    fn new(pts: &[V3]) -> Grid {
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        for p in pts {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let cell = (len(sub(hi, lo)) / 32.0).max(1e-6);
        let mut buckets: std::collections::HashMap<(i64, i64, i64), Vec<V3>> =
            std::collections::HashMap::new();
        for p in pts {
            let k = (
                ((p[0] - lo[0]) / cell).floor() as i64,
                ((p[1] - lo[1]) / cell).floor() as i64,
                ((p[2] - lo[2]) / cell).floor() as i64,
            );
            buckets.entry(k).or_default().push(*p);
        }
        Grid {
            cell,
            origin: lo,
            buckets,
        }
    }

    fn key(&self, p: V3) -> (i64, i64, i64) {
        (
            ((p[0] - self.origin[0]) / self.cell).floor() as i64,
            ((p[1] - self.origin[1]) / self.cell).floor() as i64,
            ((p[2] - self.origin[2]) / self.cell).floor() as i64,
        )
    }

    /// Distance to the nearest stored point, expanding shells until no closer
    /// point can still be hiding outside the shell already searched.
    fn nearest(&self, q: V3) -> Option<f64> {
        let base = self.key(q);
        let mut best = f64::MAX;
        let mut ring = 0i64;
        loop {
            let shell_min = (ring as f64 - 1.0).max(0.0) * self.cell;
            if best < f64::MAX && shell_min > best {
                return Some(best);
            }
            if ring > 512 {
                return if best < f64::MAX { Some(best) } else { None };
            }
            for dx in -ring..=ring {
                for dy in -ring..=ring {
                    for dz in -ring..=ring {
                        if ring > 0 && dx.abs() != ring && dy.abs() != ring && dz.abs() != ring {
                            continue;
                        }
                        if let Some(b) = self.buckets.get(&(base.0 + dx, base.1 + dy, base.2 + dz))
                        {
                            for p in b {
                                let d = len(sub(*p, q));
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
    let ga = Grid::new(&a);
    let gb = Grid::new(&b);

    let mut forward = 0.0;
    let mut max_gap = 0.0f64;
    for p in &a {
        let d = gb.nearest(*p)?;
        forward += d;
        max_gap = max_gap.max(d);
    }
    forward /= a.len() as f64;

    let mut backward = 0.0;
    for p in &b {
        backward += ga.nearest(*p)?;
    }
    backward /= b.len() as f64;

    let sa = extents(&a);
    let sb = extents(&b);
    Some(Score {
        chamfer: 0.5 * (forward + backward),
        forward,
        backward,
        max_gap,
        samples: a.len() + b.len(),
        size_delta: [sa[0] - sb[0], sa[1] - sb[1], sa[2] - sb[2]],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri(a: V3, b: V3, c: V3) -> Vec<f32> {
        let mut v = Vec::new();
        crate::vec3::push_tri(&mut v, a, b, c);
        v
    }

    #[test]
    fn identical_shapes_score_zero() {
        let soup = tri([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let s = compare(&soup, &soup).unwrap();
        assert_eq!(s.chamfer, 0.0);
        assert_eq!(s.max_gap, 0.0);
    }

    #[test]
    fn displaced_shapes_score_roughly_the_offset() {
        let a = tri([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let b = tri([2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 1.0, 0.0]);
        let s = compare(&a, &b).unwrap();
        assert!(s.chamfer > 1.0 && s.chamfer < 3.0, "chamfer {}", s.chamfer);
    }

    #[test]
    fn empty_input_has_no_score() {
        assert!(compare(&[], &[1.0, 2.0, 3.0]).is_none());
    }
}
