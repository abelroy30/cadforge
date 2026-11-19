//! The training pipeline.
//!
//! Today it runs a built-in simulation so the whole loop is visible end to end:
//! point the studio at a base model and a dataset, start a run, watch the
//! stages advance, get a checkpoint out. The stage list is real, and a real
//! trainer plugs in without touching the rest of the app:
//!
//! * set `CADFORGE_TRAINER` to a shell command and each run executes it instead
//!   of the simulation, streaming its stdout into the run log;
//!   the run is handed to it as JSON in `$CADFORGE_RUN`.
//! * or implement [`Trainer`] in Rust and swap it in `App::trainer`.

use crate::model::{now, LogLine, RunStatus};
use crate::App;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};

/// One step of a training run.
pub struct Stage {
    pub label: &'static str,
    pub detail: &'static str,
    /// Share of the run's progress this stage accounts for.
    pub weight: f32,
}

pub trait Trainer: Send + Sync {
    fn name(&self) -> &str;
    fn stages(&self) -> Vec<Stage>;
}

/// The name a run reports for the trainer it will use.
pub fn trainer_name(external: &Option<String>) -> String {
    match external {
        Some(cmd) if !cmd.trim().is_empty() => format!("external: {cmd}"),
        _ => "cadforge-sim".to_string(),
    }
}

/// Default pipeline: the shape of the workflow the studio is meant to run.
pub struct DefaultTrainer;

impl Trainer for DefaultTrainer {
    fn name(&self) -> &str {
        "cadforge-sim"
    }

    fn stages(&self) -> Vec<Stage> {
        vec![
            Stage {
                label: "Resolve base model",
                detail: "load weights and architecture of the model to adapt",
                weight: 0.05,
            },
            Stage {
                label: "Register dataset",
                detail: "hash and index the CAD artifacts, images and video",
                weight: 0.10,
            },
            Stage {
                label: "Pair pictures with operations",
                detail: "match each view of a part with the operation program that makes it",
                weight: 0.10,
            },
            Stage {
                label: "Extract feature history",
                detail: "recover the operations behind each recorded shape",
                weight: 0.20,
            },
            Stage {
                label: "Tokenise operation sequences",
                detail: "encode sketch/extrude/fillet steps as model tokens",
                weight: 0.15,
            },
            Stage {
                label: "Fine-tune",
                detail: "adapt the small model onto the CAD operation language",
                weight: 0.30,
            },
            Stage {
                label: "Evaluate reconstruction",
                detail: "show a novel shape, ask for the steps, score the match",
                weight: 0.15,
            },
            Stage {
                label: "Save checkpoint",
                detail: "write the adapter and its training report",
                weight: 0.05,
            },
        ]
    }
}

/// Drive one run to completion, broadcasting every change to the UI.
pub async fn execute(app: Arc<App>, run_id: String) {
    let external = std::env::var("CADFORGE_TRAINER").ok();
    match external {
        Some(cmd) if !cmd.trim().is_empty() => external_run(app, run_id, cmd).await,
        _ => simulated_run(app, run_id).await,
    }
}

fn cancelled(app: &App, run_id: &str) -> bool {
    let db = app.db.lock().unwrap();
    db.runs
        .iter()
        .find(|r| r.id == run_id)
        .map(|r| r.status == RunStatus::Cancelled)
        .unwrap_or(true)
}

fn set_stage(app: &App, run_id: &str, stage: &str, progress: f32) {
    let mut db = app.db.lock().unwrap();
    if let Some(run) = db.runs.iter_mut().find(|r| r.id == run_id) {
        run.stage = stage.to_string();
        run.progress = progress.clamp(0.0, 1.0);
    }
}

fn log(app: &App, run_id: &str, level: &str, message: impl Into<String>) {
    let mut db = app.db.lock().unwrap();
    if let Some(run) = db.runs.iter_mut().find(|r| r.id == run_id) {
        run.log.push(LogLine {
            at: now(),
            level: level.to_string(),
            message: message.into(),
        });
    }
}

/// Persist the whole db and push the updated run to every open browser tab.
fn commit(app: &App, run_id: &str) {
    let snapshot = {
        let db = app.db.lock().unwrap();
        let _ = db.save(&app.root);
        db.runs.iter().find(|r| r.id == run_id).cloned()
    };
    if let Some(run) = snapshot {
        let _ = app.bus.send(run);
    }
}

/// How a dataset is actually made up: how many SolidWorks documents, how many
/// STEP files, how many pictures. Vision training needs the pictures and the
/// operations side by side, so this is the first thing a run should report.
fn composition(app: &App, run: &crate::model::Run) -> String {
    let db = app.db.lock().unwrap();
    let Some(ds) = db.datasets.iter().find(|d| d.id == run.dataset_id) else {
        return "the dataset is no longer in the studio".into();
    };
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for id in &ds.asset_ids {
        if let Some(asset) = db.asset(id) {
            *counts.entry(asset.lane.as_str()).or_default() += 1;
        }
    }
    if counts.is_empty() {
        return format!("{} holds no artifacts that are still present", ds.name);
    }
    let parts: Vec<String> = counts.iter().map(|(lane, n)| format!("{n} {lane}")).collect();
    format!("{}: {}", ds.name, parts.join(", "))
}

