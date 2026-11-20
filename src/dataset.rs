//! Pairing pictures with the operations that make them.
//!
//! A vision model is trained on (picture, operation program) pairs, and the
//! studio is where those pairs already exist: a SolidWorks or STEP document, the
//! views captured from it, and the steps the studio read out of it. This module
//! assembles them into one manifest, which is the only thing a trainer needs to
//! know about the studio.
//!
//! The pairing rule is deliberately simple and stated: a picture belongs to a
//! document when their names agree once the extension and any view suffix are
//! removed - `bracket-front.png` belongs to `bracket.SLDPRT`. Nothing fuzzy is
//! guessed, because a wrong pair teaches a model the wrong thing.

use crate::model::{Asset, AssetKind, Db};
use crate::step::StepLine;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The suffixes the view capture adds, which are not part of the part's name.
const VIEW_SUFFIXES: &[&str] = &[
    "front", "back", "left", "right", "top", "bottom", "iso", "sheet", "view",
];

fn without_extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// The document a picture belongs to, with any view suffix removed.
pub fn base_name(name: &str) -> String {
    let stem = without_extension(name);
    if let Some((head, tail)) = stem.rsplit_once('-') {
        if VIEW_SUFFIXES.contains(&tail.to_ascii_lowercase().as_str()) {
            return head.to_string();
        }
    }
    stem.to_string()
}

fn is_document(asset: &Asset) -> bool {
    matches!(
        asset.kind,
        AssetKind::Step
            | AssetKind::History
            | AssetKind::SwPart
            | AssetKind::SwAssembly
            | AssetKind::SwDrawing
    )
}

fn is_picture(asset: &Asset) -> bool {
    asset.lane == "image" || asset.lane == "view" || asset.kind == AssetKind::Image
}

/// Which side of the train/eval split a picture falls on.
///
/// The side is a hash of the artifact id, not its position in the list. That is
/// the difference between a split and a shuffle: the same picture lands on the
/// same side every time the manifest is built, so exporting the dataset again
/// cannot quietly move a part from train into eval and let the model be scored
/// on something it has already memorised.
fn split_of(id: &str, val_pct: u32) -> &'static str {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in id.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    if hash % 100 < val_pct {
        "val"
    } else {
        "train"
    }
}

/// One training example: a picture, and what to build to make what it shows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pair {
    /// The studio's own layout for the picture. Only meaningful for a picture
    /// the studio copied in itself.
    pub image: String,
    /// Where the picture actually is, absolute. This is the one to open: a
    /// picture indexed from a folder lives in the user's folder, not in the
    /// studio's store, and `image` above would point at nothing.
    pub image_path: String,
    pub image_name: String,
    pub image_id: String,
    /// The document this picture belongs to, if one was found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub source_id: Option<String>,
    /// The recorded or reconstructed steps, in prose.
    pub steps: Vec<StepLine>,
    /// The machine form, when the studio could derive one.
    pub program: Option<crate::ops::Program>,
    /// The operations a curriculum climbs, in order, so a trainer does not have
    /// to re-derive the sequence from the program.
    pub reference_ops: Vec<String>,
    /// How the program was obtained, so a trainer can filter by trustworthiness.
    pub target: String,
    /// `train` or `val`. Fixed by the picture's id, so a re-export keeps a part
    /// on the side it was already on.
    pub split: String,
}

