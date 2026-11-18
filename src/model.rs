use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Kind of artifact the user dropped into the studio.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    /// A triangulated mesh (STL). Rendered directly in the viewport.
    Mesh,
    /// A STEP/STP part. Carries the modelling history we want to learn from.
    Step,
    /// A SolidWorks part (`.SLDPRT`) - native feature tree inside.
    SwPart,
    /// A SolidWorks assembly (`.SLDASM`).
    SwAssembly,
    /// A SolidWorks drawing (`.SLDDRW`).
    SwDrawing,
    /// A feature-tree export from SolidWorks: the true modelling history,
    /// taken from inside the document rather than inferred from its geometry.
    History,
    /// An image used as conditioning input.
    Image,
    /// A video used as conditioning input.
    Video,
    /// Anything we keep but do not yet understand.
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub ext: String,
    pub kind: AssetKind,
    pub size: u64,
    pub uploaded_at: String,
    /// Triangle count, when the artifact is a mesh.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triangles: Option<usize>,
    /// Axis-aligned bounding box as [min_x, min_y, min_z, max_x, max_y, max_z].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f32; 6]>,
    /// Free-form header we pulled out of a STEP file (schema, product name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// What the B-rep says about how the part was built.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<crate::step::Summary>,
    /// What a SolidWorks document turned out to contain: its streams, and its
    /// embedded preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalogue: Option<crate::sldworks::Catalogue>,
}

impl Asset {
    /// Is this a SolidWorks document of some kind?
    pub fn is_solidworks(&self) -> bool {
        matches!(
            self.kind,
            AssetKind::SwPart | AssetKind::SwAssembly | AssetKind::SwDrawing
        )
    }
}

/// A base model the user points the studio at, or a model they define themselves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseModel {
    pub id: String,
    pub name: String,
    /// `base` = an existing pretrained model; `custom` = a user-defined architecture.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<u64>,
    #[serde(default)]
    pub notes: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dataset {
    pub id: String,
    pub name: String,
    pub asset_ids: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub at: String,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub name: String,
    pub base_model_id: String,
    pub dataset_id: String,
    pub status: RunStatus,
    pub stage: String,
    pub progress: f32,
    pub log: Vec<LogLine>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// Checkpoints or reports the run produced.
    #[serde(default)]
    pub outputs: Vec<String>,
}

/// Everything the studio knows, persisted as one JSON document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Db {
    #[serde(default)]
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub models: Vec<BaseModel>,
    #[serde(default)]
    pub datasets: Vec<Dataset>,
    #[serde(default)]
    pub runs: Vec<Run>,
}

impl Db {
    pub fn load(root: &Path) -> Db {
        let path = db_path(root);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::error!("db is corrupt ({}), starting fresh", e);
                Db::default()
            }),
            Err(_) => Db::default(),
        }
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(root)?;
        let bytes = serde_json::to_vec_pretty(self)?;
        let tmp = db_path(root).with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, db_path(root))?;
        Ok(())
    }

    pub fn asset(&self, id: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }
}

pub fn db_path(root: &Path) -> PathBuf {
    root.join("db.json")
}

pub fn assets_dir(root: &Path) -> PathBuf {
    root.join("assets")
}

/// Where an artifact's extracted preview image lives. The format is worked out
/// from the bytes when it is served, so no extension is needed here.
pub fn preview_path(root: &Path, id: &str) -> PathBuf {
    assets_dir(root).join(format!("{id}.preview"))
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn classify(name: &str) -> (String, AssetKind) {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let kind = match ext.as_str() {
        "stl" => AssetKind::Mesh,
        "step" | "stp" => AssetKind::Step,
        "sldprt" => AssetKind::SwPart,
        "sldasm" => AssetKind::SwAssembly,
        "slddrw" => AssetKind::SwDrawing,
        "json" => AssetKind::History,
        "png" | "jpg" | "jpeg" | "webp" | "gif" => AssetKind::Image,
        "mp4" | "mov" | "webm" => AssetKind::Video,
        _ => AssetKind::Other,
    };
    (ext, kind)
}
