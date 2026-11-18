# cadforge — architecture

A CAD-shaped web studio whose job is to train small models that **generate
parametric CAD**: you show it shapes (and the operations that made them), it
learns to produce the operations for a new shape you only show the result of.

This document is the plan. `README.md` is how to run what exists today.

---

## 1. What exists today (walking skeleton)

The whole loop is wired end to end, with the modelling intelligence stubbed:

| Piece | State |
|---|---|
| Rust backend (`axum`) + JSON API | working |
| CAD-like web GUI with a 3D viewport | working |
| Artifact upload: STL, STEP, images, video | working |
| STL parser (ASCII + binary) -> triangle mesh + bounds | working |
| STEP header reader (schema, product name) | working |
| Base-model registry (base or user-defined architecture) | working |
| Datasets built from selected artifacts | working |
| Training pipeline with 7 real stages, live progress + log over SSE | working (simulated) |
| External trainer hook (`CADFORGE_TRAINER`) | working |
| STEP (ISO 10303-21) reader: B-rep topology, units, entity graph | working |
| Feature history read back off the B-rep (extrude, holes, patterns, fillets) | working |
| STEP preview: planar faces triangulated, holes drawn as sampled strips | working |
| Operation program (versioned, machine-readable build steps) | working |
| Rebuild driver: native engine executes a program to geometry | working |
| FreeCAD engine: emits a macro, runs it where freecadcmd exists | working (untested: no FreeCAD here) |
| Scoring: symmetric Chamfer distance between a rebuild and a target | working |
| SolidWorks-native history, B-spline surfaces, real fine-tuning | **not yet** |

## 2. The loop we are building

```
  upload parts            build dataset           run pipeline           ask for a new shape
  (STL/STEP/img/video) -> (artifacts + history) -> (fine-tune small LLM) -> (model emits CAD steps)
         ^                                                                          |
         |                                                                          v
         +---------------------- correct / score <---------------------- rebuild in CAD kernel
```

## 3. Components

```
 browser ── HTTP/JSON + SSE ──► cadforge (Rust, axum)
                                    │
              ┌─────────────────────┼─────────────────────────┐
              │                     │                         │
      artifact store          run orchestrator           model registry
      data/assets/*          pipeline.rs (stages)        db.json
              │                     │
              │                     ├── external trainer ──► Python (torch/peft) training
              │                     │                        │
              │                     └── outputs ────────► adapters / checkpoints
              │
      geometry services (next)
              ├── mesh: STL/OBJ/3MF            (pure Rust, started)
              ├── B-rep + history: STEP/STP    (OpenCASCADE via opencascade-rs)
              └── CAD driver: SolidWorks (VBScript/COM), FreeCAD (Python)
```

### 3.1 Web GUI (`static/`)
Deliberately shaped like CAD: a tree on the left (Artifacts, Base models,
Datasets, Runs), a 3D viewport in the middle with a shaded/wire/both toggle and
orbit camera, an inspector + training console on the right. Searchable part
tree, feature-history timeline and a command line are the obvious next adds.

### 3.2 API (`src/api.rs`)

| Route | Purpose |
|---|---|
| `GET /api/state` | everything the GUI needs on load |
| `GET /api/trainer` | pipeline name + stages (drives the UI checklist) |
| `GET /api/events` | SSE stream of every run change |
| `POST /api/assets?name=` | upload an artifact (raw body, no multipart) |
| `GET /api/assets/{id}/mesh` | triangle soup for the viewport |
| `GET /api/assets/{id}/file` | original bytes back |
| `DELETE /api/assets/{id}` | remove an artifact |
| `POST /api/models` | register a base model or a custom architecture |
| `POST /api/datasets` | create a dataset from artifact ids |
| `POST /api/runs` | start a training run |
| `POST /api/runs/{id}/cancel` | cancel a run |

### 3.3 Artifact store (`src/model.rs`)
`data/db.json` holds assets, models, datasets and runs; original bytes live in
`data/assets/<id>.<ext>`. Written atomically (tmp + rename). Fine for a single
node; Postgres + object storage when this moves to the cloud.

### 3.4 Pipeline (`src/pipeline.rs`)
The seven stages are the real workflow, not filler:

1. Resolve base model
2. Register dataset
3. Extract feature history — recover the operations behind each recorded shape
4. Tokenise operation sequences — sketch/extrude/fillet as model tokens
5. Fine-tune
6. Evaluate reconstruction — show a novel shape, ask for the steps, score it
7. Save checkpoint

**Plugging in a real trainer.** Two seams, no app changes needed:

* `CADFORGE_TRAINER="python train.py"` — each run executes that command with the
  run handed to it as JSON in `$CADFORGE_RUN`; stdout/stderr stream into the run log.
* Implement the `Trainer` trait in Rust and swap it into `pipeline::execute`.

### 3.5 The rebuild drivers, and the loop they close

An inferred history is prose; a model needs something it can be trained on and
scored against. `src/ops.rs` is that contract: a versioned `Program` — a list of
`extrude`, `hole`, `hole_pattern`, `fillet` operations with real numbers.

```
STEP file ──step::to_program──► Program ──┬── solid::evaluate (native) ──► triangles ──┐
                                          │                                           ├─► score::compare
                                          └── freecad::script/run    ──► triangles ──┘
```

* `src/solid.rs` — the native engine. It builds the same *surface model* the STEP
  file carries (outer faces plus hole walls) rather than doing booleans, and it
  says so when it cannot do something, like a fillet.
* `src/freecad.rs` — emits a real FreeCAD Python macro for the same program, and
  runs it when `freecadcmd` is on PATH. This is the engine that can genuinely
  cut, fillet and export; it is the intended production backend.
