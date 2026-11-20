# cadforge — file glossary

What every file in this repository is for, and how the files depend on each
other. For the words, see `CONCEPTS.md`; for how to drive it, see `USAGE.md`.

---

## 1. Entry point and shared state

### `src/main.rs`
The crate root and the binary. Declares all 17 modules, parses the options,
loads the data directory and the database, builds the shared `App`, mounts the
API router plus static file serving, and serves.

* `pub struct App { root, db: Mutex<Db>, bus: broadcast::Sender<Run>, asset_bus: broadcast::Sender<Asset> }`
  — the two broadcast channels are what the two SSE streams are fed from.
* CLI: `--port`/`-p` (8787), `--data`/`-d` (`data`), `--static`/`-s` (`static`),
  `--emit-samples`/`--samples <dir>`, `--help`/`-h`. Parsed by hand —
  `Options::from_args()`, no clap, no dependency.
* `fn open_browser_soon(url)` — waits 450 ms, then `cmd /C start`, `open`, or
  `xdg-open` depending on the platform.
* `fn emit_samples(dir)` — writes `bracket.SLDPRT`, `plate.SLDPRT`, `rig.SLDASM`,
  `sheet.SLDDRW` built from `ole::fixture` and `propset::fixture`, so the whole
  pipeline can be seen working without a SolidWorks licence.

**Depends on:** `model`, `api`, `ole`, `propset`.

### `src/model.rs`
The domain schema every other module shares, plus the single-document store.

* `AssetKind` — `Mesh, Step, SwPart, SwAssembly, SwDrawing, History, Image, Video, Other`.
* `Asset` — the file plus what was read out of it (`kind`, `lane`, `size`,
  `triangles`, `bbox`, `header`, `step: Option<step::Summary>`,
  `catalogue: Option<sldworks::Catalogue>`, `actions: Vec<Action>`).
* `Action { step, detail, children }` — one node of the inspector's
  "how this was processed" tree.
* `BaseModel { id, name, kind, params, notes, created_at }` — a model you point
  the studio at (a base checkpoint or your own architecture).
* `Dataset { id, name, asset_ids, created_at }` — a named set of artifact ids.
* `Run { id, name, base_model_id, dataset_id, status, stage, progress, log, created_at, finished_at, outputs }`
  and `RunStatus { Queued, Running, Succeeded, Failed, Cancelled }`.
* `LogLine { at, level, message }`.
* `Db { assets, models, datasets, runs }` — `Db::load`, `Db::save` (tmp + rename),
  `Db::asset`.
* Path helpers: `db_path`, `assets_dir`, `preview_path`; time helper `now()`.
* `classify(name)` → `(ext, AssetKind)` and `lane_for(ext, kind)` → the default
  lane. This is where extension → kind → lane is decided.

**Depends on:** `step`, `sldworks`.

## 2. HTTP surface

### `src/api.rs`
The whole HTTP API as one axum router, plus one error type.

| Route | Handler | Purpose |
|---|---|---|
| `GET /api/state` | `state` | the whole `Db`, everything the UI needs on load |
| `GET /api/system` | `system` | GPU, Python stack, configured trainer and vision command, advice |
| `GET /api/trainer` | `trainer` | pipeline name and stage list (drives the UI checklist) |
| `GET /api/engines` | `engines` | which rebuild engines are available |
| `POST /api/predict` | `predict` | raw RGBA picture → build steps |
| `POST /api/rebuild` | `rebuild` | operation program → geometry + score |
| `GET /api/events` | `events` | SSE stream of run changes |
| `GET /api/assets/events` | `asset_events` | SSE stream of artifact changes |
| `POST /api/assets` | `upload_asset` | upload one artifact (`?name=&lane=`, raw body) |
| `DELETE /api/assets/{id}` | `delete_asset` | remove an artifact and its files |
| `GET /api/assets/{id}/mesh` | `asset_mesh` | triangle soup for the viewport |
| `GET /api/assets/{id}/history` | `asset_history` | reconstructed history + preview |
| `GET /api/assets/{id}/streams` | `asset_streams` | stream catalogue of a document |
| `GET /api/assets/{id}/preview` | `asset_preview` | extracted preview image bytes |
| `GET /api/assets/{id}/file` | `asset_file` | the original bytes back |
| `POST /api/models` | `create_model` | register a base model |
| `DELETE /api/models/{id}` | `delete_model` | remove one (refused while a run uses it) |
| `POST /api/datasets` | `create_dataset` | create a dataset from artifact ids |
| `DELETE /api/datasets/{id}` | `delete_dataset` | remove one (refused while a run uses it) |
| `GET /api/datasets/{id}/export` | `export_dataset` | the trainer manifest |
| `POST /api/runs` | `create_run` | queue a run (spawns `pipeline::execute`) |
| `POST /api/runs/{id}/cancel` | `cancel_run` | cancel a queued or running run |

