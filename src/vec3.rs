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

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn arithmetic_is_componentwise() {
        assert_eq!(add([1.0, 2.0, 3.0], [10.0, 20.0, 30.0]), [11.0, 22.0, 33.0]);
        assert_eq!(sub([1.0, 2.0, 3.0], [1.0, 1.0, 1.0]), [0.0, 1.0, 2.0]);
        assert_eq!(scale([1.0, -2.0, 0.5], 4.0), [4.0, -8.0, 2.0]);
    }

    #[test]
    fn dot_and_cross_behave() {
        assert!(close(dot([1.0, 2.0, 3.0], [4.0, -5.0, 6.0]), 12.0));
        // x cross y is z: the right-handed rule the rest of the geometry assumes.
        assert_eq!(cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
        assert_eq!(cross([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]), [0.0, -1.0, 0.0]);
        assert_eq!(cross([1.0, 1.0, 1.0], [1.0, 1.0, 1.0]), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn len_is_the_magnitude() {
        assert!(close(len([3.0, 4.0, 0.0]), 5.0));
        assert!(close(len([0.0, 0.0, 0.0]), 0.0));
        assert!(close(len([-2.0, 0.0, 0.0]), 2.0));
    }

    #[test]
    fn normalize_gives_a_unit_vector_without_losing_direction() {
        let n = normalize([0.0, 0.0, 7.5]);
        assert!(close(len(n), 1.0));
        assert!(close(n[2], 1.0), "{n:?}");
    }

    /// `perpendicular` is what turns an axis into a circle, so it has to be both
    /// genuinely perpendicular and unit length, whatever it is handed.
    #[test]
    fn perpendicular_is_perpendicular_and_unit_length() {
        for a in [
            [1.0, 0.0, 0.0],
            [1.0, 2.0, 3.0],
            [0.0, -4.0, 2.0],
            [0.0, 0.0, 1.0],
        ] {
            let p = perpendicular(a);
            assert!(close(dot(a, p), 0.0), "not perpendicular: {a:?} -> {p:?}");
            assert!(close(len(p), 1.0), "not a unit vector: {p:?}");
        }
    }

    #[test]
    fn push_tri_writes_three_vertices_in_order() {
        let mut out = Vec::new();
        push_tri(&mut out, [1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]);
        assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    }
}