/// Build the manifest for a dataset: every picture paired with its document.
///
/// Documents that no picture pairs with are reported rather than dropped, so it
/// is obvious when a dataset is missing the pictures a vision run needs.
pub fn manifest(dir: &Path, db: &Db, dataset_id: &str) -> Result<serde_json::Value, String> {
    let dataset = db
        .datasets
        .iter()
        .find(|d| d.id == dataset_id)
        .ok_or_else(|| format!("no dataset with id {dataset_id}"))?;

    let members: Vec<&Asset> = dataset
        .asset_ids
        .iter()
        .filter_map(|id| db.asset(id))
        .collect();

    let mut pairs: Vec<Pair> = Vec::new();
    let mut unpaired: Vec<String> = Vec::new();

    // 10% held out by default. The env var is here so a small dataset can push
    // the held-out share up without editing code.
    let val_pct = std::env::var("CADFORGE_VAL_PCT")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(10)
        .clamp(5, 50);

    for picture in members.iter().filter(|a| is_picture(a)) {
        let base = base_name(&picture.name);
        let source = members
            .iter()
            .find(|s| is_document(s) && base_name(&s.name) == base)
            .copied();

        let (steps, program, target, source_name, source_id) = match source {
            Some(doc) => {
                let (program, target) = target_for(dir, doc);
                let steps = doc
                    .step
                    .as_ref()
                    .map(|s| s.steps.clone())
                    .unwrap_or_default();
                (
                    steps,
                    program,
                    target,
                    Some(doc.name.clone()),
                    Some(doc.id.clone()),
                )
            }
            None => (Vec::new(), None, "unpaired".to_string(), None, None),
        };

        let image = format!("assets/{}.{}", picture.id, picture.ext);
        let image_path = picture.bytes_path(dir).to_string_lossy().to_string();
        let reference_ops: Vec<String> = match &program {
            Some(p) => p.op_names().iter().map(|s| s.to_string()).collect(),
            // A recorded export leads with its heading, which is no operation.
            None => steps.iter().skip(1).map(|l| l.op.clone()).collect(),
        };

        pairs.push(Pair {
            image,
            image_path,
            image_name: picture.name.clone(),
            image_id: picture.id.clone(),
            source: source_name,
            source_id,
            steps,
            program,
            reference_ops,
            target,
            split: split_of(&picture.id, val_pct).to_string(),
        });
    }

    for doc in members.iter().filter(|a| is_document(a)) {
        let base = base_name(&doc.name);
        let paired = members
            .iter()
            .any(|p| is_picture(p) && base_name(&p.name) == base);
        if !paired {
            unpaired.push(doc.name.clone());
        }
    }

    let usable = pairs.iter().filter(|p| p.program.is_some()).count();
    let training = pairs
        .iter()
        .filter(|p| p.program.is_some() && p.split == "train")
        .count();
    let held_out = pairs
        .iter()
        .filter(|p| p.program.is_some() && p.split == "val")
        .count();

    Ok(serde_json::json!({
        "dataset": dataset.name,
        "dataset_id": dataset.id,
        "version": 1,
        // The reward the run will be paid with travels with the data, so a
        // trainer never has to guess what it is being asked to optimise.
        "rewards": crate::reward::spec_for_manifest(db),
        "pairs": pairs,
        "counts": {
            "pictures": pairs.len(),
            "with_program": usable,
            "train": training,
            "val": held_out,
            "pictures_without_a_document": pairs.iter().filter(|p| p.source.is_none()).count(),
            "documents_without_a_picture": unpaired.len(),
        },
        "documents_without_a_picture": unpaired,
        "notes": [
            "A picture is paired with a document when their names agree once the \
             extension and any view suffix (front, top, right, iso, sheet) are removed.",
            "Only STEP documents yield a program here: their operations are reconstructed \
             from the geometry. A SolidWorks document carries its parameters, but its \
             feature tree is not decoded - export it from SolidWorks to train on it.",
            "`split` is `train` or `val`, decided by a hash of the picture's id: the same \
             picture always lands on the same side, so re-exporting cannot leak a part \
             from train into eval.",
            "`image_path` is the path to open; it is absolute and always correct. `image` \
             is the studio's own layout and means nothing for a picture it did not copy in.",
        ],
    }))
}

/// Can this dataset train anything?
///
/// A dataset whose only pairs are held out is a run that cannot start, and the
/// moment to say so is before it starts rather than in a trainer's dying words.
/// Returns what the manifest counts: pairs that carry a program, and how many of
/// those landed on the training side of the split.
pub fn trainable(dir: &Path, db: &Db, dataset_id: &str) -> Result<(usize, usize), String> {
    let m = manifest(dir, db, dataset_id)?;
    let usable = m["counts"]["with_program"].as_u64().unwrap_or(0) as usize;
    let train = m["counts"]["train"].as_u64().unwrap_or(0) as usize;
    Ok((usable, train))
}