* Uploads are a raw body with the name in the query string — no multipart, so the
  browser can stream a file straight through. Body limit is 512 MiB.
* `ApiError { status, message }` is the single error shape; it always renders as
  `{ "error": "..." }`.

**Depends on:** nearly everything — `model`, `pipeline`, `stl`, `step`,
`sldworks`, `swhist`, `ops`, `freecad`, `solid`, `score`, `dataset`, `system`,
`vision`.

## 3. Geometry core (no CAD kernel, no dependencies)

### `src/vec3.rs`
`pub type V3 = [f64; 3]` and the handful of helpers the geometry code needs:
`add`, `sub`, `scale`, `dot`, `cross`, `len`, `normalize`, `perpendicular`,
`push_tri`. No tests.

### `src/stl.rs`
Minimal STL reader, ASCII and binary. `Mesh { positions, triangles, bbox }`,
`parse(bytes)`. Discards facet normals and caps at `MAX_TRIANGLES = 250_000` —
an upper bound on what is handed to the browser in one response.

### `src/step.rs` (the largest module, 1305 lines)
A dependency-free ISO 10303-21 reader and history reconstructor.

* Parsing: `StepFile`, `Entity`, `Value`, `StepFile::parse/get/deref/named/count`.
* Geometry: `point_of`, `direction_of`, `placement_of`, `loop_points`,
  `vertex_point`, `triangulate_planes`, `triangulate_cylinders`.
* History: `analyze`, `analyze_file` → `Analysis { summary, triangles, points }`,
  with `Summary { solids, shells, faces, planar_faces, cylindrical_faces,
  toroidal_faces, vertices, edges, points, unit, steps }` and
  `StepLine { op, detail }`.
* `to_program(file)` → `crate::ops::Program`, the executable form.
* Inference rules live here: two parallel planes are an extrude, a cylindrical
  face is a hole, repeated holes on one axis are a pattern, a toroidal face is a
  fillet. When nothing is recognised it emits an honest line: *"no buildable
  features recognised - the file may be a surface model or an unsupported
  flavour"*.
* Its own limits are documented: planar faces are fan-triangulated from the
  outer loop's centroid ("good enough to see the part"), and the program's
  profile is a convex hull, so a concave footprint is approximated.

### `src/solid.rs`
The native engine: executes a program into a surface model without a CAD kernel.
`Face::Planar` / `Face::Cylindrical`, `Solid`, `Rebuild { solid, warnings,
ops_applied }`, `evaluate(program)`, `convex_hull_2d` (Andrew's monotone chain,
used to turn a point cloud into a profile outline). Drills holes at
`SEGMENTS = 32`. It does not do boolean subtraction and says so; fillets come
back as a warning, not a silent omission. No tests of its own.

### `src/ops.rs`
The operation program: `VERSION: u32 = 1`, `Program { version, units, ops }`,
`Op` and `Profile`, `Program::validate`, `Program::op_names`. The ops enum is
serde-tagged with `"op"` and the profile with `"shape"`, both snake_case. See
`CONCEPTS.md` for the exact JSON. No tests of its own.

### `src/score.rs`
The metric: `compare(rebuilt, target) -> Option<Score>` with
`Score { chamfer, forward, backward, max_gap, samples, size_delta }`. Samples four
points per triangle (three corners and the centroid) and uses a spatial `Grid`
for nearest-neighbour search. Symmetric on purpose.

