//! HTTP API. JSON in, JSON out; one SSE stream carries live run progress.

use crate::model::{classify, now, Asset, AssetKind, BaseModel, Dataset, Db, Run, RunStatus};
use crate::{stl, App};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use futures::StreamExt;
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;

/// Everything the browser needs to render the whole studio on load.
async fn state(State(app): State<Arc<App>>) -> Json<Db> {
    Json(app.db.lock().unwrap().clone())
}

/// The pipeline the studio will run, so the UI can show the stages up front.
async fn trainer() -> Json<serde_json::Value> {
    use crate::pipeline::Trainer;
    let t = crate::pipeline::DefaultTrainer;
    let stages: Vec<serde_json::Value> = t
        .stages()
        .iter()
        .map(|s| serde_json::json!({ "label": s.label, "detail": s.detail }))
        .collect();
    Json(serde_json::json!({
        "name": t.name(),
        "external": std::env::var("CADFORGE_TRAINER").ok(),
        "stages": stages,
    }))
}

#[derive(Deserialize)]
struct UploadQuery {
    name: String,
}

/// Upload one artifact. The body is the raw file; the name arrives as a query
/// parameter, which keeps the upload path free of multipart parsing.
async fn upload_asset(
    State(app): State<Arc<App>>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> Result<Json<Asset>, ApiError> {
    if body.is_empty() {
        return Err(ApiError::bad_request("uploaded file is empty"));
    }
    let name = safe_name(&q.name);
    let (ext, kind) = classify(&name);
    let id = uuid::Uuid::new_v4().to_string();

    let dir = crate::model::assets_dir(&app.root);
    std::fs::create_dir_all(&dir).map_err(ApiError::internal)?;
    let path = dir.join(format!("{id}.{ext}"));
    std::fs::write(&path, &body).map_err(ApiError::internal)?;

    let mut asset = Asset {
        id: id.clone(),
        name,
        ext,
        kind: kind.clone(),
        size: body.len() as u64,
        uploaded_at: now(),
        triangles: None,
        bbox: None,
        header: None,
        step: None,
    };

    match kind {
        AssetKind::Mesh => match stl::parse(&body) {
            Ok(mesh) => {
                asset.triangles = Some(mesh.triangles);
                asset.bbox = Some(mesh.bbox);
            }
            Err(e) => return Err(ApiError::bad_request(format!("could not read STL: {e}"))),
        },
        AssetKind::Step => {
            asset.header = Some(step_header(&body));
            match crate::step::analyze(&String::from_utf8_lossy(&body)) {
                Ok(analysis) => asset.step = Some(analysis.summary),
                Err(e) => tracing::warn!("could not read STEP geometry: {e}"),
            }
        }
        _ => {}
    }

    let mut db = app.db.lock().unwrap();
    db.assets.push(asset.clone());
    db.save(&app.root).map_err(ApiError::internal)?;
    Ok(Json(asset))
}

fn safe_name(raw: &str) -> String {
    std::path::Path::new(raw)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("upload")
        .to_string()
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

/// Triangle soup for the viewport, as a flat float array.
async fn asset_mesh(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (path, meta) = {
        let db = app.db.lock().unwrap();
        let asset = db.asset(&id).ok_or_else(|| ApiError::not_found("asset"))?;
        (
            crate::model::assets_dir(&app.root).join(format!("{}.{}", asset.id, asset.ext)),
            asset.clone(),
        )
    };
    let bytes = std::fs::read(&path).map_err(|_| ApiError::not_found("asset file"))?;
    let mesh = stl::parse(&bytes).map_err(ApiError::bad_request)?;
    Ok(Json(serde_json::json!({
        "id": meta.id,
        "triangles": mesh.triangles,
        "bbox": mesh.bbox,
        "positions": mesh.positions,
    })))
}

/// The build history reconstructed from a STEP part, plus a preview to draw.
async fn asset_history(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (path, meta) = {
        let db = app.db.lock().unwrap();
        let asset = db.asset(&id).ok_or_else(|| ApiError::not_found("asset"))?;
        (
            crate::model::assets_dir(&app.root).join(format!("{}.{}", asset.id, asset.ext)),
            asset.clone(),
        )
    };
    let bytes = std::fs::read(&path).map_err(|_| ApiError::not_found("asset file"))?;
    let analysis = crate::step::analyze(&String::from_utf8_lossy(&bytes)).map_err(ApiError::bad_request)?;
    let (mode, positions) = if !analysis.triangles.is_empty() {
        ("triangles", analysis.triangles)
    } else if !analysis.points.is_empty() {
        ("points", analysis.points)
    } else {
        ("none", Vec::new())
    };
    Ok(Json(serde_json::json!({
        "id": meta.id,
        "name": meta.name,
        "summary": analysis.summary,
        "preview": { "mode": mode, "positions": positions },
    })))
}

/// Raw bytes back out, for download.
async fn asset_file(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let (path, name) = {
        let db = app.db.lock().unwrap();
        let asset = db.asset(&id).ok_or_else(|| ApiError::not_found("asset"))?;
        (
            crate::model::assets_dir(&app.root).join(format!("{}.{}", asset.id, asset.ext)),
            asset.name.clone(),
        )
    };
    let bytes = std::fs::read(&path).map_err(|_| ApiError::not_found("asset file"))?;
    Ok(([("content-disposition", format!("attachment; filename=\"{name}\"")),], bytes))
}

async fn delete_asset(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mut db = app.db.lock().unwrap();
    let before = db.assets.len();
    db.assets.retain(|a| a.id != id);
    if db.assets.len() == before {
        return Err(ApiError::not_found("asset"));
    }
    for ds in db.datasets.iter_mut() {
        ds.asset_ids.retain(|a| a != &id);
    }
    db.save(&app.root).map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct NewModel {
    name: String,
    #[serde(default = "default_kind")]
    kind: String,
    #[serde(default)]
    params: Option<u64>,
    #[serde(default)]
    notes: String,
}

fn default_kind() -> String {
    "base".into()
}

async fn create_model(
    State(app): State<Arc<App>>,
    Json(body): Json<NewModel>,
) -> Result<Json<BaseModel>, ApiError> {
    if body.name.trim().is_empty() {
        return Err(ApiError::bad_request("model needs a name"));
    }
    let model = BaseModel {
        id: uuid::Uuid::new_v4().to_string(),
        name: body.name.trim().to_string(),
        kind: body.kind,
        params: body.params,
        notes: body.notes,
        created_at: now(),
    };
    let mut db = app.db.lock().unwrap();
    db.models.push(model.clone());
    db.save(&app.root).map_err(ApiError::internal)?;
    Ok(Json(model))
}

#[derive(Deserialize)]
struct NewDataset {
    name: String,
    #[serde(default)]
    asset_ids: Vec<String>,
}

async fn create_dataset(
    State(app): State<Arc<App>>,
    Json(body): Json<NewDataset>,
) -> Result<Json<Dataset>, ApiError> {
    let ds = Dataset {
        id: uuid::Uuid::new_v4().to_string(),
        name: if body.name.trim().is_empty() { "Untitled dataset".into() } else { body.name.trim().to_string() },
        asset_ids: body.asset_ids,
        created_at: now(),
    };
    let mut db = app.db.lock().unwrap();
    db.datasets.push(ds.clone());
    db.save(&app.root).map_err(ApiError::internal)?;
    Ok(Json(ds))
}

#[derive(Deserialize)]
struct NewRun {
    #[serde(default)]
    name: String,
    base_model_id: String,
    dataset_id: String,
}

async fn create_run(
    State(app): State<Arc<App>>,
    Json(body): Json<NewRun>,
) -> Result<Json<Run>, ApiError> {
    let run = {
        let mut db = app.db.lock().unwrap();
        if !db.models.iter().any(|m| m.id == body.base_model_id) {
            return Err(ApiError::bad_request("unknown base model"));
        }
        let ds = db
            .datasets
            .iter()
            .find(|d| d.id == body.dataset_id)
            .ok_or_else(|| ApiError::bad_request("unknown dataset"))?;
        if ds.asset_ids.is_empty() {
            return Err(ApiError::bad_request("dataset has no artifacts in it"));
        }
        let run = Run {
            id: uuid::Uuid::new_v4().to_string(),
            name: if body.name.trim().is_empty() { "Training run".into() } else { body.name.trim().to_string() },
            base_model_id: body.base_model_id,
            dataset_id: body.dataset_id,
            status: RunStatus::Queued,
            stage: "Queued".into(),
            progress: 0.0,
            log: Vec::new(),
            created_at: now(),
            finished_at: None,
            outputs: Vec::new(),
        };
        db.runs.push(run.clone());
        db.save(&app.root).map_err(ApiError::internal)?;
        run
    };

    let app2 = app.clone();
    let id = run.id.clone();
    tokio::spawn(async move { crate::pipeline::execute(app2, id).await });
    Ok(Json(run))
}

async fn cancel_run(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<Run>, ApiError> {
    let mut db = app.db.lock().unwrap();
    let run = db.runs.iter_mut().find(|r| r.id == id).ok_or_else(|| ApiError::not_found("run"))?;
    if matches!(run.status, RunStatus::Queued | RunStatus::Running) {
        run.status = RunStatus::Cancelled;
        run.stage = "Cancelling".into();
    }
    let out = run.clone();
    db.save(&app.root).map_err(ApiError::internal)?;
    drop(db);
    let _ = app.bus.send(out.clone());
    Ok(Json(out))
}

/// Live run updates. Every committed change to a run is pushed here.
async fn events(
    State(app): State<Arc<App>>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let rx = app.bus.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(run) => {
                let data = serde_json::to_string(&run).unwrap_or_default();
                Some(Ok(Event::default().event("run").data(data)))
            }
            Err(_) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub fn router(app: Arc<App>) -> axum::Router {
    use axum::extract::DefaultBodyLimit;
    use axum::routing::{delete, get, post};

    axum::Router::new()
        .route("/api/state", get(state))
        .route("/api/trainer", get(trainer))
        .route("/api/events", get(events))
        .route("/api/assets", post(upload_asset))
        .route("/api/assets/{id}", delete(delete_asset))
        .route("/api/assets/{id}/mesh", get(asset_mesh))
        .route("/api/assets/{id}/history", get(asset_history))
        .route("/api/assets/{id}/file", get(asset_file))
        .route("/api/models", post(create_model))
        .route("/api/datasets", post(create_dataset))
        .route("/api/runs", post(create_run))
        .route("/api/runs/{id}/cancel", post(cancel_run))
        .layer(DefaultBodyLimit::max(512 * 1024 * 1024))
        .with_state(app)
}

/// One error shape for the whole API.
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: msg.into() }
    }
    fn not_found(what: &str) -> Self {
        Self { status: StatusCode::NOT_FOUND, message: format!("{what} not found") }
    }
    fn internal(e: impl std::fmt::Display) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: e.to_string() }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}
