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
    let payload = {
        let db = app.db.lock().unwrap();
        let run = db.runs.iter().find(|r| r.id == run_id).cloned();
        serde_json::to_string(&run).unwrap_or_default()
    };
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

    let child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .env("CADFORGE_RUN", &payload)
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
