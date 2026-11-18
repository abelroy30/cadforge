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

### 3.5 Geometry and CAD drivers
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
- Evaluation: given only a target mesh, generate steps, rebuild, compare (IoU,
  Chamfer distance, feature-by-feature match).

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
