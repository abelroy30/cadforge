//! Rewards: what a training run is actually being paid to do.
//!
//! A reward is a named, weighted measure of how good one generated operation
//! program is. They are computed here, in Rust, rather than inside the trainer,
//! for one reason: a reward decides what a model learns, so it has to be
//! inspectable and reproducible. A number that exists only inside a training
//! script cannot be argued with, and a reward nobody can argue with is a reward
//! nobody can fix.
//!
//! Validity is deliberately *not* a reward. A program that does not build scores
//! zero however good its geometry is, because a term a model can trade against
//! shape is a term it will trade against shape.

use crate::model::Reward;
use crate::ops::Program;
use serde::{Deserialize, Serialize};

/// The reward kinds this build understands. A reward whose kind is not here is
/// reported as unknown rather than silently scored as zero.
pub const KINDS: [&str; 4] = ["geometry", "recorded_history", "simplicity", "scale"];

/// What a program is being scored against: as much of the truth as is known.
///
/// Every field is optional because the truth arrives in pieces. A picture gives
/// no geometry; a STEP part gives geometry but no recorded history; a SolidWorks
/// export gives the history but nothing to rebuild against.
#[derive(Debug, Clone, Default)]
pub struct Target {
    pub triangles: Vec<f32>,
    /// The operations the part was really built with, when that is known.
    pub reference_ops: Vec<String>,
}

/// What one reward had to say about one program.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub weight: f64,
    /// The raw measurement, in its own units: millimetres for geometry, a ratio
    /// elsewhere. Shown so the number can be argued with, not just trusted.
    pub measure: f64,
    /// What that measurement is worth, from 0 to 1.
    pub score: f64,
    pub detail: String,
}

/// A verdict on one program, with the working shown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    /// The weighted mean of the component scores, from 0 to 1.
    pub total: f64,
    /// Whether the program built at all. When this is false the total is 0 and
    /// the reason is in `gate`; nothing else is paid.
    pub valid: bool,
    pub gate: String,
    pub components: Vec<Component>,
    /// Rewards that were on but could not be scored, and why. Reported rather
    /// than quietly dropped: a reward that never votes is worse than none.
    pub skipped: Vec<String>,
}

/// The rewards a studio starts with. Weights are the user's to change; these are
/// only a defensible starting point.
pub fn defaults() -> Vec<Reward> {
    let build = |name: &str, kind: &str, weight: f64, params: serde_json::Value| Reward {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        weight,
        enabled: true,
        params,
        created_at: crate::model::now(),
    };
    vec![
        build(
            "matches the part",
            "geometry",
            1.0,
            serde_json::json!({ "tolerance_mm": 0.5 }),
        ),
        build(
            "matches the history",
            "recorded_history",
            0.6,
            serde_json::json!({}),
        ),
        build(
            "fewest operations",
            "simplicity",
            0.2,
            serde_json::json!({}),
        ),
        build(
            "right size",
            "scale",
            0.4,
            serde_json::json!({ "tolerance": 0.05 }),
        ),
    ]
}

/// Make sure the studio has a reward set. Called once on load, so the defaults
/// are visible and editable rather than hidden inside this file.
pub fn ensure_defaults(db: &mut crate::model::Db) {
    if db.rewards.is_empty() {
        db.rewards = defaults();
    }
}

fn tolerance(params: &serde_json::Value, key: &str, fallback: f64) -> f64 {
    params
        .get(key)
        .and_then(|v| v.as_f64())
        .filter(|v| *v > 0.0)
        .unwrap_or(fallback)
}

/// The axis-aligned extents of a triangle soup.
fn extents(triangles: &[f32]) -> Option<[f64; 3]> {
    if triangles.len() < 3 {
        return None;
    }
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in triangles.chunks_exact(3) {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k] as f64);
            hi[k] = hi[k].max(p[k] as f64);
        }
    }
    Some([
        (hi[0] - lo[0]).max(0.0),
        (hi[1] - lo[1]).max(0.0),
        (hi[2] - lo[2]).max(0.0),
    ])
}