/// The training target for a document: a program if one can honestly be derived.
fn target_for(dir: &Path, doc: &Asset) -> (Option<crate::ops::Program>, String) {
    if doc.kind != AssetKind::Step {
        return (
            None,
            if doc.kind == AssetKind::History {
                "recorded_steps".to_string()
            } else {
                "parameters_only".to_string()
            },
        );
    }
    // Wherever this document's bytes actually are. An uploaded part lives in
    // the studio's own store, but a part indexed from a folder lives in the
    // user's - reading it from the studio's layout reported every indexed part
    // as unreadable and quietly emptied the training set.
    let path = doc.bytes_path(dir);
    let Ok(bytes) = std::fs::read(&path) else {
        return (None, "unreadable".to_string());
    };
    let text = String::from_utf8_lossy(&bytes);
    match crate::step::StepFile::parse(&text) {
        Ok(file) => (
            Some(crate::step::to_program(&file)),
            "reconstructed_from_geometry".to_string(),
        ),
        Err(_) => (None, "unreadable".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AssetKind, Dataset};

    fn asset(id: &str, name: &str, kind: AssetKind, lane: &str) -> Asset {
        Asset {
            id: id.into(),
            name: name.into(),
            ext: name.rsplit('.').next().unwrap_or("").into(),
            kind,
            size: 0,
            uploaded_at: String::new(),
            lane: lane.into(),
            source_path: None,
            source_id: None,
            triangles: None,
            bbox: None,
            header: None,
            step: None,
            catalogue: None,
            actions: Vec::new(),
        }
    }

    #[test]
    fn a_view_belongs_to_the_document_it_was_rendered_from() {
        assert_eq!(base_name("bracket.SLDPRT"), "bracket");
        assert_eq!(base_name("bracket-front.png"), "bracket");
        assert_eq!(base_name("bracket-iso.png"), "bracket");
        assert_eq!(base_name("bracket-sheet.png"), "bracket");
        assert_eq!(base_name("bracket.STEP"), "bracket");
        // A dash that is not a view suffix stays part of the name.
        assert_eq!(base_name("plate-2.SLDPRT"), "plate-2");
        assert_eq!(base_name("noextension"), "noextension");
    }

    #[test]
    fn a_dataset_pairs_pictures_with_their_documents() {
        let mut db = Db::default();
        let mut doc = asset("d1", "bracket.SLDPRT", AssetKind::SwPart, "sldworks");
        doc.step = Some(crate::step::Summary::default());
        db.assets.push(doc);
        db.assets
            .push(asset("p1", "bracket-front.png", AssetKind::Image, "view"));
        db.assets
            .push(asset("p2", "bracket-iso.png", AssetKind::Image, "view"));
        db.assets
            .push(asset("p3", "lonely.png", AssetKind::Image, "image"));
        db.datasets.push(Dataset {
            id: "ds1".into(),
            name: "brackets".into(),
            asset_ids: vec!["d1".into(), "p1".into(), "p2".into(), "p3".into()],
            created_at: String::new(),
        });

        let m = manifest(Path::new("/nonexistent"), &db, "ds1").unwrap();
        assert_eq!(m["counts"]["pictures"], 3);
        assert_eq!(m["counts"]["pictures_without_a_document"], 1);
        let pairs = m["pairs"].as_array().unwrap();
        let paired = pairs.iter().filter(|p| !p["source"].is_null()).count();
        assert_eq!(paired, 2, "{pairs:?}");
        // A SolidWorks part yields no program, and the manifest says why.
        let first = pairs
            .iter()
            .find(|p| p["image_name"] == "bracket-front.png")
            .unwrap();
        assert_eq!(first["target"], "parameters_only");
        assert!(first["program"].is_null());
        assert_eq!(first["source_id"], "d1");
    }

    /// A part the studio indexed in place lives in the user's folder, not in the
    /// studio's store. Reading it from the studio's own layout made every indexed
    /// part 'unreadable', which would have quietly emptied the very training set
    /// the folder workflow exists to build.
    #[test]
    fn a_part_indexed_from_a_folder_still_yields_a_program() {
        let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/plate.step");
        let mut db = Db::default();
        let mut doc = asset("d1", "plate.step", AssetKind::Step, "step");
        doc.source_path = Some(fixture.to_string());
        doc.source_id = Some("s1".into());
        db.assets.push(doc);
        let mut pic = asset("p1", "plate-front.png", AssetKind::Image, "view");
        pic.source_path = Some(fixture.to_string());
        pic.source_id = Some("s1".into());
        db.assets.push(pic);
        db.datasets.push(Dataset {
            id: "ds1".into(),
            name: "indexed".into(),
            asset_ids: vec!["d1".into(), "p1".into()],
            created_at: String::new(),
        });

        let m = manifest(Path::new("/nonexistent"), &db, "ds1").unwrap();
        assert_eq!(
            m["counts"]["with_program"], 1,
            "an indexed part must still yield a program: {m}"
        );
        let pair = &m["pairs"][0];
        assert_eq!(pair["target"], "reconstructed_from_geometry");
        assert!(!pair["reference_ops"].as_array().unwrap().is_empty());
        assert_eq!(
            pair["image_path"], fixture,
            "the trainer must get the real path"
        );
    }

    /// A dataset nothing can be learned from has to be visible before a run is
    /// started. A part on its own has no program, and a part whose only pair
    /// landed on the held-out side is a run that cannot work.
    #[test]
    fn trainability_is_known_before_a_run_starts() {
        let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/plate.step");
        let mut db = Db::default();
        db.assets
            .push(asset("p9", "lonely.png", AssetKind::Image, "view"));
        db.datasets.push(Dataset {
            id: "ds1".into(),
            name: "pictures only".into(),
            asset_ids: vec!["p9".into()],
            created_at: String::new(),
        });
        let (usable, train) = trainable(Path::new("/nonexistent"), &db, "ds1").unwrap();
        assert_eq!((usable, train), (0, 0), "a picture alone teaches nothing");

        let mut doc = asset("d9", "plate.step", AssetKind::Step, "step");
        doc.source_path = Some(fixture.to_string());
        db.assets.push(doc);
        let mut pic = asset("p1", "plate-front.png", AssetKind::Image, "view");
        pic.source_path = Some(fixture.to_string());
        db.assets.push(pic);
        db.datasets.push(Dataset {
            id: "ds2".into(),
            name: "one part".into(),
            asset_ids: vec!["d9".into(), "p1".into()],
            created_at: String::new(),
        });
        let (usable, train) = trainable(Path::new("/nonexistent"), &db, "ds2").unwrap();
        assert_eq!(usable, 1);
        // One part can legitimately land on the held-out side, and that is
        // exactly the case the studio now refuses to start a run for.
        assert_eq!(train, if split_of("p1", 10) == "train" { 1 } else { 0 });
    }

    #[test]
    fn documents_with_no_picture_are_reported_not_dropped() {
        let mut db = Db::default();
        db.assets
            .push(asset("d1", "lonely.SLDPRT", AssetKind::SwPart, "sldworks"));
        db.datasets.push(Dataset {
            id: "ds1".into(),
            name: "x".into(),
            asset_ids: vec!["d1".into()],
            created_at: String::new(),
        });
        let m = manifest(Path::new("/nonexistent"), &db, "ds1").unwrap();
        assert_eq!(m["counts"]["documents_without_a_picture"], 1);
        assert_eq!(m["documents_without_a_picture"][0], "lonely.SLDPRT");
    }

    #[test]
    fn an_unknown_dataset_is_an_error_not_an_empty_manifest() {
        let db = Db::default();
        assert!(manifest(Path::new("/nonexistent"), &db, "nope").is_err());
    }

    #[test]
    fn the_split_follows_the_picture_not_its_position() {
        // Ten pictures and no documents: what is under test is the split itself,
        // not the pairing. The counts of train/val only cover pairs that carry a
        // program, so asserting on the pairs is the honest check here.
        let mut db = Db::default();
        let ids = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        for id in ids {
            db.assets
                .push(asset(id, &format!("{id}.png"), AssetKind::Image, "view"));
        }
        let make = |order: &[&str]| Dataset {
            id: "ds1".into(),
            name: "x".into(),
            asset_ids: order.iter().map(|s| (*s).to_string()).collect(),
            created_at: String::new(),
        };
        db.datasets.push(make(&ids));

        let sides = |m: &serde_json::Value| -> Vec<(String, String)> {
            let mut out: Vec<(String, String)> = m["pairs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p["image_id"].as_str().unwrap().to_string(),
                        p["split"].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            out.sort();
            out
        };

        let first = sides(&manifest(Path::new("/nonexistent"), &db, "ds1").unwrap());

        // Export the same dataset again with the members listed in reverse order.
        // The side a picture lands on is a hash of its id, so it must not move.
        let mut reversed: Vec<&str> = ids.to_vec();
        reversed.reverse();
        db.datasets[0] = make(&reversed);
        let second = sides(&manifest(Path::new("/nonexistent"), &db, "ds1").unwrap());

        assert_eq!(first, second, "a side must not move when the order changes");
        assert_eq!(first.len(), 10);
        assert!(
            first.iter().all(|(_, s)| s == "train" || s == "val"),
            "{first:?}"
        );

        // The counts always carry both keys, so a trainer can read them blindly.
        let m = manifest(Path::new("/nonexistent"), &db, "ds1").unwrap();
        assert!(m["counts"]["train"].is_u64() && m["counts"]["val"].is_u64());
    }

    #[test]
    fn the_split_actually_holds_pictures_back() {
        // A hash that always answered "train" would satisfy every other check in
        // this file while quietly meaning nothing was ever held out to evaluate
        // on - the failure that looks most like success. So: count the held-back
        // side over enough ids to see the rate, with wide slack so it cannot
        // flake on an unlucky draw.
        let held = (0..2000)
            .map(|i| format!("picture-{i}"))
            .filter(|id| split_of(id, 10) == "val")
            .count();
        assert!(
            (120..=300).contains(&held),
            "a 10% split held back {held} of 2000 pictures, which is not a working split"
        );
    }
}
