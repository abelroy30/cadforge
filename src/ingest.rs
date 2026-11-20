//! Ingest: turning a file into an artifact, whether it arrived as an upload or
//! was found in a folder the user pointed the studio at.
//!
//! Two front doors, one pipeline. An upload copies its bytes into the studio; a
//! folder source reads files where they live and copies nothing. Both end in the
//! same inspection, so an artifact means the same thing however it got here, and
//! both report progress on the same bus - which is what lets the interface show
//! the extraction happening instead of only its result.

use crate::model::{classify, lane_for, now, Action, Asset, AssetKind, Db, Source, SourceState};
use crate::App;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One step of work in progress, for the studio's live view.
///
/// A single shape covers both a folder being walked and a single file being read,
/// because the interface shows them the same way: a name, what is happening to
/// it now, and how far along it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkEvent {
    /// `source` while a folder is being walked, `asset` while one file is read.
    pub kind: String,
    /// Id of the source or the artifact this is about.
    pub id: String,
    pub name: String,
    /// `walking`, `reading`, `done` or `error`.
    pub stage: String,
    pub detail: String,
    /// Items seen so far, and how many are expected. `total` is 0 while it is
    /// still unknown, which the interface shows as activity rather than a bar.
    pub seen: usize,
    pub total: usize,
    pub done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl WorkEvent {
    fn new(kind: &str, id: &str, name: &str, stage: &str, detail: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
            name: name.into(),
            stage: stage.into(),
            detail: detail.into(),
            seen: 0,
            total: 0,
            done: false,
            error: None,
        }
    }
}

fn emit(app: &App, event: WorkEvent) {
    let _ = app.work_bus.send(event);
}

/// Directories that never hold parts, and only slow a walk down.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "__pycache__",
    "target",
    ".cache",
    ".venv",
    "venv",
    ".Trash",
    "$RECYCLE.BIN",
    "System Volume Information",
];

/// The largest file a scan will read. Past this it is not a part or a picture of
/// one, and reading it would only block the walk.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// How many files one scan will look at unless the operator says otherwise.
const DEFAULT_SCAN_LIMIT: usize = 20_000;

/// Every file under a folder that could become an artifact, in a stable order.
///
/// Skips hidden directories, build directories and symlinks. Skipping symlinks
/// matters: `symlink_metadata` does not follow the link, so a link pointing back
/// up its own tree cannot send the walk round in circles. Sorted so a re-scan
/// visits files in the same order and produces the same artifacts.
fn walk(root: &Path, limit: usize) -> (Vec<PathBuf>, usize) {
    let mut files = Vec::new();
    let mut skipped = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if files.len() >= limit {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if files.len() >= limit {
                break;
            }
            let path = entry.path();
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || SKIP_DIRS.iter().any(|s| s.eq_ignore_ascii_case(&name))
                {
                    continue;
                }
                stack.push(path);
            } else if meta.is_file() {
                if meta.len() > MAX_FILE_BYTES {
                    skipped += 1;
                    continue;
                }
                files.push(path);
            }
        }
    }
    files.sort();
    (files, skipped)
}

/// Total bytes under a directory, following plain directories only. Used for the
/// studio's own footprint, where the file count is ours and small.
pub fn dir_bytes(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else {
                total += meta.len();
            }
        }
    }
    total
}

pub fn safe_name(raw: &str) -> String {
    Path::new(raw)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("upload")
        .to_string()
}