/// The longest common subsequence of two operation sequences, as a ratio.
///
/// A subsequence rather than a set because order is the point: drilling a hole
/// and then extruding the plate over it is not the same part, however similar
/// the list looks. Two ops in the right order score higher than the same two in
/// the wrong order, which is exactly what the studio wants to teach.
fn sequence_ratio(produced: &[String], reference: &[String]) -> f64 {
    if reference.is_empty() {
        return 0.0;
    }
    let n = produced.len();
    let m = reference.len();
    let mut table = vec![vec![0usize; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            table[i][j] = if produced[i - 1] == reference[j - 1] {
                table[i - 1][j - 1] + 1
            } else {
                table[i - 1][j].max(table[i][j - 1])
            };
        }
    }
    table[n][m] as f64 / m as f64
}

/// Score one program against the truth, with the working shown.
pub fn evaluate(program: &Program, target: &Target, rewards: &[Reward]) -> Verdict {
    if let Err(e) = program.validate() {
        return Verdict {
            total: 0.0,
            valid: false,
            gate: e,
            components: Vec::new(),
            skipped: Vec::new(),
        };
    }

    let produced_ops: Vec<String> = program.op_names().iter().map(|s| s.to_string()).collect();
    let rebuilt = match crate::solid::evaluate(program) {
        Ok(r) => r,
        Err(e) => {
            return Verdict {
                total: 0.0,
                valid: false,
                gate: format!("the program does not build: {e}"),
                components: Vec::new(),
                skipped: Vec::new(),
            }
        }
    };
    let built = rebuilt.solid.triangulate();

    let mut components: Vec<Component> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    for r in rewards {
        if !r.enabled {
            continue;
        }
        let (measure, score, detail) = match r.kind.as_str() {
            "geometry" => {
                if target.triangles.is_empty() || built.is_empty() {
                    skipped.push(format!("{}\u{2009}- nothing to compare against", r.name));
                    continue;
                }
                match crate::score::compare(&built, &target.triangles) {
                    Some(s) => {
                        let tol = tolerance(&r.params, "tolerance_mm", 0.5);
                        let value = 1.0 / (1.0 + s.chamfer / tol);
                        (
                            s.chamfer,
                            value,
                            format!(
                                "chamfer {:.3} mm (worst point {:.3} mm)",
                                s.chamfer, s.max_gap
                            ),
                        )
                    }
                    None => {
                        skipped.push(format!("{}\u{2009}- could not be compared", r.name));
                        continue;
                    }
                }
            }
            "recorded_history" => {
                if target.reference_ops.is_empty() {
                    skipped.push(format!(
                        "{}\u{2009}- no recorded history for this part",
                        r.name
                    ));
                    continue;
                }
                let ratio = sequence_ratio(&produced_ops, &target.reference_ops);
                (
                    ratio,
                    ratio,
                    format!(
                        "{} of {} recorded operations, in order",
                        (ratio * target.reference_ops.len() as f64).round() as usize,
                        target.reference_ops.len()
                    ),
                )
            }
            "simplicity" => {
                if target.reference_ops.is_empty() {
                    // Without a reference there is no such thing as "fewer than
                    // needed", only "fewer". A term that paid out regardless would
                    // teach the model to delete operations, so it abstains.
                    skipped.push(format!("{}\u{2009}- no reference operation count", r.name));
                    continue;
                }
                let n = produced_ops.len().max(1);
                let ideal = target.reference_ops.len();
                let value = (ideal as f64 / n as f64).min(1.0);
                (
                    n as f64,
                    value,
                    format!("{n} operation(s), against {ideal} in the reference"),
                )
            }
            "scale" => {
                let want = match extents(&target.triangles) {
                    Some(e) => e,
                    None => {
                        skipped.push(format!("{}\u{2009}- no reference size", r.name));
                        continue;
                    }
                };
                let got = match extents(&built) {
                    Some(e) => e,
                    None => {
                        skipped.push(format!("{}\u{2009}- the rebuild has no size", r.name));
                        continue;
                    }
                };
                let tol = tolerance(&r.params, "tolerance", 0.05);
                // The mean relative error across the axes, turned into a score.
                let mut error = 0.0;
                for k in 0..3 {
                    let want_k = want[k].max(1e-6);
                    error += ((got[k] - want[k]).abs() / want_k).min(1.0);
                }
                error /= 3.0;
                (
                    error,
                    (1.0 - error / tol).clamp(0.0, 1.0),
                    format!(
                        "{:.2} x {:.2} x {:.2} mm, against {:.2} x {:.2} x {:.2}",
                        got[0], got[1], got[2], want[0], want[1], want[2]
                    ),
                )
            }
            other => {
                skipped.push(format!("{}\u{2009}- unknown reward kind {other:?}", r.name));
                continue;
            }
        };

        components.push(Component {
            id: r.id.clone(),
            name: r.name.clone(),
            kind: r.kind.clone(),
            weight: r.weight,
            measure,
            score,
            detail,
        });
    }

    let weight_sum: f64 = components.iter().map(|c| c.weight).sum();
    let total = if weight_sum > 0.0 {
        components.iter().map(|c| c.weight * c.score).sum::<f64>() / weight_sum
    } else {
        0.0
    };

    Verdict {
        total,
        valid: true,
        gate: "the program builds".into(),
        components,
        skipped,
    }
}

