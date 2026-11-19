//! The native engine: execute an operation program without a CAD kernel.
//!
//! A real kernel does boolean subtraction; this does not. It builds the same
//! *surface model* a STEP file of the part carries - the outer faces plus the
//! cylindrical walls of the holes - which is exactly what we compare against,
//! so the round trip is meaningful even though the modelling is not general.
//! Fillets are reported as unsupported rather than silently dropped.

use crate::ops::{Op, Profile, Program};
use crate::vec3::{add, cross, dot, len, normalize, perpendicular, push_tri, scale, sub, V3};

/// Segments used when a circle or a cylinder has to become triangles.
const SEGMENTS: usize = 32;

#[derive(Debug, Clone)]
pub enum Face {
    Planar {
        points: Vec<V3>,
    },
    Cylindrical {
        origin: V3,
        axis: V3,
        radius: f64,
        length: f64,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Solid {
    pub faces: Vec<Face>,
}

pub struct Rebuild {
    pub solid: Solid,
    pub warnings: Vec<String>,
    pub ops_applied: usize,
}

impl Solid {
    /// Every point the faces know about. Cylinders contribute their axis ends.
    fn each_point(&self, f: &mut dyn FnMut(V3)) {
        for face in &self.faces {
            match face {
                Face::Planar { points } => {
                    for p in points {
                        f(*p);
                    }
                }
                Face::Cylindrical {
                    origin,
                    axis,
                    length,
                    ..
                } => {
                    let n = normalize(*axis);
                    f(*origin);
                    f(add(*origin, scale(n, *length)));
                }
            }
        }
    }

    /// How far the solid reaches along an axis, as (min, max).
    pub fn extent_along(&self, axis: V3) -> Option<(f64, f64)> {
        let n = normalize(axis);
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        let mut seen = false;
        self.each_point(&mut |p| {
            seen = true;
            let d = dot(p, n);
            lo = lo.min(d);
            hi = hi.max(d);
        });
        if seen {
            Some((lo, hi))
        } else {
            None
        }
    }

    /// Flat triangle soup, for the viewport and for scoring.
    pub fn triangulate(&self) -> Vec<f32> {
        let mut out = Vec::new();
        for face in &self.faces {
            match face {
                Face::Planar { points } => {
                    if points.len() < 3 {
                        continue;
                    }
                    let mut c = [0.0f64; 3];
                    for p in points {
                        c = add(c, *p);
                    }
                    c = scale(c, 1.0 / points.len() as f64);
                    // Fan from the centre, closing the loop. `windows(2)` stops one
                    // pair short, which leaves the wedge between the last vertex and
                    // the first unswept - a hole in the face that both the preview
                    // and the score can see.
                    let n = points.len();
                    for i in 0..n {
                        let a = points[i];
                        let b = points[(i + 1) % n];
                        // A loop that repeats its first point contributes a
                        // zero-width wedge; skip it rather than emit a sliver.
                        if len(sub(b, a)) < 1e-12 {
                            continue;
                        }
                        push_tri(&mut out, c, a, b);
                    }
                }
                Face::Cylindrical {
                    origin,
                    axis,
                    radius,
                    length,
                } => {
                    let n = normalize(*axis);
                    let u = perpendicular(n);
                    let v = cross(n, u);
                    let at = |t: f64, ang: f64| -> V3 {
                        let rim = add(scale(u, ang.cos() * radius), scale(v, ang.sin() * radius));
                        add(add(*origin, scale(n, t)), rim)
                    };
                    for i in 0..SEGMENTS {
                        let a0 = i as f64 / SEGMENTS as f64 * std::f64::consts::TAU;
                        let a1 = (i + 1) as f64 / SEGMENTS as f64 * std::f64::consts::TAU;
                        let p00 = at(0.0, a0);
                        let p01 = at(*length, a0);
                        let p10 = at(0.0, a1);
                        let p11 = at(*length, a1);
                        push_tri(&mut out, p00, p10, p11);
                        push_tri(&mut out, p00, p11, p01);
                    }
                }
            }
        }
        out
    }
}

pub fn evaluate(program: &Program) -> Result<Rebuild, String> {
    program.validate()?;
    let mut solid = Solid::default();
    let mut warnings: Vec<String> = Vec::new();
    let mut ops_applied = 0usize;

    for (i, op) in program.ops.iter().enumerate() {
        let n = i + 1;
        match op {
            Op::Extrude {
                profile,
                distance,
                direction,
            } => {
                let dir = normalize(*direction);
                match profile {
                    Profile::Polygon { points } => {
                        let top: Vec<V3> = points
                            .iter()
                            .map(|p| add(*p, scale(dir, *distance)))
                            .collect();
                        solid.faces.push(Face::Planar {
                            points: points.clone(),
                        });
                        solid.faces.push(Face::Planar {
                            points: top.clone(),
                        });
                        for w in 0..points.len() {
                            let a = points[w];
                            let b = points[(w + 1) % points.len()];
                            solid.faces.push(Face::Planar {
                                points: vec![
                                    a,
                                    b,
                                    add(b, scale(dir, *distance)),
                                    add(a, scale(dir, *distance)),
                                ],
                            });
                        }
                        ops_applied += 1;
                    }
                    Profile::Circle {
                        center, diameter, ..
                    } => {
                        let radius = diameter / 2.0;
                        let u = perpendicular(dir);
                        let v = cross(dir, u);
                        let mut bottom = Vec::new();
                        let mut top = Vec::new();
                        for k in 0..SEGMENTS {
                            let ang = k as f64 / SEGMENTS as f64 * std::f64::consts::TAU;
                            let rim =
                                add(scale(u, ang.cos() * radius), scale(v, ang.sin() * radius));
                            bottom.push(add(*center, rim));
                            top.push(add(add(*center, rim), scale(dir, *distance)));
                        }
                        solid.faces.push(Face::Planar { points: bottom });
                        solid.faces.push(Face::Planar { points: top });
                        solid.faces.push(Face::Cylindrical {
                            origin: *center,
                            axis: dir,
                            radius,
                            length: *distance,
                        });
                        ops_applied += 1;
                    }
                }
            }
            Op::Hole {
                diameter,
                through,
                depth,
                at,
                axis,
            } => {
                if let Some(face) = drill(
                    &solid,
                    *diameter,
                    *through,
                    *depth,
                    *at,
                    *axis,
                    n,
                    &mut warnings,
                ) {
                    solid.faces.push(face);
                    ops_applied += 1;
                }
            }
            Op::HolePattern {
                diameter,
                through,
                depth,
                positions,
                axis,
            } => {
                let mut made = 0;
                for p in positions {
                    if let Some(face) = drill(
                        &solid,
                        *diameter,
                        *through,
                        *depth,
                        *p,
                        *axis,
                        n,
                        &mut warnings,
                    ) {
                        solid.faces.push(face);
                        made += 1;
                    }
                }
                if made > 0 {
                    ops_applied += 1;
                }
            }
            Op::Fillet { .. } => {
                warnings.push(format!(
                    "op {n}: the native engine cannot fillet; FreeCAD can. The rest of the shape is complete."
                ));
            }
        }
    }
    Ok(Rebuild {
        solid,
        warnings,
        ops_applied,
    })
}

/// Build the cylindrical wall of one hole.
fn drill(
    solid: &Solid,
    diameter: f64,
    through: bool,
    depth: f64,
    at: V3,
    axis: V3,
    op: usize,
    warnings: &mut Vec<String>,
) -> Option<Face> {
    if solid.faces.is_empty() {
        warnings.push(format!("op {op}: cannot drill before anything exists"));
        return None;
    }
    let a = normalize(axis);
    let (lo, hi) = solid.extent_along(a)?;
    let (start, length) = if through {
        (lo, hi - lo)
    } else {
        (dot(at, a), depth)
    };
    let origin = add(at, scale(a, start - dot(at, a)));
    Some(Face::Cylindrical {
        origin,
        axis: a,
        radius: diameter / 2.0,
        length,
    })
}

/// Andrew's monotone chain. Turns a point cloud into a profile outline.
pub fn convex_hull_2d(mut pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    pts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    pts.dedup();
    if pts.len() <= 2 {
        return pts;
    }
    let turn = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| -> f64 {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    let mut lower: Vec<(f64, f64)> = Vec::new();
    for p in &pts {
        while lower.len() >= 2 && turn(lower[lower.len() - 2], lower[lower.len() - 1], *p) <= 0.0 {
            lower.pop();
        }
        lower.push(*p);
    }
    let mut upper: Vec<(f64, f64)> = Vec::new();
    for p in pts.iter().rev() {
        while upper.len() >= 2 && turn(upper[upper.len() - 2], upper[upper.len() - 1], *p) <= 0.0 {
            upper.pop();
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vec3::{cross, len, sub};

    fn square(w: f64, d: f64) -> Profile {
        Profile::Polygon {
            points: vec![[0.0, 0.0, 0.0], [w, 0.0, 0.0], [w, d, 0.0], [0.0, d, 0.0]],
        }
    }

    fn program(ops: Vec<Op>) -> Program {
        Program {
            version: crate::ops::VERSION,
            units: "mm".into(),
            ops,
        }
    }

    fn extrude(w: f64, d: f64, up: f64) -> Op {
        Op::Extrude {
            profile: square(w, d),
            distance: up,
            direction: [0.0, 0.0, 1.0],
        }
    }

    fn soup_area(tris: &[f32]) -> f64 {
        let mut total = 0.0;
        for t in tris.chunks_exact(9) {
            let a = [t[0] as f64, t[1] as f64, t[2] as f64];
            let b = [t[3] as f64, t[4] as f64, t[5] as f64];
            let c = [t[6] as f64, t[7] as f64, t[8] as f64];
            total += 0.5 * len(cross(sub(b, a), sub(c, a)));
        }
        total
    }

    /// The bug this guards: a fan over a face's loop has to include the wedge
    /// between the last vertex and the first. Missing it leaves a hole in every
    /// face - invisible in a preview, but read by `score` as a real deviation of
    /// several millimetres, which is exactly the kind of thing a training signal
    /// must not invent.
    ///
    /// Area is the test that cannot be fooled: a missing wedge loses area, a
    /// doubled one gains it, and counting triangles would notice neither.
    #[test]
    fn every_face_is_fully_triangulated() {
        let rebuilt = evaluate(&program(vec![extrude(20.0, 10.0, 5.0)])).unwrap();
        let tris = rebuilt.solid.triangulate();

        // Six quad faces, one triangle per edge of every loop.
        assert_eq!(tris.len() / 9, 6 * 4);

        let expected = 2.0 * (20.0 * 10.0) + 2.0 * (20.0 * 5.0) + 2.0 * (10.0 * 5.0);
        let area = soup_area(&tris);
        assert!(
            (area - expected).abs() < 1e-6,
            "surface area {area} should be {expected}"
        );
    }

    #[test]
    fn an_extrude_is_as_thick_as_it_was_told_to_be() {
        let rebuilt = evaluate(&program(vec![extrude(20.0, 10.0, 5.0)])).unwrap();
        assert_eq!(rebuilt.ops_applied, 1);
        assert!(rebuilt.warnings.is_empty(), "{:?}", rebuilt.warnings);

        let tris = rebuilt.solid.triangulate();
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        for t in tris.chunks_exact(9) {
            for k in [2usize, 5, 8] {
                let z = t[k] as f64;
                lo = lo.min(z);
                hi = hi.max(z);
            }
        }
        assert!((hi - lo - 5.0).abs() < 1e-9, "thickness {}", hi - lo);
        assert!(lo.abs() < 1e-9, "should start at the origin, got {lo}");
    }

    /// The engine cannot fillet. Saying so is part of the contract: a caller has to
    /// be able to tell a rounded part from one where the round was dropped.
    #[test]
    fn a_fillet_is_reported_rather_than_dropped() {
        let rebuilt = evaluate(&program(vec![
            extrude(20.0, 10.0, 5.0),
            Op::Fillet { radius: 1.0 },
        ]))
        .unwrap();
        assert!(
            rebuilt.warnings.iter().any(|w| w.contains("fillet")),
            "the engine must say it cannot fillet: {:?}",
            rebuilt.warnings
        );
        // What it could build is still there, complete.
        assert_eq!(rebuilt.solid.triangulate().len() / 9, 6 * 4);
    }

    #[test]
    fn drilling_before_anything_exists_says_so() {
        let rebuilt = evaluate(&program(vec![Op::Hole {
            diameter: 6.0,
            through: true,
            depth: 0.0,
            at: [10.0, 5.0, 0.0],
            axis: [0.0, 0.0, 1.0],
        }]))
        .unwrap();
        assert!(
            rebuilt.warnings.iter().any(|w| w.contains("cannot drill")),
            "warnings: {:?}",
            rebuilt.warnings
        );
    }

    #[test]
    fn a_hole_pattern_drills_one_wall_per_position() {
        let rebuilt = evaluate(&program(vec![
            extrude(60.0, 40.0, 8.0),
            Op::HolePattern {
                diameter: 6.0,
                through: true,
                depth: 0.0,
                positions: vec![
                    [12.0, 10.0, 0.0],
                    [48.0, 10.0, 0.0],
                    [12.0, 30.0, 0.0],
                    [48.0, 30.0, 0.0],
                ],
                axis: [0.0, 0.0, 1.0],
            },
        ]))
        .unwrap();
        assert_eq!(rebuilt.ops_applied, 2);
        assert!(rebuilt.warnings.is_empty(), "{:?}", rebuilt.warnings);

        let walls = rebuilt
            .solid
            .faces
            .iter()
            .filter(|f| matches!(f, Face::Cylindrical { .. }))
            .count();
        assert_eq!(walls, 4, "one wall per hole");
    }

    #[test]
    fn the_hull_drops_points_inside_and_on_an_edge() {
        let hull = convex_hull_2d(vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (5.0, 5.0), // inside
            (5.0, 0.0), // on an edge
        ]);
        assert_eq!(hull.len(), 4, "{hull:?}");
    }

    #[test]
    fn the_hull_of_a_triangle_is_its_three_corners() {
        let hull = convex_hull_2d(vec![(0.0, 0.0), (4.0, 0.0), (0.0, 4.0), (1.0, 1.0)]);
        assert_eq!(hull.len(), 3, "{hull:?}");
    }

    #[test]
    fn a_cylinder_face_can_be_measured_along_its_axis() {
        let rebuilt = evaluate(&program(vec![
            extrude(60.0, 40.0, 8.0),
            Op::HolePattern {
                diameter: 6.0,
                through: true,
                depth: 0.0,
                positions: vec![[12.0, 10.0, 0.0]],
                axis: [0.0, 0.0, 1.0],
            },
        ]))
        .unwrap();
        let wall = rebuilt
            .solid
            .faces
            .iter()
            .find_map(|f| match f {
                Face::Cylindrical { radius, length, .. } => Some((*radius, *length)),
                _ => None,
            })
            .expect("a hole should leave a cylindrical wall");
        assert!((wall.0 - 3.0).abs() < 1e-9, "radius {}", wall.0);
        assert!((wall.1 - 8.0).abs() < 1e-9, "length {}", wall.1);
    }
}