/// Read a file's bytes into an artifact.
///
/// The one place that knows what each kind of document means, so an upload and a
/// folder scan can never disagree about what a file contains.
#[allow(clippy::too_many_arguments)]
pub fn inspect(
    app: &App,
    name: String,
    ext: String,
    kind: AssetKind,
    lane: String,
    source_path: Option<String>,
    source_id: Option<String>,
    bytes: &[u8],
) -> Result<Asset, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let mut asset = Asset {
        id,
        name,
        ext,
        kind: kind.clone(),
        size: bytes.len() as u64,
        uploaded_at: now(),
        lane,
        source_path,
        source_id,
        triangles: None,
        bbox: None,
        header: None,
        step: None,
        catalogue: None,
        actions: Vec::new(),
    };

    match kind {
        AssetKind::Mesh => match crate::stl::parse(bytes) {
            Ok(mesh) => {
                asset.triangles = Some(mesh.triangles);
                asset.bbox = Some(mesh.bbox);
            }
            Err(e) => return Err(format!("could not read STL: {e}")),
        },
        AssetKind::Step => {
            asset.header = Some(step_header(bytes));
            match crate::step::analyze(&String::from_utf8_lossy(bytes)) {
                Ok(analysis) => asset.step = Some(analysis.summary),
                Err(e) => tracing::warn!("could not read STEP geometry: {e}"),
            }
        }
        AssetKind::SwPart | AssetKind::SwAssembly | AssetKind::SwDrawing => {
            // A document we can open: read the container, keep the preview image,
            // and record what is inside it.
            match crate::sldworks::inspect(bytes, &asset.ext) {
                Ok(inspected) => {
                    asset.header = Some(inspected.catalogue.headline());
                    if let Some(preview) = &inspected.preview {
                        let path = crate::model::preview_path(&app.root, &asset.id);
                        if let Err(e) = std::fs::write(&path, &preview.bytes) {
                            tracing::warn!("could not store preview for {}: {e}", asset.name);
                        }
                    }
                    asset.catalogue = Some(inspected.catalogue);
                }
                Err(e) => {
                    // Keep the file - the user dropped it here - but say plainly
                    // that it could not be read.
                    asset.header = Some(format!("could not be read as a document: {e}"));
                    tracing::warn!("could not read {}: {e}", asset.name);
                }
            }
        }
        AssetKind::History => {
            // A feature tree exported from inside SolidWorks. This is the true
            // modelling history, not an inference from geometry.
            match crate::swhist::parse(&String::from_utf8_lossy(bytes)) {
                Ok(export) => {
                    let summary = crate::swhist::summary(&export);
                    let n = summary.steps.len().saturating_sub(1);
                    asset.header = Some(format!(
                        "{} - {} feature{} exported from SolidWorks",
                        if export.document.is_empty() {
                            "feature tree"
                        } else {
                            export.document.as_str()
                        },
                        n,
                        if n == 1 { "" } else { "s" }
                    ));
                    asset.step = Some(summary);
                }
                Err(e) => return Err(format!("could not read history export: {e}")),
            }
        }
        _ => {}
    }

    asset.actions = actions_for(&asset);
    Ok(asset)
}

/// Record an artifact and tell every open studio about it.
///
/// Upserts by id, so re-scanning a folder updates the records it already has
/// instead of piling up duplicates of the same file.
fn commit(app: &App, asset: Asset) {
    {
        let mut db = app.db.lock().unwrap();
        match db.assets.iter_mut().find(|a| a.id == asset.id) {
            Some(slot) => *slot = asset.clone(),
            None => db.assets.push(asset.clone()),
        }
        if let Err(e) = db.save(&app.root) {
            tracing::warn!("could not save db after ingest: {e}");
        }
    }
    let _ = app.asset_bus.send(asset);
}

/// Take an uploaded file: store the bytes in the studio, read them, and report.
pub fn upload(
    app: &App,
    raw_name: &str,
    lane: Option<String>,
    bytes: &[u8],
) -> Result<Asset, String> {
    if bytes.is_empty() {
        return Err("uploaded file is empty".into());
    }
    let name = safe_name(raw_name);
    let (ext, kind) = classify(&name);
    let lane = lane
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| lane_for(&ext, &kind).to_string());

    let asset = inspect(app, name, ext, kind, lane, None, None, bytes)?;

    // The record exists first because the inspection needs the id to name the
    // preview it may extract; the bytes land at that id's path straight after.
    let dir = crate::model::assets_dir(&app.root);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.{}", asset.id, asset.ext));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;

    commit(app, asset.clone());
    Ok(asset)
}

