//! The operation program: what a model is asked to produce.
//!
//! The prose in `step::Summary::steps` is for people. This is the machine form -
//! a short, versioned list of CAD operations that a driver can execute. A model
//! is trained to emit this; the rebuild driver turns it back into geometry, and
//! a score says how close it came.

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    pub version: u32,
    #[serde(default = "units_default")]
    pub units: String,
    pub ops: Vec<Op>,
}

fn units_default() -> String {
    "mm".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Push a closed profile along a direction.
    Extrude {
        profile: Profile,
        distance: f64,
        #[serde(default = "up_default")]
        direction: [f64; 3],
    },
    /// Drill one hole.
    Hole {
        diameter: f64,
        #[serde(default)]
        through: bool,
        #[serde(default)]
        depth: f64,
        at: [f64; 3],
        #[serde(default = "up_default")]
        axis: [f64; 3],
    },
    /// The same hole, several times. What a pattern is once it is measured.
    HolePattern {
        diameter: f64,
        #[serde(default)]
        through: bool,
        #[serde(default)]
        depth: f64,
        positions: Vec<[f64; 3]>,
        #[serde(default = "up_default")]
        axis: [f64; 3],
    },
    /// Round an edge. The native engine reports this as unsupported.
    Fillet { radius: f64 },
}