### `src/freecad.rs`
The production engine: `script(program)` emits a real FreeCAD Python macro;
`run(...)` executes it where `freecadcmd` is on PATH, and errors with *"FreeCAD is
not installed on this machine (no freecadcmd on PATH)"* when it is not. Macro
generation is unit-tested; the actual invocation is not, because there is no
FreeCAD in the test environment.

## 4. CAD-native readers (no SolidWorks installed)

### `src/ole.rs`
A reader for the Compound File Binary Format — the small filesystem inside a
`.sldprt`. `OleFile::parse`, `entries()`, `Entry::kind`, `looks_like_ole`,
`sector_size`, `read`. It understands the *container* only.

It can also **write** compound files: `pub mod fixture` provides
`fixture::build(streams)`. That is how the reader is tested without a SolidWorks
licence, and it is what the studio's sample documents are built from.

### `src/propset.rs`
Reads OLE property-set streams (`SummaryInformation`,
`DocumentSummaryInformation`) — the one place a document keeps parameters in a
documented format. `PropertySet::parse`, `groups_for`, plus a `fixture` writer
for tests. Reads title, author, dates, and the custom name/value properties a
shop fills in (Material, Mass, Part Number, Revision). "What this does not do is
invent meaning": a property is reported with the name the file gives it.

### `src/sldworks.rs`
Opens a `.sldprt`/`.sldasm`/`.slddrw` and reports what can honestly be read:

* **the stream catalogue** — every stream with size and role, including
  unrecognised ones ("nothing is hidden");
* **the parameters** — via `propset`;
* **the references** — an assembly's components or a drawing's model, found by
  reading printable text runs in the streams (`scan_references`, capped by
  `MAX_REFERENCES`);
* **the embedded preview** — sniffed by magic bytes (`sniff_image`);
* **the document kind** — from the extension, and it says so when it cannot tell.

Recognised stream names: `Preview`, `PreviewPNG`, `ThirdPv`, `Contents`,
`SwDocContentMgr`, `SummaryInformation`, `DocumentSummaryInformation`,
`DisplayLists`. Types: `StreamInfo`, `PreviewInfo`, `Reference`, `Catalogue`,
`Preview`, `Inspected`, `inspect`.

**It does not decode the feature tree.** That encoding is proprietary, and the
module states that guessing "would produce confidently wrong build histories -
the one thing worse than none". The stream is listed and named instead, with a
runtime note saying exactly that.

### `src/swhist.rs`
Reads the JSON written by the companion SolidWorks macro
(`tools/solidworks/ExportFeatureTree.bas`) and converts it into the same
`Summary` the STEP reader produces — so a *recorded* history and an *inferred*
one travel the same path. `Feature`, `Export`, `parse`, `summary`, `describe`,
`render`. An export that parses but is empty yields an empty history, so the
caller can say "this export was empty" rather than "this is corrupt".

## 5. Learning glue

### `src/dataset.rs`
Turns a dataset into training pairs and one JSON manifest — the only thing a
trainer needs to know about the studio.

* `VIEW_SUFFIXES` = `front, back, left, right, top, bottom, iso, sheet, view`.
* `base_name(name)` — strips the extension and one trailing view word. This is
  the pairing rule.
* `manifest(dir, db, dataset_id)` — builds the whole manifest: `dataset`,
  `dataset_id`, `version`, `pairs`, `counts`, `documents_without_a_picture`,
  `notes`.
* `target_for(dir, doc)` — decides what a document can teach, producing the
  `target` label (`reconstructed_from_geometry`, `recorded_steps`,
  `parameters_only`, `unpaired`, `unreadable`).
* `split_of(id, val_pct)` — FNV-1a over the artifact id; `val` when
  `hash % 100 < val_pct`. Stable by construction.

### `src/pipeline.rs`
One training run, end to end.

* `Trainer` trait (`name()`, `stages()`) and `DefaultTrainer` — the eight stages
  with their weights.
* `execute(app, run_id)` — reads `CADFORGE_TRAINER` and picks the simulated or
  the external path.
* `simulated_run` — walks the stages with ticks so progress is visibly live, and
  logs what the dataset is made of and how the pairs split.
