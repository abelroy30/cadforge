//! Tiny 3D vector helpers shared by the geometry code.

pub type V3 = [f64; 3];

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}

pub fn normalize(a: V3) -> V3 {
    let m = len(a);
    if m > 1e-9 {
        [a[0] / m, a[1] / m, a[2] / m]
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// Any unit vector perpendicular to `a`.
pub fn perpendicular(a: V3) -> V3 {
    let seed = if a[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    normalize(cross(a, seed))
}

/// Extend a flat vertex buffer with one triangle.
pub fn push_tri(out: &mut Vec<f32>, a: V3, b: V3, c: V3) {
    for p in [a, b, c] {
        out.push(p[0] as f32);
        out.push(p[1] as f32);
        out.push(p[2] as f32);
    }
}