/// Read the reward set off disk for a trainer, in the form a script can use.
///
/// The manifest carries this, so a finished run can always be explained after
/// the fact even if the rewards have been edited since - which is the whole
/// reason the reward travels with the data instead of living in the trainer.
pub fn spec_for_manifest(db: &crate::model::Db) -> serde_json::Value {
    serde_json::json!(db
        .rewards
        .iter()
        .filter(|r| r.enabled)
        .map(|r| serde_json::json!({
            "id": r.id,
            "name": r.name,
            "kind": r.kind,
            "weight": r.weight,
            "params": r.params,
        }))
        .collect::<Vec<_>>())
}

/// Score a program the studio keeps on disk, if it has one. This is what makes a
/// reward set arguable: the same numbers the trainer will use, shown now.
pub fn preview(program: &Program, target: &Target, db: &crate::model::Db) -> Verdict {
    evaluate(program, target, &db.rewards)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{Op, Profile, Program};

    fn plate() -> Program {
        Program {
            version: crate::ops::VERSION,
            units: "mm".into(),
            ops: vec![
                Op::Extrude {
                    profile: Profile::Polygon {
                        points: vec![
                            [0.0, 0.0, 0.0],
                            [60.0, 0.0, 0.0],
                            [60.0, 40.0, 0.0],
                            [0.0, 40.0, 0.0],
                        ],
                    },
                    distance: 10.0,
                    direction: [0.0, 0.0, 1.0],
                },
                Op::Hole {
                    diameter: 8.0,
                    through: true,
                    depth: 0.0,
                    at: [30.0, 20.0, 0.0],
                    axis: [0.0, 0.0, 1.0],
                },
            ],
        }
    }

    fn target_of(program: &Program) -> Target {
        let built = crate::solid::evaluate(program).unwrap();
        Target {
            triangles: built.solid.triangulate(),
            reference_ops: program.op_names().iter().map(|s| s.to_string()).collect(),
        }
    }

    fn only(kind: &str, weight: f64) -> Vec<Reward> {
        vec![Reward {
            id: "r".into(),
            name: kind.into(),
            kind: kind.into(),
            weight,
            enabled: true,
            params: serde_json::json!({}),
            created_at: String::new(),
        }]
    }

    #[test]
    fn a_program_that_matches_its_own_part_scores_well() {
        let p = plate();
        let t = target_of(&p);
        let v = evaluate(&p, &t, &defaults());
        assert!(v.valid, "{}", v.gate);
        assert!(
            v.total > 0.9,
            "a perfect answer scored {}: {:?}",
            v.total,
            v.components
        );
        assert_eq!(v.components.len(), 4, "every default should have voted");
    }

    #[test]
    fn a_program_that_does_not_build_scores_zero_however_good_its_shape_is() {
        // Validity is a gate rather than a weighted term on purpose: a reward a
        // model can trade against shape is a reward it will trade against shape.
        let mut broken = plate();
        broken.version = crate::ops::VERSION + 1;
        let v = evaluate(&broken, &target_of(&plate()), &defaults());
        assert!(!v.valid);
        assert_eq!(v.total, 0.0);
        assert!(
            v.components.is_empty(),
            "nothing should be paid for a broken program"
        );
    }

    #[test]
    fn the_wrong_shape_scores_worse_than_the_right_one() {
        let p = plate();
        let t = target_of(&p);
        let mut wrong = plate();
        wrong.ops.pop();
        wrong.ops.push(Op::Hole {
            diameter: 3.0,
            through: false,
            depth: 2.0,
            at: [5.0, 5.0, 0.0],
            axis: [0.0, 0.0, 1.0],
        });
        let good = evaluate(&p, &t, &only("geometry", 1.0)).total;
        let bad = evaluate(&wrong, &t, &only("geometry", 1.0)).total;
        assert!(
            bad < good,
            "the wrong hole scored {bad}, the right one {good}"
        );
    }

    #[test]
    fn order_matters_to_the_recorded_history_reward() {
        let p = plate();
        let t = target_of(&p);
        let forwards = evaluate(&p, &t, &only("recorded_history", 1.0)).total;

        let mut backwards = plate();
        backwards.ops.reverse();
        let other = Target {
            triangles: t.triangles.clone(),
            reference_ops: vec!["extrude".into(), "hole".into()],
        };
        let reversed = evaluate(&backwards, &other, &only("recorded_history", 1.0)).total;
        assert_eq!(forwards, 1.0, "the same order must score a full mark");
        assert!(
            reversed < 1.0,
            "the same operations in the wrong order scored {reversed}"
        );
    }

    #[test]
    fn a_disabled_reward_does_not_vote_at_all() {
        let p = plate();
        let t = target_of(&p);
        let mut rewards = only("geometry", 1.0);
        rewards.push(Reward {
            id: "s".into(),
            name: "simplicity".into(),
            kind: "simplicity".into(),
            weight: 100.0,
            enabled: false,
            params: serde_json::json!({}),
            created_at: String::new(),
        });
        let v = evaluate(&p, &t, &rewards);
        assert_eq!(v.components.len(), 1, "a disabled reward must be left out");
        assert!(
            v.total > 0.9,
            "a disabled reward must not drag the total down"
        );
    }

    #[test]
    fn a_reward_with_nothing_to_measure_against_says_so() {
        // A picture gives no geometry and no history: the honest answer is that
        // the reward could not vote, not a silent zero that looks like failure.
        let v = evaluate(&plate(), &Target::default(), &defaults());
        assert!(v.valid);
        assert_eq!(v.components.len(), 0);
        assert_eq!(v.skipped.len(), 4, "{:?}", v.skipped);
        assert_eq!(v.total, 0.0);
    }

    #[test]
    fn an_unknown_reward_kind_is_reported_not_scored() {
        let v = evaluate(&plate(), &target_of(&plate()), &only("vibes", 1.0));
        assert_eq!(v.components.len(), 0);
        assert!(v.skipped[0].contains("vibes"), "{:?}", v.skipped);
    }

    #[test]
    fn the_defaults_are_seeded_once_and_left_alone_after() {
        let mut db = crate::model::Db::default();
        ensure_defaults(&mut db);
        assert_eq!(db.rewards.len(), 4);
        let first = db.rewards[0].id.clone();
        db.rewards[0].weight = 9.0;
        ensure_defaults(&mut db);
        assert_eq!(db.rewards.len(), 4, "seeding must not run twice");
        assert_eq!(db.rewards[0].id, first);
        assert_eq!(db.rewards[0].weight, 9.0, "a tuned weight must survive");
    }

    #[test]
    fn the_manifest_spec_carries_only_the_rewards_that_vote() {
        let mut db = crate::model::Db::default();
        ensure_defaults(&mut db);
        db.rewards[0].enabled = false;
        let spec = spec_for_manifest(&db);
        let list = spec.as_array().unwrap();
        assert_eq!(list.len(), 3);
        assert!(list.iter().all(|r| r["kind"] != "geometry"));
    }
}