* `external_run` — writes the manifest to `runs/<id>/dataset.json`, spawns the
  command through the platform shell with `CADFORGE_RUN` / `CADFORGE_DATASET` /
  `CADFORGE_OUT` / `CADFORGE_DATA`, streams stdout into the log and stderr as
  warnings, then diffs the output directory with `tree()` and records what the
  trainer produced as run outputs.
* `set_stage` / `log` / `commit` — every change is saved to the database and
  broadcast on the run bus, which is how the UI updates live.
* Tests: `a_run_finishes_and_says_what_the_dataset_is_made_of` (a run must reach
  the end rather than deadlock) and `an_external_trainer_output_is_reported_by_the_run`.

### `src/vision.rs`
Picture → build steps, as a geometric baseline. `infer(rgba, width, height)`
returns `Inference { source, width, height, steps, program, confidence,
rectangularity, holes, notes }`.

It measures the silhouette (alpha, or luma against the corner average), the
bounding box, `rectangularity` (area ÷ box area), and enclosed holes by
flood-fill from the frame edge — ignoring specks under 9 px as anti-aliasing.
Holes of similar diameter become one `hole_pattern`; different diameters stay
individual `hole` ops. Similar diameter means within `max(25%, 0.5)`.

Two stated assumptions, and it states them in the reply: size is proportional
(the longest side is taken as 100 mm) and depth is *assumed* at a quarter of the
shorter side. It does not measure symmetry.

### `src/system.rs`
A read-only probe of the machine: `SystemInfo { os, gpu, python, trainer,
external_trainer, vision_command, advice }`, with `Gpu { present, name,
vram_mb, driver }` and `Python { present, command, version, torch, cuda }`.
Shells out to `nvidia-smi --query-gpu=...` and tries `$CADFORGE_PYTHON`, then
`python`, `python3`, `py` (`py -3`), running `--version` and a torch probe.
Advice strings cover the missing-GPU, low-VRAM, missing-torch and
CUDA-not-visible cases. "Nothing is installed, nothing is downloaded, and no
probe leaves the computer." A missing tool is information, not an error.

### `src/ingest.rs`

Turning a file into an artifact, from either front door. An upload copies its
bytes in; a folder source reads them where they live and copies nothing. Both go
through the same `inspect`, so an artifact cannot mean two different things
depending on how it arrived - which is also why the inspection lives here rather
than in the API layer.

Key items: `WorkEvent` (one step of progress, for the live feed), `inspect`,
`upload`, `read_referenced`, `scan_worker`, `walk`, `dir_bytes`, `human_bytes`,
`actions_for`, `storage_report`.

`walk` skips hidden and build directories, never follows a symlink (so a link
pointing up its own tree cannot loop) and sorts what it finds, so a re-scan is
deterministic.

`storage_report` reports two kinds of number differently on purpose: the size of
the data is *measured*, while a base model's size is *estimated* from its
parameter count, because the studio never downloads weights. Adapter sizes are
measured from what runs wrote.

### `src/reward.rs`

The reward a training run is paid with. Four kinds - `geometry`,
`recorded_history`, `simplicity`, `scale` - plus the rules that keep them honest:
validity is a gate rather than a weighted term, and a reward with nothing to
compare against abstains and says so.

Key items: `KINDS`, `Target`, `Component`, `Verdict`, `evaluate`, `defaults`,
`ensure_defaults`, `spec_for_manifest`, `preview`.

`Target` is the truth in pieces: a picture gives no geometry, a STEP part gives
geometry but no recorded history, and a SolidWorks export gives the history but
nothing to rebuild against. Each reward takes what it needs and abstains
otherwise. The `Source` and `Reward` types themselves live in `src/model.rs`
with the rest of the persisted state.

**Deployment** - src/model.rs

One record saying which run is answering, with the adapter path and the base model by name. One at a time by construction. Routes: POST /api/runs/{id}/deploy, GET and DELETE /api/deployment, which predict reads after CADFORGE_ADAPTER.

## 6. Web front end (`static/`)

### `static/index.html`
The page shell: an inline SVG sprite of nine symbols (all icons are local SVG,
no icon font, no external image), a `<canvas id="scene">`, and three panels. All
content is injected at runtime by `app.js`.