fn up_default() -> [f64; 3] {
    [0.0, 0.0, 1.0]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Profile {
    /// A closed, coplanar polygon.
    Polygon { points: Vec<[f64; 3]> },
    Circle {
        center: [f64; 3],
        #[serde(default = "up_default")]
        normal: [f64; 3],
        diameter: f64,
    },
}

impl Program {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != VERSION {
            return Err(format!("unsupported program version {}", self.version));
        }
        if self.ops.is_empty() {
            return Err("program has no operations".into());
        }
        for (i, op) in self.ops.iter().enumerate() {
            let n = i + 1;
            match op {
                Op::Extrude {
                    profile, distance, ..
                } => {
                    if !(distance.is_finite() && *distance > 0.0) {
                        return Err(format!("op {n}: extrude distance must be positive"));
                    }
                    match profile {
                        Profile::Polygon { points } => {
                            if points.len() < 3 {
                                return Err(format!("op {n}: profile needs at least 3 points"));
                            }
                        }
                        Profile::Circle { diameter, .. } => {
                            if !(diameter.is_finite() && *diameter > 0.0) {
                                return Err(format!("op {n}: circle diameter must be positive"));
                            }
                        }
                    }
                }
                Op::Hole {
                    diameter,
                    depth,
                    through,
                    ..
                } => {
                    if !(diameter.is_finite() && *diameter > 0.0) {
                        return Err(format!("op {n}: hole diameter must be positive"));
                    }
                    if !through && !(depth.is_finite() && *depth > 0.0) {
                        return Err(format!("op {n}: a blind hole needs a positive depth"));
                    }
                }
                Op::HolePattern {
                    diameter,
                    through,
                    depth,
                    positions,
                    ..
                } => {
                    if !(diameter.is_finite() && *diameter > 0.0) {
                        return Err(format!("op {n}: hole diameter must be positive"));
                    }
                    if positions.is_empty() {
                        return Err(format!("op {n}: pattern has no positions"));
                    }
                    if !through && !(depth.is_finite() && *depth > 0.0) {
                        return Err(format!("op {n}: a blind pattern needs a positive depth"));
                    }
                }
                Op::Fillet { radius } => {
                    if !(radius.is_finite() && *radius > 0.0) {
                        return Err(format!("op {n}: fillet radius must be positive"));
                    }
                }
            }
        }
        Ok(())
    }

    /// Short names of the operations, in order.
    pub fn op_names(&self) -> Vec<&'static str> {
        self.ops
            .iter()
            .map(|o| match o {
                Op::Extrude { .. } => "extrude",
                Op::Hole { .. } => "hole",
                Op::HolePattern { .. } => "hole_pattern",
                Op::Fillet { .. } => "fillet",
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three_sided() -> Program {
        Program {
            version: VERSION,
            units: "mm".into(),
            ops: vec![Op::Extrude {
                profile: Profile::Polygon {
                    points: vec![[0.0, 0.0, 0.0], [20.0, 0.0, 0.0], [20.0, 10.0, 0.0]],
                },
                distance: 5.0,
                direction: [0.0, 0.0, 1.0],
            }],
        }
    }

    /// This JSON is a contract with `ml/`: the trainer is taught to emit it and the
    /// drivers are built to read it, so the discriminants and the unit must not
    /// drift without both sides being changed together.
    #[test]
    fn json_uses_the_names_a_model_is_trained_on() {
        let text = serde_json::to_string(&Program {
            version: VERSION,
            units: "mm".into(),
            ops: vec![
                Op::Extrude {
                    profile: Profile::Circle {
                        center: [0.0, 0.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                        diameter: 40.0,
                    },
                    distance: 8.0,
                    direction: [0.0, 0.0, 1.0],
                },
                Op::HolePattern {
                    diameter: 6.0,
                    through: true,
                    depth: 0.0,
                    positions: vec![[12.0, 10.0, 0.0], [48.0, 10.0, 0.0]],
                    axis: [0.0, 0.0, 1.0],
                },
                Op::Fillet { radius: 2.0 },
            ],
        })
        .unwrap();
        assert!(text.contains(r#""op":"extrude""#), "{text}");
        assert!(text.contains(r#""shape":"circle""#), "{text}");
        assert!(text.contains(r#""op":"hole_pattern""#), "{text}");
        assert!(text.contains(r#""op":"fillet""#), "{text}");
        assert!(text.contains(r#""units":"mm""#), "{text}");
    }

    /// A model that leaves out a field it was not asked about should still parse
    /// into a valid program, rather than costing the whole answer.
    #[test]
    fn omitted_fields_fall_back_to_their_defaults() {
        let text = r#"{"version":1,"ops":[{"op":"extrude","profile":{"shape":"circle","center":[0,0,0],"diameter":10},"distance":4}]}"#;
        let p: Program = serde_json::from_str(text).unwrap();
        assert_eq!(p.units, "mm");
        match &p.ops[0] {
            Op::Extrude { direction, .. } => assert_eq!(*direction, [0.0, 0.0, 1.0]),
            other => panic!("expected an extrude, got {other:?}"),
        }
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }

    #[test]
    fn a_program_the_step_reader_builds_is_valid() {
        assert!(three_sided().validate().is_ok());
    }

    /// Each case is something a driver could not build, so validation has to catch
    /// it here rather than let it reach an engine.
    #[test]
    fn validate_refuses_what_a_driver_could_not_build() {
        let poly = |points: Vec<[f64; 3]>, distance: f64| Op::Extrude {
            profile: Profile::Polygon { points },
            distance,
            direction: [0.0, 0.0, 1.0],
        };
        let cases: Vec<(Program, &str)> = vec![
            (
                Program {
                    version: 99,
                    ..three_sided()
                },
                "version",
            ),
            (
                Program {
                    ops: vec![],
                    ..three_sided()
                },
                "no operations",
            ),
            (
                Program {
                    ops: vec![poly(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]], 1.0)],
                    ..three_sided()
                },
                "at least 3 points",
            ),
            (
                Program {
                    ops: vec![poly(
                        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                        0.0,
                    )],
                    ..three_sided()
                },
                "distance must be positive",
            ),
            (
                Program {
                    ops: vec![Op::Extrude {
                        profile: Profile::Circle {
                            center: [0.0, 0.0, 0.0],
                            normal: [0.0, 0.0, 1.0],
                            diameter: 0.0,
                        },
                        distance: 1.0,
                        direction: [0.0, 0.0, 1.0],
                    }],
                    ..three_sided()
                },
                "diameter must be positive",
            ),
            (
                Program {
                    ops: vec![Op::Hole {
                        diameter: 6.0,
                        through: false,
                        depth: 0.0,
                        at: [0.0, 0.0, 0.0],
                        axis: [0.0, 0.0, 1.0],
                    }],
                    ..three_sided()
                },
                "blind hole needs a positive depth",
            ),
            (
                Program {
                    ops: vec![Op::HolePattern {
                        diameter: 6.0,
                        through: true,
                        depth: 0.0,
                        positions: vec![],
                        axis: [0.0, 0.0, 1.0],
                    }],
                    ..three_sided()
                },
                "no positions",
            ),
            (
                Program {
                    ops: vec![Op::Fillet { radius: -1.0 }],
                    ..three_sided()
                },
                "fillet radius",
            ),
        ];

        for (program, want) in cases {
            let err = program
                .validate()
                .expect_err("this should have been refused");
            assert!(err.contains(want), "expected {want:?} in {err:?}");
        }
    }

    /// A through hole needs no depth, which is the one case the rule above must
    /// not catch.
    #[test]
    fn a_through_hole_needs_no_depth() {
        let p = Program {
            ops: vec![Op::Hole {
                diameter: 6.0,
                through: true,
                depth: 0.0,
                at: [10.0, 5.0, 0.0],
                axis: [0.0, 0.0, 1.0],
            }],
            ..three_sided()
        };
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }

    #[test]
    fn op_names_come_back_in_order() {
        let p = Program {
            ops: vec![
                Op::Fillet { radius: 1.0 },
                Op::HolePattern {
                    diameter: 6.0,
                    through: true,
                    depth: 0.0,
                    positions: vec![[0.0, 0.0, 0.0]],
                    axis: [0.0, 0.0, 1.0],
                },
                Op::Hole {
                    diameter: 6.0,
                    through: true,
                    depth: 0.0,
                    at: [0.0, 0.0, 0.0],
                    axis: [0.0, 0.0, 1.0],
                },
            ],
            ..three_sided()
        };
        assert_eq!(p.op_names(), vec!["fillet", "hole_pattern", "hole"]);
    }

    /// The version is what lets the format grow without silently misreading an old
    /// program, so it is written by callers from this constant.
    #[test]
    fn the_version_constant_is_what_validate_expects() {
        let p = Program {
            version: VERSION,
            ..three_sided()
        };
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }
}