async fn simulated_run(app: Arc<App>, run_id: String) {
    let trainer = DefaultTrainer;
    {
        let mut db = app.db.lock().unwrap();
        if let Some(run) = db.runs.iter_mut().find(|r| r.id == run_id) {
            run.status = RunStatus::Running;
        }
    }
    log(
        &app,
        &run_id,
        "info",
        format!("trainer `{}` started", trainer.name()),
    );
    commit(&app, &run_id);

    let stages = trainer.stages();
    let total: f32 = stages.iter().map(|s| s.weight).sum();
    let mut done = 0.0f32;

    for stage in &stages {
        if cancelled(&app, &run_id) {
            log(&app, &run_id, "warn", "run cancelled");
            commit(&app, &run_id);
            return;
        }
        set_stage(&app, &run_id, stage.label, done / total);
        commit(&app, &run_id);
        log(
            &app,
            &run_id,
            "info",
            format!("{} - {}", stage.label, stage.detail),
        );
        if stage.label == "Register dataset" {
            // Read the run out under the lock, then describe the dataset once the
            // guard is gone. `composition` takes the same lock, and a
            // `std::sync::Mutex` taken twice on one thread does not block the
            // task - it deadlocks the whole server, so the two critical
            // sections must never overlap.
            let queue = {
                let db = app.db.lock().unwrap();
                db.runs.iter().find(|r| r.id == run_id).cloned()
            };
            if let Some(queue) = queue {
                let line = composition(&app, &queue);
                log(&app, &run_id, "info", line);
            }
        }

        // A handful of ticks so progress is visibly live, not a jump.
        for tick in 1..=4u32 {
            tokio::time::sleep(std::time::Duration::from_millis(180)).await;
            if cancelled(&app, &run_id) {
                log(&app, &run_id, "warn", "run cancelled");
                commit(&app, &run_id);
                return;
            }
            set_stage(
                &app,
                &run_id,
                stage.label,
                (done + stage.weight * tick as f32 / 4.0) / total,
            );
            commit(&app, &run_id);
        }
        done += stage.weight;
    }

    {
        let mut db = app.db.lock().unwrap();
        if let Some(run) = db.runs.iter_mut().find(|r| r.id == run_id) {
            run.status = RunStatus::Succeeded;
            run.stage = "Done".into();
            run.progress = 1.0;
            run.finished_at = Some(now());
            run.outputs.push(format!("checkpoints/{}.adapter", run.id));
        }
    }
    log(&app, &run_id, "info", "checkpoint written, run complete");
    commit(&app, &run_id);
}

async fn external_run(app: Arc<App>, run_id: String, cmd: String) {
    let (payload, dataset_id) = {
        let db = app.db.lock().unwrap();
        let run = db.runs.iter().find(|r| r.id == run_id).cloned();
        (
            serde_json::to_string(&run).unwrap_or_default(),
            run.map(|r| r.dataset_id).unwrap_or_default(),
        )
    };

    // Hand the trainer what it actually needs on disk: the dataset described as
    // picture/operation pairs, and a directory to write checkpoints into. This is
    // the whole interface between the studio and a training script.
    let out_dir = app.root.join("runs").join(&run_id);
    let _ = std::fs::create_dir_all(&out_dir);
    let dataset_file = out_dir.join("dataset.json");
    let described = {
        let db = app.db.lock().unwrap();
        crate::dataset::manifest(&app.root, &db, &dataset_id).ok()
    };
    match described.and_then(|m| serde_json::to_vec_pretty(&m).ok()) {
        Some(bytes) => match std::fs::write(&dataset_file, bytes) {
            Ok(_) => log(
                &app,
                &run_id,
                "info",
                format!("dataset described for the trainer at {}", dataset_file.display()),
            ),
            Err(e) => log(&app, &run_id, "warn", format!("could not write the dataset: {e}")),
        },
        None => log(
            &app,
            &run_id,
            "warn",
            "the dataset could not be described for the trainer",
        ),
    }
    {
        let mut db = app.db.lock().unwrap();
        if let Some(run) = db.runs.iter_mut().find(|r| r.id == run_id) {
            run.status = RunStatus::Running;
            run.stage = "External trainer".into();
        }
    }
    log(
        &app,
        &run_id,
        "info",
        format!("running external trainer: {cmd}"),
    );
    commit(&app, &run_id);

    // Windows has no `sh`; a trainer command has to go through cmd there.
    let shell = if cfg!(windows) { "cmd" } else { "sh" };
    let shell_flag = if cfg!(windows) { "/C" } else { "-c" };
    let child = tokio::process::Command::new(shell)
        .arg(shell_flag)
        .arg(&cmd)
        .env("CADFORGE_RUN", &payload)
        .env("CADFORGE_DATASET", &dataset_file)
        .env("CADFORGE_OUT", &out_dir)
        .env("CADFORGE_DATA", &app.root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            log(
                &app,
                &run_id,
                "error",
                format!("could not start trainer: {e}"),
            );
            finish(&app, &run_id, RunStatus::Failed);
            return;
        }
    };

    if let Some(out) = child.stdout.take() {
        let mut lines = BufReader::new(out).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log(&app, &run_id, "info", line);
            commit(&app, &run_id);
        }
    }
    if let Some(err) = child.stderr.take() {
        let mut lines = BufReader::new(err).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log(&app, &run_id, "warn", line);
            commit(&app, &run_id);
        }
    }

    let ok = child.wait().await.map(|s| s.success()).unwrap_or(false);
    finish(
        &app,
        &run_id,
        if ok {
            RunStatus::Succeeded
        } else {
            RunStatus::Failed
        },
    );
}