| Region | Ids |
|---|---|
| top bar | `trainer-pill`, `conn-pill` |
| viewport | `viewport`, `scene`, `vp-toolbar` (`.tool[data-view=shaded\|wire\|both]`), `btn-fit`, `btn-grid`, `btn-capture`, `btn-drawing`, `view-strip`, `readout`, `dropzone` |
| upload tiles | `lane-sldworks`, `lane-step`, `lane-image` |
| processing | `activity-list`, `btn-clear-activity` |
| artifacts | `asset-filters` (`data-filter`), `asset-list`, `btn-dataset` |
| datasets / runs | `dataset-list`, `run-list` |
| inspector | `inspector-title`, `props` |
| picture to steps | `vision-drop`, `vision-canvas` (256×256), `vision-run`, `vision-status`, `vision-result` |
| base models | `btn-model`, `model-list` |
| training | `run-pill`, `system-status`, `btn-refresh-system`, `sel-model`, `sel-dataset`, `btn-run`, `run-bar`, `run-stage`, `btn-cancel-run`, `stage-list`, `run-log` |
| hidden inputs | `file-input` (multiple), `vision-input` |

Scripts load in order: `/vendor/three.min.js`, `/vendor/OrbitControls.js`,
`/app.js` — all same-origin. The symbol `#i-upload` is defined but never used.

### `static/app.js` (~1170 lines)
The entire client. No modules, no bundler, no type checking.

* Entry: `main()` → `initViewport`, `wire`, `wireFilters`, `setupVision`,
  `connectEvents`, `refresh`.
* Server access: `api(path, opts)` (JSON helper), `postAsset(name, lane, body)`.
* Viewport: `initViewport` (WebGLRenderer, camera 45°, hemisphere + two
  directional lights, `GridHelper`, `partGroup`), **OrbitControls** with damping,
  `ResizeObserver` → `resizeViewport`, `renderLoop`, `setPart`, `setPoints`,
  `buildPart` (wire/shaded/both), `fitView`.
* Lists: `renderAssets`, `renderModels`, `renderDatasets`, `renderRuns`,
  `renderPickers`, `renderActivity`.
* Inspector: `selectAsset`, `renderProps`, `renderStep`, `renderActions`,
  `actionTree`, `renderCatalogue`, `renderRebuild`.
* Uploads: `uploadFiles(files, lane)` with `UPLOADS` and three concurrent
  workers, `laneFor`, `markUploaded`.
* Capture: `captureViews('views'\|'drawing')`, `renderViews` (a second offscreen
  renderer with alpha, 512 px per view), `composeSheet`, `appendThumb`.
* Vision: `setupVision`, `loadVisionImage`, `drawVisionImage` (letterboxes into
  256×256), `runVision`, `confidenceMeter`, `renderVision`.
* Runs: `showRun`, `paintStages`, `renderTrainer`, `renderSystem`, `sysLine`.
* Live updates: `connectEvents()` opens **two** `EventSource`s — `/api/events`
  (drives the connection pill and run updates) and `/api/assets/events` (fills
  the artifact list as uploads land).

### `static/app.css`
A dark theme built on one accent: `--bg #0f1216`, `--accent #4f8cff` for every
action and selection, and `--ok/--warn/--err` kept for status. A three-column
grid (`300px 1fr 380px`). No CDN, no web fonts, no images, no media queries — the
layout does not adapt to narrow screens.

### `static/vendor/`
`three.min.js` and `OrbitControls.js`, vendored so the page needs no network.

## 7. Training and inference scripts (`ml/`, optional)

### `ml/train_grpo.py`

The reinforcement-learning loop the studio runs as an external trainer. It samples
a group of candidate programs for a part, asks the studio to score them
(`POST /api/rewards/score`), and moves the policy towards the better members of
that group; it climbs a curriculum counted against each part's own history, hides
a dimension in the final stage, and stops on a plateau that has a floor beneath
it.

It speaks one protocol, on stdout:

```text
CADFORGE_STEP {"step": 3, "reward": 0.42, "mean": 0.44, "best": 0.68, "stage": 1}
CADFORGE_DONE {"converged": true, "reason": "the reward settled at 0.46"}
```

