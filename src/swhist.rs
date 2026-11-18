//! Read a feature-tree export produced inside SolidWorks.
//!
//! The real modelling history of a `.SLDPRT`/`.SLDASM` lives in a proprietary
//! binary we do not decode here. But SolidWorks will hand it over itself: the
//! companion macro in `tools/solidworks/` walks the FeatureManager and writes
//! the tree out as JSON. This module reads that JSON back.
//!
//! The result is deliberately the same [`Summary`](crate::step::Summary) the
//! STEP reader produces, so an exported history and an inferred one travel the
//! same path: build steps -> operation program -> rebuild -> score.

use crate::step::{StepLine, Summary};
use serde::Deserialize;

/// One feature as the exporter recorded it.
#[derive(Debug, Clone, Deserialize)]
pub struct Feature {
    /// Name in the FeatureManager, e.g. `Boss-Extrude1`.
    pub name: String,
    /// Feature type as SolidWorks reports it, e.g. `Extrude`.
    #[serde(rename = "type", alias = "featureType")]
    pub kind: String,
    /// Suppressed features are in the tree but did not build anything.
    #[serde(default)]
    pub suppressed: bool,
    /// Free-form parameters: depth, radius, count, and so on.
    #[serde(default)]
    pub details: serde_json::Map<String, serde_json::Value>,
}

/// A whole exported document.
#[derive(Debug, Clone, Deserialize)]
pub struct Export {
    /// Where it came from, e.g. `SolidWorks 2023`.
    #[serde(default)]
    pub source: String,
    /// The file the tree was taken from.
    #[serde(default)]
    pub document: String,
    /// Active configuration.
    #[serde(default)]
    pub configuration: String,
    /// The feature tree, in build order, as SolidWorks shows it.
    pub features: Vec<Feature>,
}

/// Parse a feature-tree export.
///
/// Fails when the bytes are not the JSON this module expects; a file that
/// parses but has no features is accepted and yields an empty history, so the
/// caller can say "this export was empty" rather than "this is corrupt".
pub fn parse(json: &str) -> Result<Export, String> {
    let export: Export = serde_json::from_str(json)
        .map_err(|e| format!("not a feature-tree export: {e}"))?;
    Ok(export)
}

/// Turn an export into the build-step summary the rest of the studio uses.
///
/// Suppressed features are kept in the list but marked, rather than dropped:
/// knowing a step exists and was switched off is part of how a part was made.
pub fn summary(export: &Export) -> Summary {
    let mut steps: Vec<StepLine> = Vec::new();

    if !export.document.is_empty() || !export.source.is_empty() {
        let mut from = export.document.clone();
        if from.is_empty() {
            from = "SolidWorks document".into();
        }
        if !export.source.is_empty() {
            from = format!("{from} ({})", export.source);
        }
        if !export.configuration.is_empty() {
            from = format!("{from}, config {}", export.configuration);
        }
        steps.push(StepLine {
            op: "source".into(),
            detail: from,
        });
    }

    for feature in &export.features {
        steps.push(StepLine {
            op: feature.kind.clone(),
            detail: describe(feature),
        });
    }

    Summary {
        unit: "feature tree".into(),
        steps,
        ..Summary::default()
    }
}

/// One line describing a feature: its name, its parameters, and whether it is
/// switched off.
fn describe(feature: &Feature) -> String {
    let mut parts: Vec<String> = vec![feature.name.clone()];
    let mut keys: Vec<&String> = feature.details.keys().collect();
    keys.sort();
    for key in keys {
        if let Some(value) = feature.details.get(key) {
            parts.push(format!("{key}: {}", render(value)));
        }
    }
    if feature.suppressed {
        parts.push("suppressed".into());
    }
    parts.join(" - ")
}

/// A parameter value as one short string.
fn render(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "-".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small but realistic export: a sketch, a boss, a hole, a fillet, and a
    /// suppressed feature, as a bracket would produce.
    const SAMPLE: &str = r#"
    {
      "source": "SolidWorks 2023",
      "document": "bracket.SLDPRT",
      "configuration": "Default",
      "features": [
        { "name": "Sketch1", "type": "Sketch", "details": { "plane": "Front" } },
        { "name": "Boss-Extrude1", "type": "Extrude", "details": { "depth": "12 mm", "direction": "Blind" } },
        { "name": "Sketch2", "type": "Sketch", "details": { "plane": "Top" } },
        { "name": "Cut-Extrude1", "type": "Cut", "details": { "depth": "Through all" } },
        { "name": "Fillet1", "type": "Fillet", "details": { "radius": "3 mm", "edges": 4 } },
        { "name": "Draft1", "type": "Draft", "suppressed": true, "details": { "angle": "2 deg" } }
      ]
    }
    "#;

    #[test]
    fn reads_the_tree_in_build_order() {
        let e = parse(SAMPLE).unwrap();
        assert_eq!(e.source, "SolidWorks 2023");
        assert_eq!(e.features.len(), 6);
        assert_eq!(e.features[1].name, "Boss-Extrude1");
        assert_eq!(e.features[1].kind, "Extrude");
    }

    #[test]
    fn turns_the_tree_into_build_steps() {
        let s = summary(&parse(SAMPLE).unwrap());
        // One source line, then one line per feature.
        assert_eq!(s.steps.len(), 7);
        assert_eq!(s.steps[0].op, "source");
        assert!(s.steps[0].detail.contains("bracket.SLDPRT"));
        assert!(s.steps[0].detail.contains("SolidWorks 2023"));
        assert!(s.steps[0].detail.contains("Default"));

        let ops: Vec<&str> = s.steps[1..].iter().map(|l| l.op.as_str()).collect();
        assert_eq!(ops, ["Sketch", "Extrude", "Sketch", "Cut", "Fillet", "Draft"]);

        let extrude = &s.steps[2];
        assert!(extrude.detail.contains("Boss-Extrude1"), "{}", extrude.detail);
        assert!(extrude.detail.contains("depth: 12 mm"), "{}", extrude.detail);
    }

    #[test]
    fn a_suppressed_feature_is_kept_and_marked() {
        let s = summary(&parse(SAMPLE).unwrap());
        let draft = s.steps.iter().find(|l| l.op == "Draft").unwrap();
        assert!(draft.detail.contains("suppressed"), "{}", draft.detail);
    }

    #[test]
    fn rejects_a_file_that_is_not_an_export() {
        assert!(parse("just some text").is_err());
        assert!(parse(r#"{ "parts": [] }"#).is_err());
    }

    #[test]
    fn an_export_with_no_features_is_accepted_but_empty() {
        let e = parse(r#"{ "document": "empty.SLDPRT", "features": [] }"#).unwrap();
        let s = summary(&e);
        assert_eq!(s.steps.len(), 1);
        assert_eq!(s.steps[0].op, "source");
    }
}