* `src/score.rs` — samples points on both surfaces and takes the mean
  nearest-neighbour distance in both directions. Symmetric on purpose: leaving a
  hole out is penalised, and so is inventing material.

The round trip is a test, not a demo: `rebuilds_the_plate_from_its_own_history`
reads a part, rebuilds it from its own inferred program, and fails if the two
surfaces drift apart. `a_rebuild_that_loses_the_holes_scores_worse` guards the
metric itself.

### 3.6 Requirements added later

* **Bulk upload.** Point it at a folder of SolidWorks files, not one at a time.
* **Categorise parts, assemblies and drawings** — the ask was explicitly via the
  SolidWorks feature tree.
* **Learn the build story per file**, and later: given a photo of a part, work out
  how it was made.
* **Runs locally on the user's machine.** Nothing may depend on a cloud service.
* **Deployed to Windows**, browser-based UI.
* **Pushed to GitHub** and reusable on any computer.

### 3.7 Geometry and CAD drivers
* **Mesh** (STL, OBJ, 3MF): in-process Rust, `src/stl.rs`. Working.
* **STEP history** (STEP/STP): `src/step.rs` is a dependency-free ISO 10303-21
  reader - tokenizer, entity graph, topology counts, units - plus an inference
  pass that reads the *build steps* back out of the B-rep: two parallel planes
  are an extrude, a cylindrical face is a hole, repeated holes on one axis are
  a pattern, a toroidal face is a fillet. Working, and covered by tests against
  generated parts.
* **Still missing here**: B-spline/advanced surfaces, true face trimming, and
  the SolidWorks-native feature tree (which lives in the `.SLDPRT`, not in the
  exchanged file). `opencascade-rs` is the intended route for the heavy B-rep
  work once preview fidelity matters more than zero dependencies.
* **Drivers**: SolidWorks via its COM API (VBScript/PowerShell on Windows, or a
  Windows worker), FreeCAD headless via Python. Both emit a script plus the
  resulting geometry so the model can be scored against the target.

## 4. Roadmap

**Phase 1 — see real parts (now).** Done: upload, viewport, pipeline shell, live runs.

**Phase 2 — learn the operation language.** *(in progress)*
- ~~Read STEP and recover the build steps from the B-rep.~~ Done: `src/step.rs`,
  with `tools/make_step.py` generating parts to test against.
- Define the operation token set (sketch, constraint, extrude, cut, fillet, chamfer,
  pattern, boolean, mate) as a versioned schema, and map the inferred steps onto it.
- A converter that turns that history into token sequences, and back into a script.
- FreeCAD driver so a generated sequence can be executed and rebuilt.

**Phase 3 — train the small model.**
- Dataset format on disk (features + history + optional renders).
- Python trainer (LoRA/QLoRA on a small open base, or the user's architecture)
  behind the `CADFORGE_TRAINER` hook, reporting progress back per stage.
- ~~Evaluation: given a target, rebuild and compare.~~ Done for one route: the
  operation program is the prediction, the rebuild driver executes it, and
  `score::compare` measures the result. What is still missing is the model that
  produces the program in the first place.

**Phase 2.5 — bulk intake, and SolidWorks.** *(new; the user's own priority)*
- Point the studio at a folder or a zip; upload many files in one go.
- Classify `.SLDPRT` / `.SLDASM` / `.SLDDRW` by file signature, then by content.
- Pull the build story out of a native SolidWorks file. Honest position: the
  feature tree lives in a proprietary stream inside an OLE compound document.
  Achievable without SolidWorks: document type, custom properties, embedded
  feature-name strings, timestamps. Full fidelity needs the SolidWorks API —
  which the user will have, on Windows, where a COM exporter can dump the true
  feature tree and rebuild order as structured text. That exporter is the
  highest-value Windows-side work.
- Keep everything local: no cloud calls, no telemetry, no CDN. Already true.

**Phase 4 — SolidWorks and the wider inputs.**
- Windows worker running the SolidWorks driver; job queue.
- Image/video conditioning: a photo or a video of a part becomes a starting shape.

**Phase 5 — make it a product.**
- Users, projects, queues, GPU scheduling, artifact versioning, multi-tenant storage.

## 5. Decisions taken

* **Rust backend, JavaScript GUI.** Rust for the server, parsers and orchestration;
  the browser needs JS anyway.
* **No build step for the GUI.** Plain modules + vendored three.js, so the first
  version has zero toolchain friction. A bundler comes when the UI grows.
* **Simulated pipeline first.** The stages and the live-progress plumbing are the
  contract; a real trainer drops in behind them later.
* **Tolerant parsers.** STLs in the wild have wrong normals and truncated data;
  we keep what is usable instead of refusing the file.

## 6. Open questions

1. Which base model, and does the architecture stay yours end to end?
2. Which format carries the SolidWorks history — STEP AP214/AP242, or a native
   export through the API?
3. Where does training run: this machine, or a GPU box the studio submits jobs to?
4. Rebuild target: FreeCAD first (free, scriptable, Linux-friendly) then
   SolidWorks, or SolidWorks first because that is where the history comes from?

## 7. Known issues

* `score::compare` reports a `max_gap` of 7.77 mm on the generated plate even
  though the chamfer distance is 0.02 mm and every triangle vertex matches
  exactly. 539 of the 544 surface sample points are identical between target and
  rebuild; the 5 that differ are triangle *centroids* belonging to different but
  equivalent triangulations of the same quads. The geometry is therefore right,
  and the suspicion sits in the sample set or the nearest-neighbour search in
  `src/score.rs`. Unresolved. It matters because this distance is the training
  signal - a phantom outlier would teach the model to chase nothing.