The second line is the trainer's verdict, and the studio believes it: a run that
stopped without converging is recorded as such rather than stamped `Done`.

With the stand-in policy (`CADFORGE_STUB=1`) the loop runs on a machine with no
graphics card and trains nothing - it exists to exercise the plumbing.

Full GRPO's PPO ratio and KL term are not implemented; the section on the
training loop in `CONCEPTS.md` says exactly what is.



### `ml/train_vision.py`
QLoRA fine-tune of a vision-language model. Default base model
`Qwen/Qwen2-VL-2B-Instruct`. 4-bit NF4 quantisation with double quant, LoRA
`r=16` / `alpha=32` / dropout 0.05 on `q_proj, k_proj, v_proj, o_proj`,
gradient checkpointing, bf16.

Reads: `CADFORGE_DATASET` (required), `CADFORGE_OUT` (default `checkpoints`),
`CADFORGE_DATA`, `CADFORGE_MODEL`, `CADFORGE_EPOCHS` (3), `CADFORGE_LR` (2e-4),
`CADFORGE_MAXLEN` (1024), `CADFORGE_BATCH` (1), `CADFORGE_GRAD_ACCUM` (16),
`CADFORGE_LORA_R` (16).

Writes `out_dir/adapter/` and `out_dir/training-report.json` (`base_model`,
`adapter`, `pairs`, `train`, `held_out`, `held_out_loss`, `epochs`,
`learning_rate`, `lora_r`, `max_len`).

The split comes from each pair's manifest `split` field, not a percentage: it
trains on `train` and scores the held-out pairs. With no usable pairs it stops
rather than training on nothing; with no held-out pairs it says the run has no
honest score. Full guide: `docs/TRAINING.md`.

### `ml/predict.py`
Answers picture → steps with a trained adapter (or the base model). Reads the raw
RGBA from `CADFORGE_IMAGE` with `CADFORGE_WIDTH`/`CADFORGE_HEIGHT`, optionally
loads `CADFORGE_ADAPTER` (default `data/runs/adapter`) for `CADFORGE_MODEL`, and
prints one JSON object as its last stdout line: `source`, `width`, `height`,
`steps`, `program`, `confidence`, `notes`. Exiting non-zero is intended — the
studio falls back to its own baseline. `confidence` is a hardcoded 0.5 placeholder.

### `ml/requirements.txt`
`transformers`, `peft`, `bitsandbytes`, `accelerate`, `datasets`, `pillow`,
`safetensors`, `sentencepiece`. PyTorch is deliberately absent: it needs its own
CUDA index.

## 8. Tools (`tools/`)

### `tools/make_step.py`
Generates AP214 STEP test parts with a manifold solid B-rep, stdlib only.
`make_step.py box > box.step` (plain box) and `make_step.py plate > plate.step`
(60×40×8 with four D6 holes). Its docstring is candid that the top and bottom
faces are plain rectangles and do not carry the hole loops — deliberate, to keep
the generator short.

### `tools/make_preview.py`
Freezes a running instance into one self-contained HTML file — same CSS, same
`app.js`, same data, with `fetch` and `EventSource` shimmed — so the interface can
be opened from disk. `--base` (default `http://127.0.0.1:8787`), `--out` (default
`preview/cadforge-preview.html`). It is a photograph, not the live app: buttons
that change things do not reach the backend.

### `tools/solidworks/ExportFeatureTree.bas`
A SolidWorks VBA macro that walks the FeatureManager and writes the real tree to
JSON — every feature, its dimensions, whether it is suppressed. Upload that JSON
and the studio shows the history *as recorded*. Optional bulk mode walks a folder
of parts.

### `tools/solidworks/CaptureViews.bas`
A SolidWorks VBA macro that screenshots the open model from four angles
(`-front`, `-top`, `-right`, `-iso`) beside the model file, named so each picture
pairs with its part. It verifies every file it claims to write is non-empty and
reports failure instead of leaving a broken capture behind. **Not yet executed
anywhere** — see `SUMMARY.md`.

## 9. Launchers and setup (`scripts/`, root)