/// Read a file the user pointed at, from where it lives. Nothing is copied.
pub fn read_referenced(app: &App, file: &Path, source_id: &str) -> Result<Asset, String> {
    let name = file
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "file has no usable name".to_string())?
        .to_string();
    let (ext, kind) = classify(&name);
    if kind == AssetKind::Other {
        return Err("nothing here reads that kind of file".into());
    }
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let lane = lane_for(&ext, &kind).to_string();
    inspect(
        app,
        name,
        ext,
        kind,
        lane,
        Some(file.to_string_lossy().to_string()),
        Some(source_id.to_string()),
        &bytes,
    )
}

/// Walk one source and index what it holds.
///
/// Blocking on purpose: it is file I/O end to end and the events it emits are
/// what make it visible, so it runs on a blocking thread and never holds the db
/// lock across a read.
pub fn scan_worker(app: Arc<App>, source_id: String) {
    let (root, label) = {
        let db = app.db.lock().unwrap();
        match db.sources.iter().find(|s| s.id == source_id) {
            Some(s) => (s.path.clone(), s.label.clone()),
            None => return,
        }
    };

    set_source(&app, &source_id, |s| {
        s.state = SourceState::Scanning;
        s.error = None;
    });
    emit(
        &app,
        WorkEvent::new(
            "source",
            &source_id,
            &label,
            "walking",
            format!("looking through {root}"),
        ),
    );

    let limit = std::env::var("CADFORGE_SCAN_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_SCAN_LIMIT);
    let (files, mut skipped) = walk(Path::new(&root), limit);
    let total = files.len();

    {
        let mut event = WorkEvent::new(
            "source",
            &source_id,
            &label,
            "reading",
            format!("{total} file(s) to read"),
        );
        event.total = total;
        emit(&app, event);
    }

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut indexed = 0usize;
    let mut bytes = 0u64;

    for (i, file) in files.iter().enumerate() {
        let abs = file.to_string_lossy().to_string();
        let shown = file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&abs)
            .to_string();

        match read_referenced(&app, file, &source_id) {
            Ok(mut asset) => {
                // Keep the id a previous scan gave this path, so a re-scan
                // refreshes records rather than duplicating them.
                if let Some(existing) = find_by_path(&app, &abs) {
                    asset.id = existing;
                }
                let size = asset.size;
                let detail = headline(&asset);
                commit(&app, asset);
                seen.insert(abs);
                indexed += 1;
                bytes += size;
                let mut event = WorkEvent::new("asset", &source_id, &shown, "reading", detail);
                event.seen = i + 1;
                event.total = total;
                emit(&app, event);
            }
            Err(e) => {
                skipped += 1;
                let mut event = WorkEvent::new("asset", &source_id, &shown, "skipped", e);
                event.seen = i + 1;
                event.total = total;
                emit(&app, event);
            }
        }
    }

    // A file that vanished since the last scan should stop being an artifact:
    // keeping it would leave the trainer pointing at something that is gone.
    let dropped = drop_vanished(&app, &source_id, &seen);

    set_source(&app, &source_id, |s| {
        s.state = SourceState::Ready;
        s.indexed = indexed;
        s.skipped = skipped;
        s.bytes = bytes;
        s.last_scan = Some(now());
    });

    let mut event = WorkEvent::new(
        "source",
        &source_id,
        &label,
        "done",
        format!(
            "{indexed} indexed, {skipped} skipped, {dropped} removed - {}",
            human_bytes(bytes)
        ),
    );
    event.seen = total;
    event.total = total;
    event.done = true;
    emit(&app, event);
}

fn set_source(app: &App, id: &str, f: impl FnOnce(&mut Source)) {
    let mut db = app.db.lock().unwrap();
    if let Some(s) = db.sources.iter_mut().find(|s| s.id == id) {
        f(s);
    }
    if let Err(e) = db.save(&app.root) {
        tracing::warn!("could not save db after scan: {e}");
    }
}

