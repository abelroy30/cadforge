//! Turning a picture into build steps - honestly.
//!
//! This is a **geometric baseline**, not a trained network, and it says so in
//! everything it returns. It measures the silhouette of a part in a picture: is
//! it a rectangle, how big, how many holes does it have, where are they - and
//! writes the operations that would produce exactly that. It exists for three
//! reasons:
//!
//! * the picture-to-steps loop is visible and testable with no model loaded, so
//!   the plumbing can be proved before a single weight is downloaded;
//! * a trained model has a baseline to be measured against, which is the only
//!   way to know whether training helped at all;
//! * when `CADFORGE_VISION` names a real model command, that is run instead,
//!   and the reply is shaped exactly like this one.
//!
//! The pixels arrive from the browser, which can already decode every format a
//! user might drop (PNG, JPEG, BMP, WebP) - so the studio never has to carry an
//! image decoder, and this file stays pure geometry-in, operations-out.

use crate::ops::{Op, Profile, Program};
use crate::step::StepLine;
use serde::{Deserialize, Serialize};

/// What a picture was read as, and the operations that would build it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inference {
    pub source: String,
    pub width: usize,
    pub height: usize,
    /// Prose steps, the same shape the build-history view already uses.
    pub steps: Vec<StepLine>,
    /// The machine form: what the rebuild driver executes.
    pub program: Program,
    /// How much the picture actually supports the reading, 0..1.
    pub confidence: f32,
    /// Fraction of the bounding box the shape fills: 1.0 is a perfect rectangle.
    pub rectangularity: f32,
    pub holes: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct Box2 {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Box2 {
    fn w(&self) -> usize {
        self.x1 - self.x0 + 1
    }
    fn h(&self) -> usize {
        self.y1 - self.y0 + 1
    }
}

/// A hole found in the silhouette.
#[derive(Debug, Clone, Copy)]
struct Hole {
    cx: f64,
    cy: f64,
    area: usize,
}

fn luma(r: u8, g: u8, b: u8) -> u32 {
    (299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000
}

/// Work out which pixels are the part.
///
/// The studio renders its own views on a transparent background, so alpha is the
/// truest signal. A photograph has no alpha at all, so when the picture is fully
/// opaque the shape is taken to be whatever differs from the background colour
/// around the edges of the frame.
fn silhouette(rgba: &[u8], w: usize, h: usize) -> Vec<bool> {
    let n = w * h;
    let mut mask = vec![false; n];

    let transparent = (0..n).filter(|i| rgba[i * 4 + 3] < 128).count();
    if transparent * 100 > n {
        for i in 0..n {
            mask[i] = rgba[i * 4 + 3] >= 128;
        }
        return mask;
    }

    // Average the four corners: that is the backdrop.
    let corners = [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)];
    let mut sum = (0u32, 0u32, 0u32);
    for (x, y) in corners {
        let i = (y * w + x) * 4;
        sum.0 += rgba[i] as u32;
        sum.1 += rgba[i + 1] as u32;
        sum.2 += rgba[i + 2] as u32;
    }
    let bg = luma((sum.0 / 4) as u8, (sum.1 / 4) as u8, (sum.2 / 4) as u8);
    for i in 0..n {
        let l = luma(rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]);
        mask[i] = l.abs_diff(bg) > 26;
    }
    mask
}

fn bounding_box(mask: &[bool], w: usize, h: usize) -> Option<Box2> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if mask[y * w + x] {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (x0 <= x1 && y0 <= y1).then_some(Box2 { x0, y0, x1, y1 })
}

