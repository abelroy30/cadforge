//! ISO 10303-21 ("STEP") reader.
//!
//! A CAD part is not a picture: it is a boundary representation plus, in the
//! native formats, the sequence of operations that built it. The native
//! SolidWorks history is not in the exchanged file, but the B-rep *is*, and the
//! B-rep carries the fingerprints of those operations - a pair of parallel
//! planar faces is an extrude, a cylindrical face is a hole, a toroidal face is
//! a fillet. This module reads the file and reads those fingerprints back out.
//!
//! Nothing here needs a CAD kernel: it is a text format and a graph.

use std::collections::BTreeMap;

/* ------------------------------------------------------------------ values */

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Ref(u64),
    Str(String),
    Enum(String),
    Num(f64),
    List(Vec<Value>),
    Typed(String, Vec<Value>),
    /// `$` - omitted.
    Null,
    /// `*` - derived, computed from the rest of the schema.
    Derived,
}

impl Value {
    pub fn as_ref(&self) -> Option<u64> {
        match self {
            Value::Ref(id) => Some(*id),
            _ => None,
        }
    }

    pub fn as_num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    #[allow(dead_code)] // part of the Value accessor set, used by tests today
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_enum(&self) -> Option<&str> {
        match self {
            Value::Enum(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_vec3(&self) -> Option<[f64; 3]> {
        let items = self.as_list()?;
        if items.len() < 3 {
            return None;
        }
        Some([items[0].as_num()?, items[1].as_num()?, items[2].as_num()?])
    }
}

#[derive(Debug, Clone)]
pub struct Entity {
    #[allow(dead_code)] // kept so an entity knows where it came from when debugging
    pub id: u64,
    pub name: String,
    pub args: Vec<Value>,
}

impl Entity {
    /// Argument `n`, skipping the leading name string STEP puts on every entity.
    pub fn arg(&self, n: usize) -> Option<&Value> {
        self.args.get(n + 1)
    }
}

/* ------------------------------------------------------------- tokenizer */

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Hash(u64),
    Word(String),
    Str(String),
    Enum(String),
    Num(f64),
    LParen,
    RParen,
    Comma,
    Semi,
    Eq,
    Dollar,
    Star,
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '!' || c == '?'
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let cs: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // comments
        if c == '/' && cs.get(i + 1) == Some(&'*') {
            i += 2;
            while i < cs.len() && !(cs[i] == '*' && cs.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        match c {
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            ';' => {
                out.push(Tok::Semi);
                i += 1;
            }
            '=' => {
                out.push(Tok::Eq);
                i += 1;
            }
            '$' => {
                out.push(Tok::Dollar);
                i += 1;
            }
            '*' => {
                out.push(Tok::Star);
                i += 1;
            }
            '#' => {
                i += 1;
                let start = i;
                while i < cs.len() && cs[i].is_ascii_digit() {
                    i += 1;
                }
                let digits: String = cs[start..i].iter().collect();
                let id = digits.parse().map_err(|_| "bad entity id".to_string())?;
                out.push(Tok::Hash(id));
            }
            '\'' => {
                // string, with '' as the escape for a quote
                i += 1;
                let mut s = String::new();
                while i < cs.len() {
                    if cs[i] == '\'' {
                        if cs.get(i + 1) == Some(&'\'') {
                            s.push('\'');
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    s.push(cs[i]);
                    i += 1;
                }
                out.push(Tok::Str(s));
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                while i < cs.len() && cs[i] != '"' {
                    s.push(cs[i]);
                    i += 1;
                }
                i += 1;
                out.push(Tok::Str(s));
            }
            '.' if !cs.get(i + 1).map(|c| c.is_ascii_digit()).unwrap_or(false) => {
                // enumeration, .T. / .UNSPECIFIED. etc.
                i += 1;
                let mut s = String::new();
                while i < cs.len() && cs[i] != '.' {
                    s.push(cs[i]);
                    i += 1;
                }
                i += 1;
                out.push(Tok::Enum(s));
            }
            _ if c.is_ascii_digit() || c == '.' || c == '-' || c == '+' => {
                let start = i;
                if c == '-' || c == '+' {
                    i += 1;
                }
                while i < cs.len() && cs[i].is_ascii_digit() {
                    i += 1;
                }
                if i < cs.len() && cs[i] == '.' {
                    i += 1;
                    while i < cs.len() && cs[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if i < cs.len() && (cs[i] == 'e' || cs[i] == 'E') {
                    let save = i;
                    i += 1;
                    if i < cs.len() && (cs[i] == '-' || cs[i] == '+') {
                        i += 1;
                    }
                    if i < cs.len() && cs[i].is_ascii_digit() {
                        while i < cs.len() && cs[i].is_ascii_digit() {
                            i += 1;
                        }
                    } else {
                        i = save;
                    }
                }
                let text: String = cs[start..i].iter().collect();
                let n: f64 = text.parse().map_err(|_| format!("bad number {text}"))?;
                out.push(Tok::Num(n));
            }
            _ if is_word_char(c) => {
                let start = i;
                while i < cs.len() {
                    let ch = cs[i];
                    if is_word_char(ch) {
                        i += 1;
                    } else if ch == '-'
                        && cs
                            .get(i + 1)
                            .map(|n| n.is_ascii_alphabetic())
                            .unwrap_or(false)
                    {
                        // END-ISO-10303-21
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push(Tok::Word(cs[start..i].iter().collect()));
            }
            _ => {
                i += 1;
            }
        }
    }
    Ok(out)
}

/* ---------------------------------------------------------------- parser */

#[derive(Debug, Clone, Default)]
pub struct StepFile {
    pub entities: BTreeMap<u64, Entity>,
}

impl StepFile {
    pub fn parse(src: &str) -> Result<StepFile, String> {
        let toks = tokenize(src)?;
        let mut i = 0usize;

        // The header section is metadata; the instances we want follow DATA;.
        while i < toks.len() {
            if toks[i] == Tok::Word("DATA".into()) {
                i += 1;
                break;
            }
            i += 1;
        }
        while i < toks.len() && toks[i] != Tok::Semi {
            i += 1;
        }
        i += 1;

        let mut entities = BTreeMap::new();
        while i < toks.len() {
            let id = match &toks[i] {
                Tok::Hash(id) => *id,
                _ => {
                    i += 1;
                    continue;
                }
            };
            i += 1;
            if toks.get(i) != Some(&Tok::Eq) {
                return Err(format!("expected '=' after #{id}"));
            }
            i += 1;
            let (entity, next) = parse_instance(id, &toks, i)?;
            i = next;
            entities.insert(id, entity);
        }
        Ok(StepFile { entities })
    }

    pub fn get(&self, id: u64) -> Option<&Entity> {
        self.entities.get(&id)
    }

    /// Follow a reference argument to its entity.
    pub fn deref(&self, value: Option<&Value>) -> Option<&Entity> {
        self.get(value?.as_ref()?)
    }

    pub fn named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Entity> {
        self.entities.values().filter(move |e| e.name == name)
    }

    pub fn count(&self, name: &str) -> usize {
        self.named(name).count()
    }
}

fn parse_instance(id: u64, toks: &[Tok], mut i: usize) -> Result<(Entity, usize), String> {
    // complex instance: #1 = (A(...)B(...));  - parts are written back to back,
    // with no commas between them.
    if toks.get(i) == Some(&Tok::LParen) {
        i += 1;
        let mut parts = Vec::new();
        while i < toks.len() && toks[i] != Tok::RParen {
            if toks[i] == Tok::Comma {
                i += 1;
                continue;
            }
            let (value, next) = parse_value(toks, i)?;
            parts.push(value);
            i = next;
        }
        i += 1; // consume ')'
        if toks.get(i) == Some(&Tok::Semi) {
            i += 1;
        }
        return Ok((
            Entity {
                id,
                name: "COMPLEX".into(),
                args: parts,
            },
            i,
        ));
    }
    let name = match &toks[i] {
        Tok::Word(w) => w.clone(),
        other => return Err(format!("expected entity name at #{id}, found {other:?}")),
    };
    i += 1;
    let (args, next) = parse_args(toks, i)?;
    i = next;
    if toks.get(i) == Some(&Tok::Semi) {
        i += 1;
    }
    Ok((Entity { id, name, args }, i))
}

fn parse_args(toks: &[Tok], mut i: usize) -> Result<(Vec<Value>, usize), String> {
    if toks.get(i) != Some(&Tok::LParen) {
        return Err("expected '(' ".into());
    }
    i += 1;
    let mut args = Vec::new();
    if toks.get(i) == Some(&Tok::RParen) {
        return Ok((args, i + 1));
    }
    loop {
        let (value, next) = parse_value(toks, i)?;
        args.push(value);
        i = next;
        match toks.get(i) {
            Some(Tok::Comma) => i += 1,
            Some(Tok::RParen) => {
                i += 1;
                break;
            }
            other => {
                return Err(format!(
                    "expected ',' or ')' in argument list, found {other:?}"
                ))
            }
        }
    }
    Ok((args, i))
}

fn parse_value(toks: &[Tok], i: usize) -> Result<(Value, usize), String> {
    match toks.get(i) {
        Some(Tok::Num(n)) => Ok((Value::Num(*n), i + 1)),
        Some(Tok::Str(s)) => Ok((Value::Str(s.clone()), i + 1)),
        Some(Tok::Enum(e)) => Ok((Value::Enum(e.clone()), i + 1)),
        Some(Tok::Hash(id)) => Ok((Value::Ref(*id), i + 1)),
        Some(Tok::Dollar) => Ok((Value::Null, i + 1)),
        Some(Tok::Star) => Ok((Value::Derived, i + 1)),
        Some(Tok::LParen) => {
            let (args, next) = parse_args(toks, i)?;
            Ok((Value::List(args), next))
        }
        Some(Tok::Word(w)) => {
            if toks.get(i + 1) == Some(&Tok::LParen) {
                let (args, next) = parse_args(toks, i + 1)?;
                Ok((Value::Typed(w.clone(), args), next))
            } else {
                Ok((Value::Typed(w.clone(), Vec::new()), i + 1))
            }
        }
        other => Err(format!("unexpected token {other:?}")),
    }
}

/* -------------------------------------------------------------- geometry */

use crate::vec3::{add, cross, dot, normalize, perpendicular, scale, sub};

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // ref_dir is unused until we reconstruct sketches
pub struct Placement {
    pub origin: [f64; 3],
    pub axis: [f64; 3],
    pub ref_dir: [f64; 3],
}

impl StepFile {
    pub fn point_of(&self, value: Option<&Value>) -> Option<[f64; 3]> {
        let e = self.deref(value)?;
        if e.name != "CARTESIAN_POINT" {
            return None;
        }
        e.arg(0)?.as_vec3()
    }

    pub fn direction_of(&self, value: Option<&Value>) -> Option<[f64; 3]> {
        let e = self.deref(value)?;
        if e.name != "DIRECTION" {
            return None;
        }
        e.arg(0)?.as_vec3()
    }

    pub fn placement_of(&self, value: Option<&Value>) -> Option<Placement> {
        let e = self.deref(value)?;
        if e.name != "AXIS2_PLACEMENT_3D" && e.name != "AXIS2_PLACEMENT_2D" {
            return None;
        }
        let origin = self.point_of(e.arg(0))?;
        Some(Placement {
            origin,
            axis: self.direction_of(e.arg(1)).unwrap_or([0.0, 0.0, 1.0]),
            ref_dir: self.direction_of(e.arg(2)).unwrap_or([1.0, 0.0, 0.0]),
        })
    }

    /// Ordered points around one loop, following edge orientations.
    fn loop_points(&self, loop_ref: &Value) -> Vec<[f64; 3]> {
        let Some(lp) = self.deref(Some(loop_ref)) else {
            return Vec::new();
        };
        match lp.name.as_str() {
            "POLY_LOOP" => lp
                .arg(0)
                .and_then(|v| v.as_list())
                .map(|refs| refs.iter().filter_map(|r| self.point_of(Some(r))).collect())
                .unwrap_or_default(),
            "EDGE_LOOP" => {
                let mut pts = Vec::new();
                let Some(edges) = lp.arg(0).and_then(|v| v.as_list()) else {
                    return pts;
                };
                for oe in edges {
                    let Some(oriented) = self.deref(Some(oe)) else {
                        continue;
                    };
                    let forward = oriented
                        .arg(3)
                        .and_then(|v| v.as_enum())
                        .map(|f| f != "F")
                        .unwrap_or(true);
                    // ORIENTED_EDGE(name, edge_start, edge_end, edge_element, orientation)
                    let Some(curve) = self.deref(oriented.arg(2)) else {
                        continue;
                    };
                    let (a, b) = (curve.arg(0), curve.arg(1));
                    let pick = if forward { a } else { b };
                    if let Some(p) = self.vertex_point(pick) {
                        pts.push(p);
                    }
                }
                pts
            }
            "VERTEX_LOOP" => self.vertex_point(lp.arg(0)).into_iter().collect(),
            _ => Vec::new(),
        }
    }

    fn vertex_point(&self, value: Option<&Value>) -> Option<[f64; 3]> {
        let v = self.deref(value)?;
        if v.name != "VERTEX_POINT" {
            return None;
        }
        self.point_of(v.arg(0))
    }

    /// Fan-triangulate each planar face from its outer loop's centroid. Good
    /// enough to see the part; a kernel would do this properly.
    fn triangulate(&self, cap: usize, out: &mut Vec<f32>) {
        self.triangulate_planes(cap, out);
        self.triangulate_cylinders(cap, out);
    }

    fn triangulate_planes(&self, cap: usize, out: &mut Vec<f32>) {
        for face in self.named("ADVANCED_FACE") {
            if out.len() / 9 >= cap {
                return;
            }
            let Some(bounds) = face.arg(0).and_then(|v| v.as_list()) else {
                continue;
            };
            // prefer the outer bound
            let mut chosen: Option<Vec<[f64; 3]>> = None;
            for b in bounds {
                let Some(be) = self.deref(Some(b)) else {
                    continue;
                };
                let pts = be.arg(0).map(|l| self.loop_points(l)).unwrap_or_default();
                if pts.len() < 3 {
                    continue;
                }
                if be.name == "FACE_OUTER_BOUND" {
                    chosen = Some(pts);
                    break;
                }
                if chosen.is_none() {
                    chosen = Some(pts);
                }
            }
            let Some(pts) = chosen else { continue };
            let mut c = [0.0f64; 3];
            for p in &pts {
                for k in 0..3 {
                    c[k] += p[k];
                }
            }
            for k in 0..3 {
                c[k] /= pts.len() as f64;
            }
            for w in pts.windows(2) {
                if out.len() / 9 >= cap {
                    return;
                }
                for v in [c, w[0], w[1]] {
                    out.push(v[0] as f32);
                    out.push(v[1] as f32);
                    out.push(v[2] as f32);
                }
            }
        }
    }

    /// Cylindrical faces (holes, bosses) previewed as a sampled strip between
    /// the circles that lie on them.
    fn triangulate_cylinders(&self, cap: usize, out: &mut Vec<f32>) {
        let mut circles: Vec<([f64; 3], [f64; 3], f64)> = Vec::new();
        for e in self.named("CIRCLE") {
            let radius = e.arg(1).and_then(|v| v.as_num()).unwrap_or(0.0);
            if let Some(pl) = self.placement_of(e.arg(0)) {
                circles.push((pl.origin, pl.axis, radius));
            }
        }
        for e in self.named("CYLINDRICAL_SURFACE") {
            if out.len() / 9 >= cap {
                return;
            }
            let Some(pl) = self.placement_of(e.arg(0)) else {
                continue;
            };
            let radius = e.arg(1).and_then(|v| v.as_num()).unwrap_or(0.0);
            if radius <= 1e-9 {
                continue;
            }
            let axis = normalize(pl.axis);
            let mut ts: Vec<f64> = Vec::new();
            for (c, a, r) in &circles {
                if axis_key(*a) != axis_key(pl.axis) {
                    continue;
                }
                // A bounding circle of this face: same radius, and its centre
                // sits on the cylinder's axis (so its distance to the axis
                // line is zero, not the radius).
                if (r - radius).abs() > 1e-3 {
                    continue;
                }
                let rel = sub(*c, pl.origin);
                let along = dot(rel, axis);
                let radial = sub(rel, [axis[0] * along, axis[1] * along, axis[2] * along]);
                if dot(radial, radial).sqrt() < 1e-3 {
                    ts.push(along);
                }
            }
            if ts.len() < 2 {
                continue;
            }
            let mut t0 = f64::MAX;
            let mut t1 = f64::MIN;
            for t in ts {
                t0 = t0.min(t);
                t1 = t1.max(t);
            }
            if (t1 - t0).abs() < 1e-9 {
                continue;
            }
            let seed = if axis[0].abs() < 0.9 {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 1.0, 0.0]
            };
            let u = normalize(cross(axis, seed));
            let v = cross(axis, u);
            let origin = pl.origin;
            let point = move |t: f64, ang: f64| -> [f32; 3] {
                let c = ang.cos() * radius;
                let s = ang.sin() * radius;
                [
                    (origin[0] + axis[0] * t + u[0] * c + v[0] * s) as f32,
                    (origin[1] + axis[1] * t + u[1] * c + v[1] * s) as f32,
                    (origin[2] + axis[2] * t + u[2] * c + v[2] * s) as f32,
                ]
            };
            let seg = 32usize;
            for i in 0..seg {
                let a0 = (i as f64) / (seg as f64) * std::f64::consts::TAU;
                let a1 = ((i + 1) as f64) / (seg as f64) * std::f64::consts::TAU;
                let p00 = point(t0, a0);
                let p01 = point(t1, a0);
                let p10 = point(t0, a1);
                let p11 = point(t1, a1);
                out.extend_from_slice(&p00);
                out.extend_from_slice(&p10);
                out.extend_from_slice(&p11);
                out.extend_from_slice(&p00);
                out.extend_from_slice(&p11);
                out.extend_from_slice(&p01);
            }
        }
    }
}

/* -------------------------------------------------------------- analysis */

use serde::{Deserialize, Serialize};

/// One step of the history reconstructed from the B-rep.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepLine {
    pub op: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub solids: usize,
    pub shells: usize,
    pub faces: usize,
    pub planar_faces: usize,
    pub cylindrical_faces: usize,
    pub toroidal_faces: usize,
    pub vertices: usize,
    pub edges: usize,
    pub points: usize,
    pub unit: String,
    pub steps: Vec<StepLine>,
}

pub struct Analysis {
    pub summary: Summary,
    pub triangles: Vec<f32>,
    pub points: Vec<f32>,
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn axis_key(v: [f64; 3]) -> String {
    let mut v = v;
    let mag = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if mag > 1e-9 {
        for k in 0..3 {
            v[k] /= mag;
        }
    }
    // Opposite normals describe the same slab; canonicalise the sign so a pair
    // of parallel faces always lands in one group.
    let first = v.iter().find(|c| c.abs() > 1e-6).copied().unwrap_or(1.0);
    if first < 0.0 {
        // `0.0 - x`, not `-x`: negating a zero yields -0.0, which formats as
        // "-0.00" and would give a parallel plane a different group key.
        for k in 0..3 {
            v[k] = 0.0 - v[k];
        }
    }
    format!("{:.2},{:.2},{:.2}", v[0], v[1], v[2])
}

fn parse_axis_key(key: &str) -> [f64; 3] {
    let mut out = [0.0f64; 3];
    for (k, part) in key.split(',').take(3).enumerate() {
        out[k] = part.parse().unwrap_or(0.0);
    }
    out
}

pub fn analyze(src: &str) -> Result<Analysis, String> {
    Ok(analyze_file(&StepFile::parse(src)?))
}

pub fn analyze_file(file: &StepFile) -> Analysis {
    let mut summary = Summary {
        solids: file.count("MANIFOLD_SOLID_BREP") + file.count("BREP_WITH_VOIDS"),
        shells: file.count("CLOSED_SHELL") + file.count("OPEN_SHELL"),
        faces: file.count("ADVANCED_FACE") + file.count("FACE_SURFACE"),
        planar_faces: file.count("PLANE"),
        cylindrical_faces: file.count("CYLINDRICAL_SURFACE"),
        toroidal_faces: file.count("TOROIDAL_SURFACE"),
        vertices: file.count("VERTEX_POINT"),
        edges: file.count("EDGE_CURVE"),
        points: file.count("CARTESIAN_POINT"),
        unit: unit_label(file),
        steps: Vec::new(),
    };

    let mut triangles = Vec::new();
    file.triangulate(60_000, &mut triangles);

    let mut points = Vec::new();
    for e in file.named("CARTESIAN_POINT") {
        if points.len() / 3 >= 20_000 {
            break;
        }
        if let Some(p) = e.arg(0).and_then(|v| v.as_vec3()) {
            points.push(p[0] as f32);
            points.push(p[1] as f32);
            points.push(p[2] as f32);
        }
    }

    infer(file, &mut summary);
    Analysis {
        summary,
        triangles,
        points,
    }
}

/// True when some part of a value - however deeply nested - is `part` carrying
/// the enumeration `want`. Complex instances hide SI_UNIT and friends inside a
/// typed part rather than naming them directly.
fn value_has_typed_enum(v: &Value, part: &str, want: &str) -> bool {
    match v {
        Value::Typed(n, args) => {
            (n == part && args.iter().any(|a| a.as_enum() == Some(want)))
                || args.iter().any(|a| value_has_typed_enum(a, part, want))
        }
        Value::List(items) => items.iter().any(|a| value_has_typed_enum(a, part, want)),
        _ => false,
    }
}

fn unit_label(file: &StepFile) -> String {
    let mut milli = false;
    let mut metre = false;
    let mut inch = false;
    for e in file.entities.values() {
        if e.name == "SI_UNIT" {
            milli |= e.args.iter().any(|a| a.as_enum() == Some("MILLI"));
            metre |= e.args.iter().any(|a| a.as_enum() == Some("METRE"));
        }
        for a in &e.args {
            if let Value::Str(s) = a {
                inch |= s.to_uppercase().contains("INCH");
            }
            milli |= value_has_typed_enum(a, "SI_UNIT", "MILLI");
            metre |= value_has_typed_enum(a, "SI_UNIT", "METRE");
            inch |= value_has_typed_enum(a, "CONVERSION_BASED_UNIT", "INCH");
        }
    }
    if inch {
        "in".into()
    } else if metre && milli {
        "mm".into()
    } else if metre {
        "m".into()
    } else {
        "mm".into()
    }
}

/// Read the build steps back out of the B-rep.
fn infer(file: &StepFile, summary: &mut Summary) {
    let unit = summary.unit.clone();

    summary.steps.push(StepLine {
        op: "Import".into(),
        detail: format!(
            "{} solid{}, {} face{} - {} planar, {} cylindrical, {} toroidal - {} vertices",
            summary.solids,
            plural(summary.solids),
            summary.faces,
            plural(summary.faces),
            summary.planar_faces,
            summary.cylindrical_faces,
            summary.toroidal_faces,
            summary.vertices,
        ),
    });

    // Bounding box, for the extents the sketch would have had.
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    let mut have_box = false;
    for e in file.named("CARTESIAN_POINT") {
        if let Some(p) = e.arg(0).and_then(|v| v.as_vec3()) {
            have_box = true;
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }

    // Two parallel planes bound the travel of a profile: that is an extrude.
    let mut groups: BTreeMap<String, Vec<[f64; 3]>> = BTreeMap::new();
    for e in file.named("PLANE") {
        if let Some(pl) = file.placement_of(e.arg(0)) {
            groups.entry(axis_key(pl.axis)).or_default().push(pl.origin);
        }
    }
    let mut best: Option<([f64; 3], f64)> = None;
    for (key, origins) in &groups {
        if origins.len() < 2 {
            continue;
        }
        let n = parse_axis_key(key);
        let mut dmin = f64::MAX;
        let mut dmax = f64::MIN;
        for o in origins {
            let d = o[0] * n[0] + o[1] * n[1] + o[2] * n[2];
            dmin = dmin.min(d);
            dmax = dmax.max(d);
        }
        let depth = dmax - dmin;
        if depth > 1e-6 && best.map(|(_, d)| depth > d).unwrap_or(true) {
            best = Some((n, depth));
        }
    }
    if let Some((n, depth)) = best {
        let size = if have_box {
            format!(
                ", part is {:.2} x {:.2} x {:.2} {unit}",
                hi[0] - lo[0],
                hi[1] - lo[1],
                hi[2] - lo[2]
            )
        } else {
            String::new()
        };
        summary.steps.push(StepLine {
            op: "Extrude".into(),
            detail: format!(
                "{:.2} {unit} along ({:.2}, {:.2}, {:.2}){size}",
                depth, n[0], n[1], n[2]
            ),
        });
    }

    // A cylindrical face is a hole or a boss; repeated ones on parallel axes
    // are a pattern.
    let mut cyls: Vec<(f64, [f64; 3], [f64; 3])> = Vec::new();
    for e in file.named("CYLINDRICAL_SURFACE") {
        let radius = e.arg(1).and_then(|v| v.as_num()).unwrap_or(0.0);
        if let Some(pl) = file.placement_of(e.arg(0)) {
            cyls.push((radius, pl.origin, pl.axis));
        }
    }
    if !cyls.is_empty() {
        let mut pat: BTreeMap<String, Vec<(f64, [f64; 3], [f64; 3])>> = BTreeMap::new();
        for c in &cyls {
            pat.entry(format!("{}|{:.2}", axis_key(c.2), c.0))
                .or_default()
                .push(*c);
        }
        for group in pat.values() {
            let (radius, origin, _) = group[0];
            if group.len() >= 2 {
                summary.steps.push(StepLine {
                    op: "Hole pattern".into(),
                    detail: format!(
                        "{} holes - diameter {:.2} {unit}, same axis",
                        group.len(),
                        radius * 2.0
                    ),
                });
            } else {
                summary.steps.push(StepLine {
                    op: "Hole".into(),
                    detail: format!(
                        "diameter {:.2} {unit} through at ({:.2}, {:.2}, {:.2})",
                        radius * 2.0,
                        origin[0],
                        origin[1],
                        origin[2]
                    ),
                });
            }
        }
    }

    // A toroidal face is a round or a fillet.
    let mut tori: BTreeMap<String, usize> = BTreeMap::new();
    for e in file.named("TOROIDAL_SURFACE") {
        let minor = e.arg(2).and_then(|v| v.as_num()).unwrap_or(0.0);
        *tori.entry(format!("{:.2}", minor)).or_insert(0) += 1;
    }
    for (radius, count) in tori {
        summary.steps.push(StepLine {
            op: "Fillet".into(),
            detail: format!("R{radius} {unit} x{count}"),
        });
    }

    let circles = file.count("CIRCLE");
    if circles > 0 {
        summary.steps.push(StepLine {
            op: "Sketch".into(),
            detail: format!(
                "{circles} circular profile{} in the underlying sketches",
                plural(circles)
            ),
        });
    }

    if summary.steps.len() == 1 {
        summary.steps.push(StepLine {
            op: "Reconstruct".into(),
            detail: "no buildable features recognised - the file may be a surface model or an unsupported flavour".into(),
        });
    }
}

/// Turn the geometry into the machine-readable build program.
///
/// The same facts `infer` describes in prose, kept as numbers so a driver can
/// execute them. Where the prose says "extrude 60.00 mm along (1, 0, 0)", this
/// says `extrude` with a profile, a distance and a direction.
pub fn to_program(file: &StepFile) -> crate::ops::Program {
    use crate::ops::{Op, Profile, Program};

    let mut ops: Vec<Op> = Vec::new();

    let mut pts: Vec<[f64; 3]> = Vec::new();
    for e in file.named("CARTESIAN_POINT") {
        if let Some(p) = e.arg(0).and_then(|v| v.as_vec3()) {
            pts.push(p);
        }
    }

    // the extrusion: the parallel-plane group with the largest travel
    let mut groups: BTreeMap<String, Vec<[f64; 3]>> = BTreeMap::new();
    for e in file.named("PLANE") {
        if let Some(pl) = file.placement_of(e.arg(0)) {
            groups.entry(axis_key(pl.axis)).or_default().push(pl.origin);
        }
    }
    let mut best: Option<([f64; 3], f64, f64)> = None;
    for (key, origins) in &groups {
        if origins.len() < 2 {
            continue;
        }
        let n = parse_axis_key(key);
        let mut dmin = f64::MAX;
        let mut dmax = f64::MIN;
        for o in origins {
            let d = dot(*o, n);
            dmin = dmin.min(d);
            dmax = dmax.max(d);
        }
        let depth = dmax - dmin;
        if depth > 1e-6 && best.map(|(_, d, _)| depth > d).unwrap_or(true) {
            best = Some((n, depth, dmin));
        }
    }

    if let Some((n, depth, base)) = best {
        // The profile is the outline of everything, seen along the pull
        // direction - a convex hull, so a part with a concave footprint is
        // approximated. Documented rather than hidden.
        let u = perpendicular(n);
        let v = cross(n, u);
        let flat: Vec<(f64, f64)> = pts.iter().map(|p| (dot(*p, u), dot(*p, v))).collect();
        let hull = crate::solid::convex_hull_2d(flat);
        let points: Vec<[f64; 3]> = hull
            .iter()
            .map(|(x, y)| add(add(scale(u, *x), scale(v, *y)), scale(n, base)))
            .collect();
        if points.len() >= 3 {
            ops.push(Op::Extrude {
                profile: Profile::Polygon { points },
                distance: depth,
                direction: n,
            });
        }
    }

    // holes, grouped the way `infer` groups them
    let mut cyls: Vec<(f64, [f64; 3], [f64; 3])> = Vec::new();
    for e in file.named("CYLINDRICAL_SURFACE") {
        let radius = e.arg(1).and_then(|v| v.as_num()).unwrap_or(0.0);
        if let Some(pl) = file.placement_of(e.arg(0)) {
            cyls.push((radius, pl.origin, pl.axis));
        }
    }
    let mut pat: BTreeMap<String, Vec<(f64, [f64; 3], [f64; 3])>> = BTreeMap::new();
    for c in &cyls {
        pat.entry(format!("{}|{:.2}", axis_key(c.2), c.0))
            .or_default()
            .push(*c);
    }
    for group in pat.values() {
        let (radius, _, axis) = group[0];
        let diameter = radius * 2.0;
        if group.len() >= 2 {
            ops.push(Op::HolePattern {
                diameter,
                through: true,
                depth: 0.0,
                positions: group.iter().map(|c| c.1).collect(),
                axis,
            });
        } else {
            ops.push(Op::Hole {
                diameter,
                through: true,
                depth: 0.0,
                at: group[0].1,
                axis,
            });
        }
    }

    if let Some(radius) = file
        .named("TOROIDAL_SURFACE")
        .next()
        .and_then(|e| e.arg(2))
        .and_then(|v| v.as_num())
    {
        ops.push(Op::Fillet { radius });
    }

    Program {
        version: crate::ops::VERSION,
        units: unit_label(file),
        ops,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('a part'),'2;1');
ENDSEC;
DATA;
#1=SI_UNIT($,.MILLI.,.METRE.);
#2=CARTESIAN_POINT('',(0.,0.,0.));
#3=DIRECTION('',(0.,0.,1.));
#4=DIRECTION('',(1.,0.,0.));
#5=AXIS2_PLACEMENT_3D('',#2,#3,#4);
#6=PLANE('',#5);
#7=CARTESIAN_POINT('',(0.,0.,6.));
#8=AXIS2_PLACEMENT_3D('',#7,#3,#4);
#9=PLANE('',#8);
#10=CARTESIAN_POINT('',(3.,3.,0.));
#11=AXIS2_PLACEMENT_3D('',#10,#3,#4);
#12=CYLINDRICAL_SURFACE('',#11,3.);
#13=MANIFOLD_SOLID_BREP('body',#14);
ENDSEC;
END-ISO-10303-21;
";

    #[test]
    fn parses_instances() {
        let f = StepFile::parse(SAMPLE).unwrap();
        assert_eq!(f.entities.len(), 13);
        assert_eq!(f.get(6).unwrap().name, "PLANE");
        assert_eq!(f.get(12).unwrap().name, "CYLINDRICAL_SURFACE");
    }

    #[test]
    fn reads_enums_and_refs() {
        let f = StepFile::parse(SAMPLE).unwrap();
        let unit = f.get(1).unwrap();
        assert_eq!(
            unit.args
                .iter()
                .filter(|a| a.as_enum() == Some("MILLI"))
                .count(),
            1
        );
        assert_eq!(f.get(6).unwrap().arg(0).unwrap().as_ref(), Some(5));
    }

    #[test]
    fn reads_numbers_and_escaped_quotes() {
        let src = "DATA;\n#1=P('it''s',(-1.5E-3,2.,+3));\nENDSEC;\n";
        let f = StepFile::parse(src).unwrap();
        let e = f.get(1).unwrap();
        assert_eq!(e.args[0].as_str(), Some("it's"));
        let list = e.arg(0).unwrap().as_list().unwrap();
        assert_eq!(list[0].as_num(), Some(-0.0015));
        assert_eq!(list[1].as_num(), Some(2.0));
        assert_eq!(list[2].as_num(), Some(3.0));
    }

    #[test]
    fn reconstructs_the_build_history() {
        let a = analyze(SAMPLE).unwrap();
        assert_eq!(a.summary.planar_faces, 2);
        assert_eq!(a.summary.cylindrical_faces, 1);
        assert_eq!(a.summary.solids, 1);
        assert_eq!(a.summary.unit, "mm");
        let ops: Vec<&str> = a.summary.steps.iter().map(|s| s.op.as_str()).collect();
        assert!(ops.contains(&"Extrude"), "got {ops:?}");
        assert!(ops.contains(&"Hole"), "got {ops:?}");
        let extrude = a.summary.steps.iter().find(|s| s.op == "Extrude").unwrap();
        assert!(extrude.detail.contains("6.00"), "{}", extrude.detail);
    }

    #[test]
    fn reads_complex_instances() {
        let src = "DATA;\n#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\nENDSEC;\n";
        let f = StepFile::parse(src).unwrap();
        let e = f.get(1).unwrap();
        assert_eq!(e.name, "COMPLEX");
        assert_eq!(e.args.len(), 3);
        assert!(matches!(&e.args[1], Value::Typed(n, _) if n == "NAMED_UNIT"));
        assert!(matches!(&e.args[2], Value::Typed(n, _) if n == "SI_UNIT"));
        assert_eq!(unit_label(&f), "mm");
    }

    #[test]
    fn empty_geometry_is_reported_not_faked() {
        let src = "DATA;\n#1=PLANE('',#2);\nENDSEC;\n";
        assert!(StepFile::parse(src).is_ok());
        let a = analyze(src).unwrap();
        assert!(a.summary.steps.iter().any(|s| s.op == "Reconstruct"));
        assert!(a.triangles.is_empty());
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|e| panic!("missing fixture {name}: {e}"))
    }

    #[test]
    fn reads_the_generated_plate() {
        let a = analyze(&fixture("plate.step")).unwrap();
        let ops: Vec<(&str, &str)> = a
            .summary
            .steps
            .iter()
            .map(|s| (s.op.as_str(), s.detail.as_str()))
            .collect();
        assert_eq!(a.summary.solids, 1);
        assert_eq!(a.summary.planar_faces, 6);
        assert_eq!(a.summary.cylindrical_faces, 4);
        assert_eq!(a.summary.unit, "mm");
        assert!(ops.iter().any(|(op, _)| *op == "Extrude"), "steps: {ops:?}");
        assert!(
            ops.iter().any(|(op, _)| *op == "Hole pattern"),
            "steps: {ops:?}"
        );
        assert!(!a.triangles.is_empty(), "faces should preview as triangles");
        // 6 planar quads fan into 3 triangles each, plus a 32-segment strip per hole.
        assert_eq!(a.triangles.len() / 9, 6 * 3 + 4 * 32 * 2);
    }

    #[test]
    fn reads_the_generated_box() {
        let a = analyze(&fixture("box.step")).unwrap();
        assert_eq!(a.summary.planar_faces, 6);
        assert_eq!(a.summary.cylindrical_faces, 0);
        assert!(!a.triangles.is_empty());
    }
}

#[cfg(test)]
mod round_trip_tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|e| panic!("missing fixture {name}: {e}"))
    }

    /// The point of the whole exercise: read a part, turn it into an operation
    /// program, rebuild it, and see whether the rebuilt surfaces land on the
    /// originals. This is the loop a model has to be measured against.
    #[test]
    fn rebuilds_the_plate_from_its_own_history() {
        let src = fixture("plate.step");
        let analysis = analyze(&src).unwrap();
        let file = StepFile::parse(&src).unwrap();
        let program = to_program(&file);

        assert_eq!(program.op_names(), vec!["extrude", "hole_pattern"]);
        assert_eq!(program.units, "mm");

        let rebuilt = crate::solid::evaluate(&program).unwrap();
        assert!(
            rebuilt.warnings.is_empty(),
            "warnings: {:?}",
            rebuilt.warnings
        );
        assert_eq!(rebuilt.ops_applied, 2);

        let tris = rebuilt.solid.triangulate();
        assert!(!tris.is_empty());

        let score = crate::score::compare(&tris, &analysis.triangles).unwrap();
        assert!(
            score.chamfer < 0.2,
            "rebuilt part should sit on the original: chamfer {} max gap {}",
            score.chamfer,
            score.max_gap
        );
    }

    #[test]
    fn a_rebuild_that_loses_the_holes_scores_worse() {
        let src = fixture("plate.step");
        let analysis = analyze(&src).unwrap();
        let file = StepFile::parse(&src).unwrap();
        let mut program = to_program(&file);
        program.ops.pop(); // drop the hole pattern
        let rebuilt = crate::solid::evaluate(&program).unwrap();
        let good =
            crate::score::compare(&rebuilt.solid.triangulate(), &analysis.triangles).unwrap();

        let whole = crate::solid::evaluate(&to_program(&file)).unwrap();
        let best = crate::score::compare(&whole.solid.triangulate(), &analysis.triangles).unwrap();

        assert!(
            good.chamfer > best.chamfer,
            "dropping the holes must cost: {} vs {}",
            good.chamfer,
            best.chamfer
        );
    }

    #[test]
    fn boxes_rebuild_too() {
        let src = fixture("box.step");
        let file = StepFile::parse(&src).unwrap();
        let program = to_program(&file);
        assert_eq!(program.op_names(), vec!["extrude"]);
        let rebuilt = crate::solid::evaluate(&program).unwrap();
        let tris = rebuilt.solid.triangulate();
        assert_eq!(tris.len() / 9, 6 * 3);
    }
}
