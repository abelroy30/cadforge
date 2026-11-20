//! HTTP API. JSON in, JSON out; one SSE stream carries live run progress.

use crate::model::{
    now, Asset, BaseModel, Dataset, Db, Run, RunStatus, Source, SourceState, MAX_SOURCES,
};
use crate::{ingest, stl, App};
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
    /// Which bulk lane the file came in by. Optional: a file dropped straight on
    /// the viewport is classified from its extension instead.
    #[serde(default)]
    lane: Option<String>,
}

/// Upload one artifact. The body is the raw file; the name arrives as a query
/// parameter, which keeps the upload path free of multipart parsing.
async fn upload_asset(
    State(app): State<Arc<App>>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> Result<Json<Asset>, ApiError> {
    let asset =
        ingest::upload(&app, &q.name, q.lane.clone(), &body).map_err(ApiError::bad_request)?;
    Ok(Json(asset))
}

/// The stream catalogue of a stored document.
///
/// Recomputed on demand when it is missing, so documents uploaded before they
/// could be read still work.
async fn asset_streams(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let asset = {
        let db = app.db.lock().unwrap();
        db.asset(&id)
            .cloned()
            .ok_or_else(|| ApiError::not_found("asset"))?
    };

    if let Some(catalogue) = &asset.catalogue {
        return Ok(Json(serde_json::json!({
            "id": asset.id,
            "name": asset.name,
            "catalogue": catalogue,
            "preview_url": catalogue.preview.as_ref().map(|_| format!("/api/assets/{id}/preview")),
        })));
    }

    let ext = std::path::Path::new(&asset.name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let path = asset.bytes_path(&app.root);
    let bytes = std::fs::read(&path).map_err(|_| ApiError::not_found("asset file"))?;
    let inspected = crate::sldworks::inspect(&bytes, &ext).map_err(ApiError::bad_request)?;

    if let Some(preview) = &inspected.preview {
        let ppath = crate::model::preview_path(&app.root, &asset.id);
        let _ = std::fs::write(&ppath, &preview.bytes);
    }
    let mut db = app.db.lock().unwrap();
    let updated = if let Some(a) = db.assets.iter_mut().find(|a| a.id == asset.id) {
        a.catalogue = Some(inspected.catalogue.clone());
        a.header = Some(inspected.catalogue.headline());
        a.actions = ingest::actions_for(a);
        Some(a.clone())
    } else {
        None
    };
    db.save(&app.root).map_err(ApiError::internal)?;
    drop(db);
    if let Some(a) = updated {
        let _ = app.asset_bus.send(a);
    }

    Ok(Json(serde_json::json!({
        "id": asset.id,
        "name": asset.name,
        "catalogue": inspected.catalogue,
        "preview_url": inspected.preview.as_ref().map(|_| format!("/api/assets/{id}/preview")),
    })))
}

/// The preview image extracted from a document, if it had one.
async fn asset_preview(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let stored = {
        let db = app.db.lock().unwrap();
        let asset = db.asset(&id).ok_or_else(|| ApiError::not_found("asset"))?;
        asset
            .catalogue
            .as_ref()
            .and_then(|c| c.preview.as_ref())
            .map(|p| p.content_type.clone())
    };
    let path = crate::model::preview_path(&app.root, &id);
    let bytes = std::fs::read(&path).map_err(|_| ApiError::not_found("preview image"))?;
    // Trust the bytes over the record: the file may outlive a stale catalogue.
    let content_type = crate::sldworks::sniff_image(&bytes)
        .map(|(_, ct)| ct.to_string())
        .or(stored)
        .unwrap_or_else(|| "application/octet-stream".to_string());
    Ok(([("content-type", content_type)], bytes))
}

/// Triangle soup for the viewport, as a flat float array.
async fn asset_mesh(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (path, meta) = {
        let db = app.db.lock().unwrap();
        let asset = db.asset(&id).ok_or_else(|| ApiError::not_found("asset"))?;
        (asset.bytes_path(&app.root), asset.clone())
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

/// Which rebuild engines this machine has.
async fn engines() -> Json<serde_json::Value> {
    let freecad = crate::freecad::command();
    Json(serde_json::json!({
        "engines": [
            {
                "id": "native",
                "name": "cadforge native",
                "available": true,
                "note": "built in: extrudes a profile and drills holes. Cannot fillet."
            },
            {
                "id": "freecad",
                "name": "FreeCAD",
                "available": freecad.is_some(),
                "path": freecad,
                "note": "emits a FreeCAD macro and runs it; needs freecadcmd on PATH."
            }
        ]
    }))
}

#[derive(Deserialize)]
struct RebuildRequest {
    #[serde(default)]
    asset_id: Option<String>,
    #[serde(default)]
    program: Option<crate::ops::Program>,
    #[serde(default)]
    engine: Option<String>,
}

/// Rebuild a part from an operation program and say how close it came.
async fn rebuild(
    State(app): State<Arc<App>>,
    Json(body): Json<RebuildRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Either we were handed a program, or we read one back out of a stored part.
    let (program, target) = match (&body.program, &body.asset_id) {
        (Some(p), _) => (p.clone(), None),
        (None, Some(id)) => {
            let (path, _) = asset_path(&app, id)?;
            let bytes = std::fs::read(&path).map_err(|_| ApiError::not_found("asset file"))?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let analysis = crate::step::analyze(&text).map_err(ApiError::bad_request)?;
            let file = crate::step::StepFile::parse(&text).map_err(ApiError::bad_request)?;
            (crate::step::to_program(&file), Some(analysis.triangles))
        }
        (None, None) => return Err(ApiError::bad_request("give me an asset_id or a program")),
    };
    program.validate().map_err(ApiError::bad_request)?;

    let engine = body.engine.clone().unwrap_or_else(|| "native".into());
    let mut script = None;
    let mut ran = true;
    let mut note;
    let triangles;
    let ops_applied;

    match engine.as_str() {
        "native" => {
            let r = crate::solid::evaluate(&program).map_err(ApiError::bad_request)?;
            note = format!("{} face(s) built", r.solid.faces.len());
            triangles = r.solid.triangulate();
            ops_applied = r.ops_applied;
            if !r.warnings.is_empty() {
                note.push_str("; see warnings");
            }
            return Ok(Json(finish(
                app.as_ref(),
                program,
                engine,
                ran,
                ops_applied,
                r.warnings,
                triangles,
                target,
                script,
                note,
            )));
        }
        "freecad" => {
            let out_dir = app.root.join("rebuilds");
            let s = crate::freecad::script(&program, &out_dir).map_err(ApiError::bad_request)?;
            script = Some(s);
            if crate::freecad::command().is_none() {
                ran = false;
                note = "FreeCAD is not installed here, so the macro was generated but not run. It is returned in `script`.".into();
                return Ok(Json(finish(
                    app.as_ref(),
                    program,
                    engine,
                    ran,
                    0,
                    Vec::new(),
                    Vec::new(),
                    target,
                    script,
                    note,
                )));
            }
            let log = crate::freecad::run(&program, &out_dir).map_err(ApiError::bad_request)?;
            let stl_path = out_dir.join("rebuilt.stl");
            let bytes = std::fs::read(&stl_path).map_err(|e| {
                ApiError::internal(format!(
                    "FreeCAD did not produce {}: {e}",
                    stl_path.display()
                ))
            })?;
            let mesh = crate::stl::parse(&bytes).map_err(ApiError::internal)?;
            note = log.trim().to_string();
            triangles = mesh.positions;
            ops_applied = program.ops.len();
            return Ok(Json(finish(
                app.as_ref(),
                program,
                engine,
                ran,
                ops_applied,
                Vec::new(),
                triangles,
                target,
                script,
                note,
            )));
        }
        other => return Err(ApiError::bad_request(format!("unknown engine {other}"))),
    }
}

fn asset_path(app: &App, id: &str) -> Result<(std::path::PathBuf, String), ApiError> {
    let db = app.db.lock().unwrap();
    let asset = db.asset(id).ok_or_else(|| ApiError::not_found("asset"))?;
    Ok((asset.bytes_path(&app.root), asset.name.clone()))
}

#[allow(clippy::too_many_arguments)]
fn finish(
    _app: &App,
    program: crate::ops::Program,
    engine: String,
    ran: bool,
    ops_applied: usize,
    warnings: Vec<String>,
    triangles: Vec<f32>,
    target: Option<Vec<f32>>,
    script: Option<String>,
    note: String,
) -> serde_json::Value {
    let score = match &target {
        Some(t) if !triangles.is_empty() => crate::score::compare(&triangles, t),
        _ => None,
    };
    let bounds = if triangles.is_empty() {
        None
    } else {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in triangles.chunks_exact(3) {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        Some((lo, hi))
    };
    let ops = program.op_names();
    serde_json::json!({
        "program": program,
        "ops": ops,
        "engine": engine,
        "ran": ran,
        "ops_applied": ops_applied,
        "warnings": warnings,
        "note": note,
        "script": script,
        "score": score,
        "bounds": bounds,
        "triangles": triangles,
    })
}

/// The build history reconstructed from a STEP part, plus a preview to draw.
async fn asset_history(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (path, meta) = {
        let db = app.db.lock().unwrap();
        let asset = db.asset(&id).ok_or_else(|| ApiError::not_found("asset"))?;
        (asset.bytes_path(&app.root), asset.clone())
    };
    let bytes = std::fs::read(&path).map_err(|_| ApiError::not_found("asset file"))?;
    let analysis =
        crate::step::analyze(&String::from_utf8_lossy(&bytes)).map_err(ApiError::bad_request)?;
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
        (asset.bytes_path(&app.root), asset.name.clone())
    };
    let bytes = std::fs::read(&path).map_err(|_| ApiError::not_found("asset file"))?;
    Ok((
        [(
            "content-disposition",
            format!("attachment; filename=\"{name}\""),
        )],
        bytes,
    ))
}

async fn delete_asset(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mut db = app.db.lock().unwrap();
    let before = db.assets.len();
    let (ext, referenced) = db
        .asset(&id)
        .map(|a| (a.ext.clone(), a.is_referenced()))
        .ok_or_else(|| ApiError::not_found("asset"))?;
    db.assets.retain(|a| a.id != id);
    if db.assets.len() == before {
        return Err(ApiError::not_found("asset"));
    }
    for ds in db.datasets.iter_mut() {
        ds.asset_ids.retain(|a| a != &id);
    }
    db.save(&app.root).map_err(ApiError::internal)?;
    drop(db);

    // Take the files with the record - but only the ones this studio owns. A
    // referenced file lives in the user's own folder and is left alone.
    if !referenced {
        let _ =
            std::fs::remove_file(crate::model::assets_dir(&app.root).join(format!("{id}.{ext}")));
    }
    let _ = std::fs::remove_file(crate::model::preview_path(&app.root, &id));
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
        name: if body.name.trim().is_empty() {
            "Untitled dataset".into()
        } else {
            body.name.trim().to_string()
        },
        asset_ids: body.asset_ids,
        created_at: now(),
    };
    let mut db = app.db.lock().unwrap();
    db.datasets.push(ds.clone());
    db.save(&app.root).map_err(ApiError::internal)?;
    Ok(Json(ds))
}

/// Is a run still working with this model or dataset? Deleting something a live
/// run is using would leave that run with nothing to record against, so it is
/// refused and the reason is given. Past runs keep the id they recorded.
async fn delete_model(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mut db = app.db.lock().unwrap();
    if !db.models.iter().any(|m| m.id == id) {
        return Err(ApiError::not_found("model"));
    }
    if let Some(run) = db.runs.iter().find(|r| {
        r.base_model_id == id && (r.status == RunStatus::Queued || r.status == RunStatus::Running)
    }) {
        return Err(ApiError::bad_request(format!(
            "\"{}\" is still running against that model - cancel it first",
            run.name
        )));
    }
    db.models.retain(|m| m.id != id);
    db.save(&app.root).map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Remove a dataset. Its artifacts are untouched: a dataset is only ever a list
/// of ids, so nothing on disk belongs to it.
async fn delete_dataset(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mut db = app.db.lock().unwrap();
    if !db.datasets.iter().any(|d| d.id == id) {
        return Err(ApiError::not_found("dataset"));
    }
    if let Some(run) = db.runs.iter().find(|r| {
        r.dataset_id == id && (r.status == RunStatus::Queued || r.status == RunStatus::Running)
    }) {
        return Err(ApiError::bad_request(format!(
            "\"{}\" is still training on that dataset - cancel it first",
            run.name
        )));
    }
    db.datasets.retain(|d| d.id != id);
    db.save(&app.root).map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
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
            name: if body.name.trim().is_empty() {
                "Training run".into()
            } else {
                body.name.trim().to_string()
            },
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
    let run = db
        .runs
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or_else(|| ApiError::not_found("run"))?;
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

/// Live artifact updates: every artifact is pushed here the moment it has been
/// processed, so a bulk upload appears in the studio as it happens.
async fn asset_events(
    State(app): State<Arc<App>>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let rx = app.asset_bus.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(asset) => {
                let data = serde_json::to_string(&asset).unwrap_or_default();
                Some(Ok(Event::default().event("asset").data(data)))
            }
            Err(_) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// What the machine can do: the graphics card, the Python stack, and what to
/// install if something is missing. The studio shows this before a run, so a
/// four-billion-parameter plan is grounded in the hardware that will run it.
async fn system() -> Json<crate::system::SystemInfo> {
    // Probing shells out to nvidia-smi and python, which are slow enough to be
    // worth keeping off the async worker.
    let info = tokio::task::spawn_blocking(crate::system::detect)
        .await
        .unwrap_or_else(|_| crate::system::detect());
    Json(info)
}

#[derive(Deserialize)]
struct PredictQuery {
    w: usize,
    h: usize,
    #[serde(default)]
    asset_id: Option<String>,
}

/// Turn a picture into build steps.
///
/// The body is the picture's raw RGBA pixels - the browser has already decoded
/// whatever the user dropped, so the studio needs no image decoder of its own.
/// If `CADFORGE_VISION` names a command, that model is asked first; otherwise,
/// or if it fails, the built-in geometric baseline answers, and says so.
async fn predict(
    Query(q): Query<PredictQuery>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    if body.is_empty() {
        return Err(ApiError::bad_request(
            "POST the picture's pixels: the body must be the raw RGBA of a w x h image",
        ));
    }
    if q.w == 0 || q.h == 0 || q.w * q.h > 4096 * 4096 {
        return Err(ApiError::bad_request(
            "w and h must describe a sensible picture",
        ));
    }

    let mut notes: Vec<String> = Vec::new();

    if let Some(cmd) = std::env::var("CADFORGE_VISION")
        .ok()
        .filter(|c| !c.trim().is_empty())
    {
        match vision_command(&cmd, &body, q.w, q.h).await {
            Ok(mut inference) => {
                inference.source = format!("model: {cmd}");
                return Ok(Json(serde_json::to_value(inference).unwrap_or_default()));
            }
            Err(e) => notes.push(format!(
                "the configured vision model ({cmd}) did not answer, so the built-in 
                 geometric baseline was used instead: {e}"
            )),
        }
    }

    let mut inference = crate::vision::infer(&body, q.w, q.h).map_err(ApiError::bad_request)?;
    inference.notes.splice(0..0, notes);
    let mut value = serde_json::to_value(&inference).unwrap_or_default();
    if let Some(id) = &q.asset_id {
        value["asset_id"] = serde_json::json!(id);
    }
    Ok(Json(value))
}

/// Ask a configured vision model for the steps.
///
/// The pixels go to a temporary file, whose path is handed over in
/// `CADFORGE_IMAGE` along with the width and height; the model must print an
/// object shaped like `vision::Inference` (at minimum `steps` and `program`) as
/// JSON on stdout. That is the only contract, so a script, a Python process or
/// a compiled binary all plug in the same way.
async fn vision_command(
    cmd: &str,
    rgba: &[u8],
    w: usize,
    h: usize,
) -> Result<crate::vision::Inference, String> {
    let path = std::env::temp_dir().join(format!("cadforge-predict-{}.rgba", uuid::Uuid::new_v4()));
    std::fs::write(&path, rgba).map_err(|e| format!("could not stage the picture: {e}"))?;

    let (program, args): (&str, Vec<&str>) = if cfg!(windows) {
        ("cmd", vec!["/C", cmd])
    } else {
        ("sh", vec!["-c", cmd])
    };
    let out = tokio::process::Command::new(program)
        .args(args)
        .env("CADFORGE_IMAGE", &path)
        .env("CADFORGE_WIDTH", w.to_string())
        .env("CADFORGE_HEIGHT", h.to_string())
        .output()
        .await;
    let _ = std::fs::remove_file(&path);

    let out = out.map_err(|e| format!("could not run it: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "it exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    // A model that logs before it answers is normal; take the last JSON object.
    let text = String::from_utf8_lossy(&out.stdout);
    let start = text
        .rfind('{')
        .ok_or_else(|| "it printed no JSON".to_string())?;
    let candidate = &text[start..];
    let mut inference: crate::vision::Inference = serde_json::from_str(candidate)
        .map_err(|e| format!("its JSON did not match the expected shape: {e}"))?;
    inference.source = format!("model: {cmd}");
    Ok(inference)
}

/// The dataset in the form a trainer wants: every picture paired with the
/// operations that make what it shows, ready to be written to disk.
async fn export_dataset(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let manifest = {
        let db = app.db.lock().unwrap();
        crate::dataset::manifest(&app.root, &db, &id).map_err(ApiError::not_found_msg)?
    };
    Ok(Json(manifest))
}

#[derive(Deserialize)]
struct NewSource {
    path: String,
    #[serde(default)]
    label: String,
}

/// Point the studio at a folder of training data.
///
/// Nothing is copied: the folder is recorded and scanned, and its files become
/// artifacts that point back at where they live. Only the folder is checked
/// here - what it holds is the scan's business, and the scan reports as it goes.
async fn create_source(
    State(app): State<Arc<App>>,
    Json(body): Json<NewSource>,
) -> Result<Json<Source>, ApiError> {
    let raw = body.path.trim();
    if raw.is_empty() {
        return Err(ApiError::bad_request("a source needs a folder path"));
    }
    let path = std::path::Path::new(raw);
    if !path.is_absolute() {
        return Err(ApiError::bad_request(
            "give an absolute path, so the folder means the same thing every time it is scanned",
        ));
    }
    if !path.is_dir() {
        return Err(ApiError::bad_request(format!(
            "{} is not a folder",
            path.display()
        )));
    }
    let label = if body.label.trim().is_empty() {
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("source")
            .to_string()
    } else {
        body.label.trim().to_string()
    };

    let source = {
        let mut db = app.db.lock().unwrap();
        if db.sources.len() >= MAX_SOURCES {
            return Err(ApiError::bad_request(format!(
                "{MAX_SOURCES} folders is the limit - remove one first"
            )));
        }
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let wanted = canonical.to_string_lossy().to_string();
        if db.sources.iter().any(|s| s.path == wanted) {
            return Err(ApiError::bad_request("that folder is already a source"));
        }
        let source = Source {
            id: uuid::Uuid::new_v4().to_string(),
            path: wanted,
            label,
            state: SourceState::Idle,
            indexed: 0,
            skipped: 0,
            bytes: 0,
            error: None,
            last_scan: None,
            added_at: now(),
        };
        db.sources.push(source.clone());
        db.save(&app.root).map_err(ApiError::internal)?;
        source
    };

    // Index it straight away: pointing at a folder and seeing nothing happen
    // would be its own kind of lie.
    start_scan(app.clone(), source.id.clone());
    Ok(Json(source))
}

/// Remove a source and forget the artifacts it indexed. The files themselves
/// belong to the user and are left exactly where they are.
async fn delete_source(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mut db = app.db.lock().unwrap();
    if !db.sources.iter().any(|s| s.id == id) {
        return Err(ApiError::not_found("source"));
    }
    db.sources.retain(|s| s.id != id);
    let keep: Vec<Asset> = db
        .assets
        .iter()
        .filter(|a| a.source_id.as_deref() != Some(id.as_str()))
        .cloned()
        .collect();
    let dropped = db.assets.len() - keep.len();
    db.assets = keep;
    db.save(&app.root).map_err(ApiError::internal)?;
    drop(db);
    tracing::info!("source {id} removed; {dropped} artifact record(s) forgotten");
    Ok(StatusCode::NO_CONTENT)
}

/// Re-read a folder. This is also the answer to 'I have added files since'.
async fn scan_source(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<Json<Source>, ApiError> {
    let source = {
        let db = app.db.lock().unwrap();
        db.sources
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .ok_or_else(|| ApiError::not_found("source"))?
    };
    if source.state == SourceState::Scanning {
        return Err(ApiError::bad_request(
            "that folder is already being scanned",
        ));
    }
    start_scan(app.clone(), id);
    Ok(Json(source))
}

/// Run a scan off the request path. The reply is immediate and the progress
/// arrives on the work stream, because a folder of thousands of files would
/// otherwise hold the request open for minutes with nothing to show.
fn start_scan(app: Arc<App>, id: String) {
    tokio::task::spawn_blocking(move || crate::ingest::scan_worker(app, id));
}

/// What the data and the models weigh.
async fn storage(State(app): State<Arc<App>>) -> Json<serde_json::Value> {
    let report = {
        let db = app.db.lock().unwrap();
        ingest::storage_report(&app.root, &db)
    };
    Json(report)
}

/// Live ingest progress: folders being walked, files being read. One event per
/// file, so the interface can show work happening rather than a spinner.
async fn work_events(
    State(app): State<Arc<App>>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let rx = app.work_bus.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(work) => {
                let data = serde_json::to_string(&work).unwrap_or_default();
                Some(Ok(Event::default().event("work").data(data)))
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
        .route("/api/system", get(system))
        .route("/api/trainer", get(trainer))
        .route("/api/engines", get(engines))
        .route("/api/predict", post(predict))
        .route("/api/rebuild", post(rebuild))
        .route("/api/events", get(events))
        .route("/api/assets/events", get(asset_events))
        .route("/api/assets", post(upload_asset))
        .route("/api/assets/{id}", delete(delete_asset))
        .route("/api/assets/{id}/mesh", get(asset_mesh))
        .route("/api/assets/{id}/history", get(asset_history))
        .route("/api/assets/{id}/streams", get(asset_streams))
        .route("/api/assets/{id}/preview", get(asset_preview))
        .route("/api/assets/{id}/file", get(asset_file))
        .route("/api/models", post(create_model))
        .route("/api/models/{id}", delete(delete_model))
        .route("/api/datasets", post(create_dataset))
        .route("/api/datasets/{id}", delete(delete_dataset))
        .route("/api/datasets/{id}/export", get(export_dataset))
        .route("/api/runs", post(create_run))
        .route("/api/runs/{id}/cancel", post(cancel_run))
        .route("/api/sources", post(create_source))
        .route("/api/sources/{id}", delete(delete_source))
        .route("/api/sources/{id}/scan", post(scan_source))
        .route("/api/storage", get(storage))
        .route("/api/work/events", get(work_events))
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
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }
    fn not_found(what: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: format!("{what} not found"),
        }
    }
    fn not_found_msg(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }
    fn internal(e: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: e.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}