/// Enclosed non-part pixels are holes. Everything reachable from the frame's
/// edge is outside, so this is a flood fill from the border.
fn find_holes(mask: &[bool], w: usize, h: usize) -> Vec<Hole> {
    let n = w * h;
    let mut outside = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    for x in 0..w {
        for y in [0, h - 1] {
            let i = y * w + x;
            if !mask[i] && !outside[i] {
                outside[i] = true;
                stack.push(i);
            }
        }
    }
    for y in 0..h {
        for x in [0, w - 1] {
            let i = y * w + x;
            if !mask[i] && !outside[i] {
                outside[i] = true;
                stack.push(i);
            }
        }
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        let push = |nx: usize, ny: usize, outside: &mut Vec<bool>, stack: &mut Vec<usize>| {
            let j = ny * w + nx;
            if !mask[j] && !outside[j] {
                outside[j] = true;
                stack.push(j);
            }
        };
        if x > 0 {
            push(x - 1, y, &mut outside, &mut stack);
        }
        if y > 0 {
            push(x, y - 1, &mut outside, &mut stack);
        }
        if x + 1 < w {
            push(x + 1, y, &mut outside, &mut stack);
        }
        if y + 1 < h {
            push(x, y + 1, &mut outside, &mut stack);
        }
    }

    // Whatever is neither part nor outside is enclosed - a hole. Label the
    // components so two holes are two holes, not one big blob.
    let mut seen = vec![false; n];
    let mut holes = Vec::new();
    for start in 0..n {
        if mask[start] || outside[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![start];
        let (mut area, mut sx, mut sy) = (0usize, 0f64, 0f64);
        while let Some(i) = stack.pop() {
            area += 1;
            sx += (i % w) as f64;
            sy += (i / w) as f64;
            let (x, y) = (i % w, i / w);
            let push = |nx: usize, ny: usize, seen: &mut Vec<bool>, stack: &mut Vec<usize>| {
                let j = ny * w + nx;
                if !mask[j] && !outside[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(x - 1, y, &mut seen, &mut stack);
            }
            if y > 0 {
                push(x, y - 1, &mut seen, &mut stack);
            }
            if x + 1 < w {
                push(x + 1, y, &mut seen, &mut stack);
            }
            if y + 1 < h {
                push(x, y + 1, &mut seen, &mut stack);
            }
        }
        // A couple of stray pixels is anti-aliasing, not a hole.
        if area >= 9 {
            holes.push(Hole {
                cx: sx / area as f64,
                cy: sy / area as f64,
                area,
            });
        }
    }
    // Biggest first, so the reading is stable whatever the flood order was.
    holes.sort_by(|a, b| b.area.cmp(&a.area));
    holes
}

/// Read a picture as a set of CAD operations.
///
/// `rgba` is `width * height * 4` bytes, the way a browser canvas hands it over.
pub fn infer(rgba: &[u8], width: usize, height: usize) -> Result<Inference, String> {
    if width == 0 || height == 0 {
        return Err("the picture has no pixels".into());
    }
    if rgba.len() < width * height * 4 {
        return Err(format!(
            "expected {} bytes of RGBA for {width}x{height}, got {}",
            width * height * 4,
            rgba.len()
        ));
    }

    let mask = silhouette(rgba, width, height);
    let area = mask.iter().filter(|m| **m).count();
    if area == 0 {
        return Err(
            "no shape stands out from the background: render the part on a plain \
             background, or send it with a transparent one"
                .into(),
        );
    }

    let bb = bounding_box(&mask, width, height).ok_or("could not find the shape's outline")?;
    let rectangularity = area as f32 / (bb.w() * bb.h()) as f32;
    let holes = find_holes(&mask, width, height);

    // Scale the picture to millimetres: the longest side of the shape becomes
    // 100 mm. A single view cannot give absolute size, so the reading is
    // proportional, and the steps say so.
    let longest = bb.w().max(bb.h()) as f64;
    let scale = 100.0 / longest;
    let plate_w = (bb.w() as f64 * scale * 2.0).round() / 2.0;
    let plate_h = (bb.h() as f64 * scale * 2.0).round() / 2.0;
    // A plate is thinner than it is wide; the depth is a stated assumption.
    let thickness = (plate_w.min(plate_h) * 0.25 * 2.0).round() / 2.0;

    let half_w = plate_w / 2.0;
    let half_h = plate_h / 2.0;
    let mut ops: Vec<Op> = Vec::new();
    ops.push(Op::Extrude {
        profile: Profile::Polygon {
            points: vec![
                [-half_w, -half_h, 0.0],
                [half_w, -half_h, 0.0],
                [half_w, half_h, 0.0],
                [-half_w, half_h, 0.0],
            ],
        },
        distance: thickness,
        direction: [0.0, 0.0, 1.0],
    });

    // Hole centres are relative to the middle of the shape, in millimetres.
    let cx_px = (bb.x0 + bb.x1) as f64 / 2.0;
    let cy_px = (bb.y0 + bb.y1) as f64 / 2.0;
    let positions: Vec<[f64; 3]> = holes
        .iter()
        .map(|hole| {
            let x = (hole.cx - cx_px) * scale;
            // The picture's y grows downwards; the model's grows upwards.
            let y = -(hole.cy - cy_px) * scale;
            [(x * 2.0).round() / 2.0, (y * 2.0).round() / 2.0, 0.0]
        })
        .collect();

    let diameter_of = |hole: &Hole| -> f64 {
        let d_px = 2.0 * (hole.area as f64 / std::f64::consts::PI).sqrt();
        ((d_px * scale * 2.0).round() / 2.0).max(1.0)
    };

    let mut steps: Vec<StepLine> = Vec::new();
    steps.push(StepLine {
        op: "source".into(),
        detail: format!(
            "picture {width}x{height}, shape {}x{} px, {} hole(s)",
            bb.w(),
            bb.h(),
            holes.len()
        ),
    });
    steps.push(StepLine {
        op: "Sketch".into(),
        detail: format!("rectangle {plate_w} x {plate_h} mm, from the silhouette"),
    });
    steps.push(StepLine {
        op: "Extrude".into(),
        detail: format!("distance {thickness} mm (assumed: one view cannot show depth)"),
    });

    match holes.len() {
        0 => {}
        1 => {
            let d = diameter_of(&holes[0]);
            ops.push(Op::Hole {
                diameter: d,
                through: true,
                depth: 0.0,
                at: positions[0],
                axis: [0.0, 0.0, 1.0],
            });
            steps.push(StepLine {
                op: "Hole".into(),
                detail: format!("d {d} mm, through all, at ({:.1}, {:.1})", positions[0][0], positions[0][1]),
            });
        }
        n => {
            // Several holes of about the same size read as one pattern.
            let d = diameter_of(&holes[0]);
            let similar = holes.iter().all(|h| {
                let other = diameter_of(h);
                (other - d).abs() <= (d * 0.25).max(0.5)
            });
            if similar {
                ops.push(Op::HolePattern {
                    diameter: d,
                    through: true,
                    depth: 0.0,
                    positions: positions.clone(),
                    axis: [0.0, 0.0, 1.0],
                });
                steps.push(StepLine {
                    op: "HolePattern".into(),
                    detail: format!("d {d} mm, through all, {n} positions"),
                });
            } else {
                for (i, hole) in holes.iter().enumerate() {
                    let hd = diameter_of(hole);
                    ops.push(Op::Hole {
                        diameter: hd,
                        through: true,
                        depth: 0.0,
                        at: positions[i],
                        axis: [0.0, 0.0, 1.0],
                    });
                }
                steps.push(StepLine {
                    op: "Hole".into(),
                    detail: format!("{n} holes of differing size, through all"),
                });
            }
        }
    }

    // Confidence is what the picture can support: a clean rectangle with a
    // plausible hole count is a strong read; a ragged blob is not.
    let shape_score = ((rectangularity - 0.45) / 0.55).clamp(0.0, 1.0);
    let confidence = (0.15 + 0.65 * shape_score).clamp(0.0, 0.95);

    let mut notes = vec![
        "Geometric baseline: this measures the silhouette and writes the operations that 
         would produce it. It is not a trained model."
            .to_string(),
        format!(
            "The shape fills {:.0}% of its bounding box, which is how rectangular it reads.",
            rectangularity * 100.0
        ),
        "Size is proportional: the longest side is taken as 100 mm, because one view cannot 
         give absolute dimensions."
            .to_string(),
        "Depth is assumed, not measured: turn on 'Capture views' and send more than one view 
         to do better."
            .to_string(),
    ];
    if rectangularity < 0.5 {
        notes.push(
            "The outline is far from rectangular, so a rectangle plus holes may be the wrong 
             reading for this part."
                .into(),
        );
    }

    Ok(Inference {
        source: "baseline".into(),
        width,
        height,
        steps,
        program: Program {
            version: crate::ops::VERSION,
            units: "mm".into(),
            ops,
        },
        confidence,
        rectangularity,
        holes: holes.len(),
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture with a shape drawn on a transparent background.
    fn canvas(w: usize, h: usize) -> Vec<u8> {
        vec![0u8; w * h * 4]
    }

    fn rect(rgba: &mut [u8], w: usize, _h: usize, x0: usize, y0: usize, x1: usize, y1: usize, rgb: [u8; 3]) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = (y * w + x) * 4;
                rgba[i] = rgb[0];
                rgba[i + 1] = rgb[1];
                rgba[i + 2] = rgb[2];
                rgba[i + 3] = 255;
            }
        }
    }

    #[test]
    fn reads_a_plain_plate_as_a_rectangle() {
        let (w, h) = (200usize, 100usize);
        let mut px = canvas(w, h);
        rect(&mut px, w, h, 20, 20, 179, 79, [200, 200, 200]);
        let inf = infer(&px, w, h).unwrap();
        assert_eq!(inf.holes, 0);
        assert_eq!(inf.program.ops.len(), 1);
        assert!(matches!(inf.program.ops[0], Op::Extrude { .. }));
        assert!(inf.confidence > 0.6, "confidence {}", inf.confidence);
        assert!(inf.rectangularity > 0.9);
    }

    #[test]
    fn two_holes_of_a_size_read_as_a_pattern() {
        let (w, h) = (200usize, 100usize);
        let mut px = canvas(w, h);
        rect(&mut px, w, h, 10, 10, 189, 89, [200, 200, 200]);
        // Punch two 8x8 holes through the shape.
        for (hx, hy) in [(60usize, 50usize), (140usize, 50usize)] {
            for y in hy - 4..=hy + 4 {
                for x in hx - 4..=hx + 4 {
                    let i = (y * w + x) * 4;
                    px[i + 3] = 0;
                }
            }
        }
        let inf = infer(&px, w, h).unwrap();
        assert_eq!(inf.holes, 2, "holes: {}", inf.holes);
        assert!(matches!(inf.program.ops[1], Op::HolePattern { .. }));
        let Op::HolePattern { positions, .. } = &inf.program.ops[1] else {
            unreachable!()
        };
        assert_eq!(positions.len(), 2);
        // The left hole must land left of the right one, in model coordinates.
        assert!(positions[0][0] < positions[1][0]);
        assert!(inf.steps.iter().any(|s| s.op == "HolePattern"));
    }

    #[test]
    fn one_hole_reads_as_one_hole() {
        let (w, h) = (200usize, 100usize);
        let mut px = canvas(w, h);
        rect(&mut px, w, h, 10, 10, 189, 89, [180, 190, 200]);
        for y in 46..=54 {
            for x in 96..=104 {
                let i = (y * w + x) * 4;
                px[i + 3] = 0;
            }
        }
        let inf = infer(&px, w, h).unwrap();
        assert_eq!(inf.holes, 1);
        assert!(matches!(inf.program.ops[1], Op::Hole { .. }));
    }

    #[test]
    fn an_opaque_photo_uses_the_background_instead() {
        let (w, h) = (120usize, 120usize);
        // A white frame with a dark shape in it: no alpha at all.
        let mut px = vec![255u8; w * h * 4];
        rect(&mut px, w, h, 10, 10, 109, 109, [40, 40, 40]);
        let inf = infer(&px, w, h).unwrap();
        assert!(inf.rectangularity > 0.9, "{}", inf.rectangularity);
    }

    #[test]
    fn an_empty_picture_is_reported_not_faked() {
        let px = canvas(64, 64);
        let err = infer(&px, 64, 64).unwrap_err();
        assert!(err.contains("no shape"), "{err}");
    }

    #[test]
    fn a_short_buffer_is_rejected() {
        assert!(infer(&[0u8; 16], 64, 64).is_err());
        assert!(infer(&[], 0, 0).is_err());
    }

    #[test]
    fn the_program_it_writes_actually_validates() {
        let (w, h) = (200usize, 100usize);
        let mut px = canvas(w, h);
        rect(&mut px, w, h, 10, 10, 189, 89, [200, 200, 200]);
        let inf = infer(&px, w, h).unwrap();
        assert!(inf.program.validate().is_ok(), "{:?}", inf.program.validate());
        // ...and the native engine can build it into real geometry.
        let solid = crate::solid::evaluate(&inf.program);
        assert!(solid.is_ok(), "rebuild failed: {:?}", solid.err());
        assert!(!solid.unwrap().solid.triangulate().is_empty());
    }
}