| File | Purpose |
|---|---|
| `run.cmd` | Windows: builds once if `target\release\cadforge.exe` is missing, then runs it. Tells you how to install Rust if it is absent, and blames the C++ build tools if the build fails |
| `run.sh` | macOS/Linux: checks for cargo, opens the browser after 3 s, runs release mode |
| `scripts/setup-ml.cmd` | Windows: installs PyTorch with the CUDA index plus `ml/requirements.txt`, reports the GPU, verifies CUDA is visible |
| `scripts/train-local.cmd` | Windows: one-command fine-tune without the browser, with every default shown before it starts |

## 10. Tests

### `tests/ui_contract.rs`
Checks the interface contract without a browser, so a missing element id or an
API path with no route fails the build instead of failing in front of a user.

| Test | Enforces |
|---|---|
| `every_id_the_script_uses_exists_in_the_page` | no `$("#id")` without the id in the HTML |
| `every_api_path_the_script_calls_is_a_real_route` | no URL the server does not serve |
| `path_fragments_the_script_builds_are_not_typos` | built-up paths (`/mesh`, `/history`, …) exist |
| `the_page_is_completely_offline` | no `http(s)://`, CDN or font host in page/script/CSS |
| `the_page_loads_its_scripts_in_order` | three.js before OrbitControls before app.js |
| `every_element_id_is_unique` | a duplicate id cannot silently misbind |
| `the_upload_tiles_name_lanes_the_studio_accepts` | the three tiles use real lane names |
| `the_artifact_filters_name_real_lanes` | every filter chip is a real lane |

Unit tests live beside the code: `step.rs` (11, including the round-trip tests
`rebuilds_the_plate_from_its_own_history` and
`a_rebuild_that_loses_the_holes_scores_worse`), `score.rs` (3), `stl.rs` (2),
`freecad.rs` (2), `dataset.rs` (4), `pipeline.rs` (2), `vision.rs`,
`sldworks.rs`, `ole.rs`, `propset.rs`, `swhist.rs`.

## 11. Dependency map

```
  main.rs ──► api.rs ──┬─► pipeline.rs ──► model.rs ──┬─► step.rs ──► vec3.rs
      │                │        │                    │      │
      │                │        └─► dataset.rs ──────┘      ├─► ops.rs
      │                │                 │                 └─► solid.rs ──┐
      │                ├─► system.rs ──► pipeline.rs                     │
      │                │                                                │
      │                ├─► vision.rs ──► ops.rs, step.rs                 │
      │                │                                                │
      │                ├─► solid.rs ──► ops.rs, vec3.rs ◄───────────────┘
      │                ├─► score.rs ──► vec3.rs
      │                ├─► freecad.rs ──► ops.rs
      │                ├─► sldworks.rs ──► ole.rs, propset.rs
      │                ├─► swhist.rs ──► step.rs
      │                └─► stl.rs (stdlib only)
      │
      └─► ole.rs, propset.rs          (sample documents)

  On disk, written by the server:  db.json · assets/<id>.<ext> · rebuilds/ · runs/<id>/
```

Read it as: `api.rs` is the hub, `model.rs` is the schema everyone shares,
`ops.rs` is the contract geometry and learning both speak, and `vec3.rs`,
`stl.rs` and `ops.rs` have no dependency on the rest of the crate.

## 12. Documentation and data

| Path | What it is |
|---|---|
| `README.md` | front door: how to run it |
| `ARCHITECTURE.md` | the plan, the components, the roadmap, open questions |
| `docs/USAGE.md` | how to use it, in detail |
| `docs/CONCEPTS.md` | the vocabulary and the ideas |
| `docs/GLOSSARY.md` | this file |
| `docs/PROJECT.md` | what the project is and why |
| `docs/SUMMARY.md` | where it stands today, verified and unverified |
| `docs/TRAINING.md` | what fine-tuning on an 8 GB card really takes |
| `docs/release-workflow.yml` | the Windows release CI, kept here until a token with `workflow` scope can push it |
| `tests/fixtures/*.step` | generated parts the tests read |
| `data/` | runtime: `db.json`, `assets/`, `rebuilds/`, `runs/` |