/// The id of an existing artifact for this exact path, if there is one.
fn find_by_path(app: &App, path: &str) -> Option<String> {
    let db = app.db.lock().unwrap();
    db.assets
        .iter()
        .find(|a| a.source_path.as_deref() == Some(path))
        .map(|a| a.id.clone())
}

/// Forget artifacts of this source whose file is no longer there.
fn drop_vanished(app: &App, source_id: &str, seen: &BTreeSet<String>) -> usize {
    let mut db = app.db.lock().unwrap();
    let before = db.assets.len();
    db.assets.retain(|a| {
        if a.source_id.as_deref() != Some(source_id) {
            return true;
        }
        match &a.source_path {
            Some(p) => seen.contains(p),
            None => true,
        }
    });
    let dropped = before - db.assets.len();
    if dropped > 0 {
        if let Err(e) = db.save(&app.root) {
            tracing::warn!("could not save db after dropping vanished files: {e}");
        }
    }
    dropped
}

/// A one-line answer to 'what did this file turn out to be'.
fn headline(asset: &Asset) -> String {
    if let Some(header) = &asset.header {
        let first = header.lines().next().unwrap_or("").trim();
        if !first.is_empty() {
            return first.to_string();
        }
    }
    if let Some(s) = &asset.step {
        let ops: Vec<&str> = s.steps.iter().map(|l| l.op.as_str()).collect();
        if !ops.is_empty() {
            return format!("{} faces - {}", s.faces, ops.join(" + "));
        }
    }
    format!("{}", human_bytes(asset.size))
}

/// A step-by-step account of how an artifact was processed, for every kind, so
/// the studio can draw the whole path as a tree. Nothing about an ingest is
/// hidden: what was read, what was found, and where each value came from.
pub fn actions_for(asset: &Asset) -> Vec<Action> {
    let mut root = vec![Action::new(
        if asset.is_referenced() {
            "indexed"
        } else {
            "upload"
        },
        format!("{} (.{}) · {} KB", asset.name, asset.ext, asset.size / 1024),
    )];

    match asset.kind {
        AssetKind::Mesh => {
            if let (Some(triangles), Some(b)) = (asset.triangles, asset.bbox) {
                root.push(
                    Action::new("read mesh", format!("{triangles} triangles")).with(vec![
                        Action::new(
                            "bounds",
                            format!(
                                "{:.2} × {:.2} × {:.2}",
                                b[3] - b[0],
                                b[4] - b[1],
                                b[5] - b[2]
                            ),
                        ),
                    ]),
                );
            }
        }
        AssetKind::Step => {
            if let Some(header) = &asset.header {
                root.push(Action::new("read header", header.clone()));
            }
            if let Some(s) = &asset.step {
                root.push(
                    Action::new(
                        "reconstruct history",
                        format!(
                            "from the B-rep: {} faces, {} edges, {} vertices",
                            s.faces, s.edges, s.vertices
                        ),
                    )
                    .with(
                        s.steps
                            .iter()
                            .map(|l| Action::new(l.op.clone(), l.detail.clone()))
                            .collect(),
                    ),
                );
            }
        }
        AssetKind::History => {
            if let Some(s) = &asset.step {
                root.push(
                    Action::new(
                        "read recording",
                        "the feature tree exactly as SolidWorks recorded it",
                    )
                    .with(
                        s.steps
                            .iter()
                            .map(|l| Action::new(l.op.clone(), l.detail.clone()))
                            .collect(),
                    ),
                );
            }
        }
        AssetKind::SwPart | AssetKind::SwAssembly | AssetKind::SwDrawing => {
            match &asset.catalogue {
                Some(c) => {
                    root.push(Action::new("open container", c.container.clone()));

                    let total: usize = c.metadata.iter().map(|g| g.properties.len()).sum();
                    if total > 0 {
                        let groups: Vec<Action> = c
                            .metadata
                            .iter()
                            .filter(|g| !g.properties.is_empty())
                            .map(|g| {
                                let label = match g.group.as_str() {
                                    "summary" => "SummaryInformation",
                                    "document" => "DocumentSummaryInformation",
                                    "custom" => "custom properties (user-defined)",
                                    other => other,
                                };
                                Action::new(label, format!("{} parameter(s)", g.properties.len()))
                                    .with(
                                        g.properties
                                            .iter()
                                            .map(|p| Action::new(p.name.clone(), p.value.clone()))
                                            .collect(),
                                    )
                            })
                            .collect();
                        root.push(
                            Action::new("read parameters", format!("{total} parameter(s)"))
                                .with(groups),
                        );
                    } else {
                        root.push(Action::new(
                            "read parameters",
                            "no parameters stored in this file",
                        ));
                    }

                    if !c.references.is_empty() {
                        root.push(
                            Action::new(
                                "resolve references",
                                format!("{} document(s)", c.references.len()),
                            )
                            .with(
                                c.references
                                    .iter()
                                    .map(|r| {
                                        Action::new(
                                            r.name.clone(),
                                            format!("found in `{}`", r.stream),
                                        )
                                    })
                                    .collect(),
                            ),
                        );
                    }

                    let streams: Vec<Action> = c
                        .streams
                        .iter()
                        .map(|s| {
                            Action::new(
                                s.name.clone(),
                                format!("{} · {} KB", s.role, s.size / 1024),
                            )
                        })
                        .collect();
                    root.push(
                        Action::new("list streams", format!("{} stream(s)", c.streams.len()))
                            .with(streams),
                    );

                    if let Some(p) = &c.preview {
                        root.push(Action::new(
                            "extract preview",
                            format!("{} from `{}`", p.content_type, p.stream),
                        ));
                    }
                }
                None => {
                    if let Some(header) = &asset.header {
                        root.push(Action::new("open container", header.clone()));
                    }
                }
            }
        }
        _ => {}
    }
    root
}