fn finish(app: &App, run_id: &str, status: RunStatus) {
    {
        let mut db = app.db.lock().unwrap();
        if let Some(run) = db.runs.iter_mut().find(|r| r.id == run_id) {
            run.status = status.clone();
            run.progress = 1.0;
            run.stage = if status == RunStatus::Succeeded {
                "Done".into()
            } else {
                "Failed".into()
            };
            run.finished_at = Some(now());
        }
    }
    log(
        &app,
        run_id,
        if status == RunStatus::Succeeded {
            "info"
        } else {
            "error"
        },
        "trainer finished",
    );
    commit(app, run_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BaseModel, Dataset, Db, Run, RunStatus};
    use std::sync::Mutex;

    /// An app with a private data directory, so a test never touches the
    /// studio's own db.
    fn app(dir: &std::path::Path) -> Arc<App> {
        let (bus, _) = tokio::sync::broadcast::channel(64);
        let (asset_bus, _) = tokio::sync::broadcast::channel(64);
        Arc::new(App {
            root: dir.to_path_buf(),
            db: Mutex::new(Db::default()),
            bus,
            asset_bus,
        })
    }

    /// A run used to stop dead between stages: the "Register dataset" stage
    /// described the dataset while still holding the db lock, and describing it
    /// takes that same lock again. The task did not fail - it froze, taking
    /// every request with it - so this test asserts the run gets all the way to
    /// the end rather than merely not erroring.
    #[tokio::test]
    async fn a_run_finishes_and_says_what_the_dataset_is_made_of() {
        let dir = std::env::temp_dir().join(format!("cadforge-run-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let app = app(&dir);

        let (ext, kind) = crate::model::classify("plate.step");
        let lane = crate::model::lane_for(&ext, &kind).to_string();
        let asset = crate::model::Asset {
            id: "a1".into(),
            name: "plate.step".into(),
            ext,
            kind,
            size: 10,
            uploaded_at: crate::model::now(),
            lane,
            triangles: None,
            bbox: None,
            header: None,
            step: None,
            catalogue: None,
            actions: Vec::new(),
        };
        {
            let mut db = app.db.lock().unwrap();
            db.assets.push(asset);
            db.models.push(BaseModel {
                id: "m1".into(),
                name: "small".into(),
                kind: "base".into(),
                params: Some(1_000),
                notes: String::new(),
                created_at: crate::model::now(),
            });
            db.datasets.push(Dataset {
                id: "d1".into(),
                name: "shop".into(),
                asset_ids: vec!["a1".into()],
                created_at: crate::model::now(),
            });
            db.runs.push(Run {
                id: "r1".into(),
                name: "run".into(),
                base_model_id: "m1".into(),
                dataset_id: "d1".into(),
                status: RunStatus::Queued,
                stage: "Queued".into(),
                progress: 0.0,
                log: Vec::new(),
                created_at: crate::model::now(),
                finished_at: None,
                outputs: Vec::new(),
            });
        }

        let finished = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            execute(app.clone(), "r1".into()),
        )
        .await;
        assert!(finished.is_ok(), "the run never finished - it deadlocked");

        let db = app.db.lock().unwrap();
        let run = db.runs.iter().find(|r| r.id == "r1").unwrap();
        assert_eq!(run.status, RunStatus::Succeeded, "stalled at {}", run.stage);
        assert_eq!(run.progress, 1.0);
        let messages: Vec<&str> = run.log.iter().map(|l| l.message.as_str()).collect();
        assert!(
            messages.iter().any(|m| m.starts_with("shop:")),
            "the run should say what the dataset holds, got {messages:?}"
        );
        assert!(
            messages.iter().any(|m| m.contains("run complete")),
            "the run should reach its checkpoint, got {messages:?}"
        );
        assert!(run.finished_at.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
