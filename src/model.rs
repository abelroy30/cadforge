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

/// One step in how a document was read or built: a short operation, what it
/// found, and any sub-steps. Rendered as a tree in the studio, so the whole
/// path from upload to result is visible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub step: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Action>,
}

impl Action {
    pub fn new(step: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            step: step.into(),
            detail: detail.into(),
            children: Vec::new(),
        }
    }

    pub fn with(mut self, children: Vec<Action>) -> Self {
        self.children = children;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub ext: String,
    pub kind: AssetKind,
    pub size: u64,
    pub uploaded_at: String,
    /// Which door the file came in by: `sldworks`, `step`, `image`, `view`,
    /// `drawing` or `other`. It is what the two bulk upload lanes write, and it
    /// is how the studio and the trainer tell a part from a picture of one.
    #[serde(default)]
    pub lane: String,
    /// Set when the artifact was indexed from a folder rather than uploaded: the
    /// absolute path of the file where it lives. Nothing is copied, so pointing
    /// the studio at a 50 GB folder costs nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    /// The folder source this came from, when it came from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
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
    /// How the artifact was processed, step by step, as a tree the studio draws
    /// for every kind: uploaded, inspected, and what was found.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
}

impl Asset {
    /// Is this a SolidWorks document of some kind?
    pub fn is_solidworks(&self) -> bool {
        matches!(
            self.kind,
            AssetKind::SwPart | AssetKind::SwAssembly | AssetKind::SwDrawing
        )
    }

    /// Where this artifact's bytes are. One accessor for the whole API, because
    /// an indexed file stays where the user keeps it while an uploaded one lives
    /// in the studio's own store.
    pub fn bytes_path(&self, root: &Path) -> PathBuf {
        match &self.source_path {
            Some(p) => PathBuf::from(p),
            None => assets_dir(root).join(format!("{}.{}", self.id, self.ext)),
        }
    }

    /// Was this indexed in place rather than copied in?
    pub fn is_referenced(&self) -> bool {
        self.source_path.is_some()
    }
}

/// A folder of training data the user pointed the studio at.
///
/// A source is an *index*, never a copy: files are read where they live, so
/// pointing at a large folder costs nothing and removing a source never touches
/// the user's files. The trade is that moving or renaming them behind the
/// studio's back breaks the index, which is why a source can be re-scanned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    /// Absolute path to the folder.
    pub path: String,
    /// What to call it in the interface: the folder's own name unless the user
    /// says otherwise.
    pub label: String,
    pub state: SourceState,
    /// Files the last scan understood.
    pub indexed: usize,
    /// Files it walked past because nothing here reads them.
    pub skipped: usize,
    /// Total bytes of the indexed files: the honest 'physical size of the data'.
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scan: Option<String>,
    pub added_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SourceState {
    Idle,
    Scanning,
    Ready,
    Error,
}

/// How many folders one studio watches. Ten covers how people actually keep
/// data - a parts library, a drawings share, a captures folder - without the
/// panel turning into a file manager.
pub const MAX_SOURCES: usize = 10;

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
    /// Folders of training data. Referenced, never copied.
    #[serde(default)]
    pub sources: Vec<Source>,
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

/// The upload lane a file belongs to when the browser does not say.
///
/// The studio has two bulk lanes - SolidWorks documents and STEP files - plus
/// pictures; the lane is what a dataset is actually made of, so it is recorded
/// rather than guessed again later.
pub fn lane_for(ext: &str, kind: &AssetKind) -> &'static str {
    match ext {
        "sldprt" | "sldasm" | "slddrw" => "sldworks",
        "step" | "stp" => "step",
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => "image",
        _ => match kind {
            AssetKind::SwPart | AssetKind::SwAssembly | AssetKind::SwDrawing => "sldworks",
            AssetKind::Step => "step",
            AssetKind::Image => "image",
            _ => "other",
        },
    }
}