/// Pull the schema and product name out of a STEP header.
fn step_header(bytes: &[u8]) -> String {
    let head: String = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).into();
    let mut parts = Vec::new();
    for key in ["FILE_NAME", "FILE_SCHEMA", "FILE_DESCRIPTION"] {
        if let Some(rest) = head.split(key).nth(1) {
            if let Some(inner) = rest.split('\'').nth(1) {
                if !inner.trim().is_empty() {
                    parts.push(format!("{key}: {}", inner.trim()));
                }
            }
        }
    }
    if parts.is_empty() {
        "STEP part (no header parsed)".into()
    } else {
        parts.join("\n")
    }
}

/// Bytes, in the units a person reads.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// What the data and the models actually weigh.
///
/// Two numbers are reported differently because they are known differently. The
/// size of the data is *measured*: it is the sum of the files that were indexed.
/// The size of a base model is *estimated* from its parameter count, because the
/// studio does not download weights and has no file to measure - and an estimate
/// that says so is worth more than a number that pretends otherwise.
pub fn storage_report(root: &Path, db: &Db) -> serde_json::Value {
    let indexed_bytes: u64 = db.sources.iter().map(|s| s.bytes).sum();
    let uploaded_bytes: u64 = db
        .assets
        .iter()
        .filter(|a| !a.is_referenced())
        .map(|a| a.size)
        .sum();

    let mut by_lane: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    for a in &db.assets {
        let entry = by_lane.entry(a.lane.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += a.size;
    }

    let runs_bytes = dir_bytes(&root.join("runs"));
    let store_bytes = dir_bytes(root);

    // Real adapter sizes, from the runs that produced them. This is the one
    // model-related number that is measured rather than estimated.
    let mut adapters_by_model: BTreeMap<String, u64> = BTreeMap::new();
    let runs: Vec<serde_json::Value> = db
        .runs
        .iter()
        .map(|r| {
            let dir = root.join("runs").join(&r.id);
            let mut total = 0u64;
            let outputs: Vec<serde_json::Value> = r
                .outputs
                .iter()
                .map(|name| {
                    let bytes = std::fs::metadata(dir.join(name))
                        .map(|m| m.len())
                        .unwrap_or(0);
                    total += bytes;
                    serde_json::json!({ "name": name, "bytes": bytes })
                })
                .collect();
            *adapters_by_model
                .entry(r.base_model_id.clone())
                .or_insert(0) += total;
            serde_json::json!({
                "id": r.id,
                "name": r.name,
                "status": r.status,
                "bytes": total,
                "outputs": outputs,
            })
        })
        .collect();

    let models: Vec<serde_json::Value> = db
        .models
        .iter()
        .map(|m| {
            let params = m.params.unwrap_or(0);
            let adapters = adapters_by_model.get(&m.id).copied().unwrap_or(0);
            serde_json::json!({
                "id": m.id,
                "name": m.name,
                "kind": m.kind,
                "params": m.params,
                // Bytes per weight, at the precisions people actually run.
                "weights_estimate": {
                    "fp16": params * 2,
                    "int8": params,
                    "int4": params / 2,
                },
                "adapters_bytes": adapters,
                "measured": params == 0 && adapters > 0,
            })
        })
        .collect();

    let sources: Vec<serde_json::Value> = db
        .sources
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "label": s.label,
                "path": s.path,
                "state": s.state,
                "bytes": s.bytes,
                "indexed": s.indexed,
                "skipped": s.skipped,
                "last_scan": s.last_scan,
            })
        })
        .collect();

    serde_json::json!({
        "data": {
            "indexed_bytes": indexed_bytes,
            "uploaded_bytes": uploaded_bytes,
            "total_bytes": indexed_bytes + uploaded_bytes,
            "artifacts": db.assets.len(),
            "indexed_artifacts": db.assets.iter().filter(|a| a.is_referenced()).count(),
        },
        "sources": sources,
        "by_lane": by_lane
            .into_iter()
            .map(|(lane, (count, bytes))| (lane, serde_json::json!({ "artifacts": count, "bytes": bytes })))
            .collect::<serde_json::Map<_, _>>(),
        "store": {
            "data_dir_bytes": store_bytes,
            "assets_dir_bytes": dir_bytes(&crate::model::assets_dir(root)),
            "runs_dir_bytes": runs_bytes,
        },
        "models": models,
        "runs": runs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("cadforge-ingest-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_walk_finds_files_at_any_depth_and_ignores_junk() {
        let root = temp_dir("walk");
        std::fs::create_dir_all(root.join("parts/deep")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("plate.step"), b"ISO-10303-21;").unwrap();
        std::fs::write(root.join("parts/bracket.SLDPRT"), b"x").unwrap();
        std::fs::write(root.join("parts/deep/notes.txt"), b"hi").unwrap();
        std::fs::write(root.join(".git/config"), b"x").unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), b"x").unwrap();

        let (files, _) = walk(&root, DEFAULT_SCAN_LIMIT);
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        // The three real files, and nothing from a build or a hidden directory.
        assert_eq!(names, vec!["bracket.SLDPRT", "notes.txt", "plate.step"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_walk_is_stable_so_a_rescan_cannot_multiply_artifacts() {
        let root = temp_dir("stable");
        for name in ["c.step", "a.step", "b.step"] {
            std::fs::write(root.join(name), b"x").unwrap();
        }
        let (first, _) = walk(&root, DEFAULT_SCAN_LIMIT);
        let (second, _) = walk(&root, DEFAULT_SCAN_LIMIT);
        assert_eq!(first, second);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_walk_stops_at_the_limit_rather_than_reading_forever() {
        let root = temp_dir("limit");
        for i in 0..10 {
            std::fs::write(root.join(format!("part-{i}.step")), b"x").unwrap();
        }
        let (files, _) = walk(&root, 4);
        assert_eq!(files.len(), 4);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn measured_bytes_are_the_sum_of_what_is_there() {
        let root = temp_dir("bytes");
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("a.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(root.join("nested/b.bin"), vec![0u8; 250]).unwrap();
        assert_eq!(dir_bytes(&root), 350);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn byte_sizes_read_the_way_a_person_writes_them() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }
}
