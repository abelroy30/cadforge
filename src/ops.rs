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
